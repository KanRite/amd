//! Keeping a clone in step with its remote (ticket 102).
//!
//! Before it, nothing in amd fetched or pulled, and a rejected auto-push only
//! warned: two people on one board drifted apart silently — duplicate
//! ticket ids, a lease claimed twice, a teammate's ticket invisible until a
//! manual `git pull` (all proven by the multi-user suite, ticket 076).
//!
//! Two halves:
//!
//! - **[`catch_up`]**, before a command: fetch (a read skips it if the last
//!   fetch was under [`READ_FETCH_SECS`] ago), then fast-forward when the
//!   clone is only behind, push leftovers when it's only ahead, and refuse a
//!   change on a clone that has diverged — never write on stale state.
//! - **[`after_rejected_push`]**, when the push after a change is rejected
//!   because someone pushed in between: replay the change on top of theirs
//!   if that's clean and no ticket ids collide; otherwise save what was
//!   written, undo it, bring the board up to date, and fail with
//!   [`ConcurrentChange`] — never exit 0 with a change stranded locally.
//!
//! A repository with no remote, or `sync = off` / `AMD_SYNC=0`, is left
//! exactly as before. No network is a warning, not a failure. This is the
//! free tier's answer to concurrent ticket numbers; a cloud allocator
//! (`numbering.rs`) removes the id race, and this still guards edits.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::board::Board;
use crate::board_settings;
use crate::git;
use crate::transition::Event;

/// A read fetches at most this often; a change always fetches.
pub const READ_FETCH_SECS: u64 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Listing: a stale view only warns, and a fetch under
    /// [`READ_FETCH_SECS`] old is reused.
    Read,
    /// Asking about one named ticket (`amd show 015`): like `Read`, but
    /// always fetches — it may be one a teammate created a second ago.
    Lookup,
    /// Changing the board: refuse rather than write on stale state.
    Write,
}

/// Someone changed the board in the moment between catching up and pushing,
/// in a way the change couldn't be replayed over. Nothing was applied.
#[derive(Debug)]
pub struct ConcurrentChange {
    pub recovered: Option<PathBuf>,
}

impl std::fmt::Display for ConcurrentChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "someone changed the board at the same time, so your change was not applied — \
             the board is now up to date; run it again"
        )?;
        if let Some(dir) = &self.recovered {
            write!(f, " (what you wrote is saved in {})", dir.display())?;
        }
        Ok(())
    }
}

impl std::error::Error for ConcurrentChange {}

/// Whether this board syncs at all: on, and with somewhere to sync with.
fn syncing(board: &Board) -> bool {
    let repo = board.repo_root();
    board_settings::load(board).sync() && git::remote_url(repo).is_some() && git::has_upstream(repo)
}

/// Brings the clone in step with its remote before a command. Returns what
/// to tell the user; an `Err` only for a [`Purpose::Write`] that mustn't go
/// ahead (a clone that can't be brought up to date).
pub fn catch_up(board: &Board, purpose: Purpose) -> Result<Vec<Event>> {
    if !syncing(board) {
        return Ok(Vec::new());
    }
    let repo = board.repo_root();
    let fresh = purpose == Purpose::Read && git::fetched_within(repo, READ_FETCH_SECS);
    if !fresh && git::fetch(repo).is_err() {
        return Ok(vec![Event::Warn(
            "couldn't reach origin — working from your local copy".to_string(),
        )]);
    }
    let Some((ahead, behind)) = git::ahead_behind(repo) else {
        return Ok(Vec::new());
    };
    let stale = |what: String| -> Result<Vec<Event>> {
        match purpose {
            Purpose::Write => bail!("{what}"),
            Purpose::Read | Purpose::Lookup => Ok(vec![Event::Warn(format!(
                "{what} — showing your local copy"
            ))]),
        }
    };
    match (ahead, behind) {
        (0, 0) => Ok(Vec::new()),
        (0, behind) => match git::merge_ff_only(repo) {
            Ok(()) => Ok(vec![Event::Info(format!(
                "caught up: {behind} new change{} from origin",
                plural(behind)
            ))]),
            Err(err) => stale(format!(
                "{} is {behind} commit{} behind origin and can't fast-forward ({err:#}) — \
                 commit or stash the local edits in it first",
                repo.display(),
                plural(behind)
            )),
        },
        (ahead, 0) => match git::push(repo) {
            Ok(()) => Ok(vec![Event::Info(format!(
                "pushed {ahead} earlier change{} that hadn't reached origin",
                plural(ahead)
            ))]),
            Err(err) => Ok(vec![Event::Warn(format!(
                "{ahead} local change{} still not on origin: {err:#}",
                plural(ahead)
            ))]),
        },
        (ahead, behind) => stale(format!(
            "{} has {ahead} commit{} origin doesn't, and origin has {behind} commit{} it doesn't — \
             run `git pull --rebase` there to combine them",
            repo.display(),
            plural(ahead),
            plural(behind)
        )),
    }
}

/// After a change was committed (from `pre`, the commit before it) and its
/// push failed with `push_error`. Returns a note for the user, or
/// [`ConcurrentChange`] when the change had to be undone.
///
/// Only a *rejection* — the remote moved on — is handled here. Any other
/// failure (no network, no permission) keeps today's behaviour: the change
/// stays committed locally with a warning, and the next command's
/// [`catch_up`] pushes it.
pub fn after_rejected_push(
    board: &Board,
    pre: Option<&str>,
    changed: &BTreeSet<PathBuf>,
    push_error: &anyhow::Error,
) -> Result<Option<String>> {
    let not_pushed = || {
        Ok(Some(format!(
            "could not push automatically: {push_error:#}"
        )))
    };
    let repo = board.repo_root();
    if !syncing(board) || git::fetch(repo).is_err() {
        return not_pushed();
    }
    let Some((_, behind)) = git::ahead_behind(repo) else {
        return not_pushed();
    };
    let Some(pre) = pre else {
        return not_pushed();
    };
    if behind == 0 {
        return not_pushed();
    }

    // Replay: clean, and no ticket number taken twice, and the push lands.
    let rebased = git::rebase_upstream(repo)?;
    if rebased && board.duplicate_ids()?.is_empty() && git::push(repo).is_ok() {
        return Ok(Some(
            "someone else changed the board at the same time; your change was replayed on top \
             of theirs"
                .to_string(),
        ));
    }

    // Undo: keep a copy of what was written, drop the commit, catch up.
    let recovered = save_recovered(repo, changed);
    if rebased {
        git::reset_keep(repo, "@{u}")?;
    } else {
        git::rebase_abort(repo)?;
        git::reset_keep(repo, pre)?;
        git::merge_ff_only(repo)?;
    }
    Err(ConcurrentChange { recovered }.into())
}

/// Copies whichever of `changed` still exist into a fresh directory under
/// `$XDG_STATE_HOME/agile-md/recovered/`, so text typed into an undone
/// change isn't lost. `None` if there was nothing to keep or nowhere to put it.
fn save_recovered(repo: &Path, changed: &BTreeSet<PathBuf>) -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
        })?;
    let stamp = time::OffsetDateTime::now_utc().unix_timestamp_nanos();
    let dir = state.join("agile-md/recovered").join(stamp.to_string());
    let mut kept = false;
    for path in changed {
        let source = repo.join(path);
        let Some(name) = path.file_name() else {
            continue;
        };
        if source.is_file()
            && std::fs::create_dir_all(&dir).is_ok()
            && std::fs::copy(&source, dir.join(name)).is_ok()
        {
            kept = true;
        }
    }
    kept.then_some(dir)
}

fn plural(n: u32) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} in {}", dir.display());
    }

    fn identity(dir: &Path, who: &str) {
        git(dir, &["config", "user.email", &format!("{who}@t.co")]);
        git(dir, &["config", "user.name", who]);
        git(dir, &["config", "commit.gpgsign", "false"]);
    }

    /// A bare origin with one board commit, and two clones of it.
    fn two_clones() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let origin = root.path().join("origin.git");
        let (a, b) = (root.path().join("a"), root.path().join("b"));
        git(
            root.path(),
            &[
                "init",
                "-q",
                "--bare",
                "-b",
                "main",
                origin.to_str().unwrap(),
            ],
        );
        git(
            root.path(),
            &["init", "-q", "-b", "main", a.to_str().unwrap()],
        );
        identity(&a, "a");
        std::fs::create_dir_all(a.join("tasks/backlog")).unwrap();
        std::fs::write(a.join("tasks/backlog/001-one.md"), "one\n").unwrap();
        git(&a, &["add", "-A"]);
        git(&a, &["commit", "-q", "-m", "one"]);
        git(&a, &["remote", "add", "origin", origin.to_str().unwrap()]);
        git(&a, &["push", "-q", "-u", "origin", "main"]);
        git(
            root.path(),
            &["clone", "-q", origin.to_str().unwrap(), b.to_str().unwrap()],
        );
        identity(&b, "b");
        (root, origin, a, b)
    }

    fn commit_file(repo: &Path, name: &str, push: bool) {
        std::fs::write(repo.join("tasks/backlog").join(name), name).unwrap();
        git(repo, &["add", "-A"]);
        git(repo, &["commit", "-q", "-m", name]);
        if push {
            git(repo, &["push", "-q"]);
        }
    }

    fn board(repo: &Path) -> Board {
        Board::in_repo(repo)
    }

    #[test]
    fn a_clone_only_behind_is_fast_forwarded() {
        let (_root, _origin, a, b) = two_clones();
        commit_file(&a, "002-two.md", true);
        let events = catch_up(&board(&b), Purpose::Write).unwrap();
        assert!(b.join("tasks/backlog/002-two.md").exists());
        assert!(
            matches!(&events[..], [Event::Info(text)] if text.contains("caught up: 1 new change"))
        );
    }

    #[test]
    fn a_clone_only_ahead_pushes_its_leftovers() {
        let (_root, origin, _a, b) = two_clones();
        commit_file(&b, "002-two.md", false);
        catch_up(&board(&b), Purpose::Write).unwrap();
        let listed = Command::new("git")
            .arg("--git-dir")
            .arg(&origin)
            .args(["ls-tree", "-r", "--name-only", "main"])
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&listed.stdout).contains("002-two.md"));
    }

    #[test]
    fn a_diverged_clone_refuses_a_change_but_lets_a_read_through() {
        let (_root, _origin, a, b) = two_clones();
        commit_file(&a, "002-from-a.md", true);
        commit_file(&b, "002-from-b.md", false);
        let err = catch_up(&board(&b), Purpose::Write)
            .unwrap_err()
            .to_string();
        assert!(err.contains("git pull --rebase"), "{err}");
        let events = catch_up(&board(&b), Purpose::Read).unwrap();
        assert!(
            matches!(&events[..], [Event::Warn(text)] if text.contains("showing your local copy"))
        );
    }

    #[test]
    fn no_network_only_warns() {
        let (_root, origin, _a, b) = two_clones();
        std::fs::remove_dir_all(&origin).unwrap();
        let events = catch_up(&board(&b), Purpose::Write).unwrap();
        assert!(
            matches!(&events[..], [Event::Warn(text)] if text.contains("couldn't reach origin"))
        );
    }

    #[test]
    fn a_read_right_after_a_fetch_doesnt_fetch_again() {
        let (_root, _origin, a, b) = two_clones();
        catch_up(&board(&b), Purpose::Read).unwrap();
        commit_file(&a, "002-two.md", true);
        catch_up(&board(&b), Purpose::Read).unwrap();
        assert!(
            !b.join("tasks/backlog/002-two.md").exists(),
            "fetched within the throttle"
        );
        catch_up(&board(&b), Purpose::Lookup).unwrap();
        assert!(
            b.join("tasks/backlog/002-two.md").exists(),
            "a named lookup always fetches"
        );
        catch_up(&board(&b), Purpose::Write).unwrap();
        assert!(
            b.join("tasks/backlog/002-two.md").exists(),
            "a change always fetches"
        );
    }

    #[test]
    fn a_repository_without_a_remote_is_left_alone() {
        let root = tempfile::tempdir().unwrap();
        git(root.path(), &["init", "-q"]);
        assert!(
            catch_up(&board(root.path()), Purpose::Write)
                .unwrap()
                .is_empty()
        );
    }
}
