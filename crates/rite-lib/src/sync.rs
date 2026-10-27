//! Keeping a clone in step with its remote (ticket 102).
//!
//! Before it, nothing in rite fetched or pulled, and a rejected auto-push only
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
//! **Working while origin can't be reached** (ticket 197): changes are
//! committed locally as always, and wait. When origin is back and has moved
//! on too, [`catch_up`] puts this clone's commits on top of origin's
//! ([`replay`]) instead of stopping, as long as they only touch the board:
//! the ticket counter is settled to the higher number, and a ticket made
//! here whose number origin had already given out is renumbered (nobody
//! else has seen it). [`offline`] says how many changes are waiting.
//!
//! A repository with no remote, or `sync = off` / `RITE_SYNC=0`, is left
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

/// A catch-up that removes at least this many tickets says so (ticket 145):
/// one person tidying up removes a few, someone trashing a board removes
/// many.
pub const MASS_REMOVAL: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Listing: a stale view only warns, and a fetch under
    /// [`READ_FETCH_SECS`] old is reused.
    Read,
    /// Asking about one named ticket (`rite show 015`): like `Read`, but
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
    // A failed fetch isn't a recent one (ticket 197): try again.
    let fresh = purpose == Purpose::Read
        && git::fetched_within(repo, READ_FETCH_SECS)
        && !git::last_fetch_failed(repo);
    if !fresh && git::fetch(repo).is_err() {
        let waiting = git::ahead_behind(repo).map_or(0, |(ahead, _)| ahead);
        let detail = if waiting > 0 {
            format!("; {waiting} change{} waiting to push", plural(waiting))
        } else {
            String::new()
        };
        return Ok(vec![Event::Warn(format!(
            "couldn't reach origin — working from your local copy{detail}"
        ))]);
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
    // Commits this clone had from origin that origin no longer has: its
    // history was rewritten (ticket 145). Neither pushing them back nor
    // dropping them is rite's call, so nothing happens until a person
    // decides.
    if ahead > 0
        && let Some(lost) = rewritten(repo)
    {
        return stale(format!(
            "origin's history was rewritten (a force push): {lost} commit{} this clone had \
             from origin {} gone from it. This clone still has them, so don't reset it. \
             Check with your team; to put them back, push this clone with \
             `git push --force-with-lease origin HEAD` in {}",
            plural(lost),
            if lost == 1 { "is" } else { "are" },
            repo.display()
        ));
    }
    match (ahead, behind) {
        (0, 0) => Ok(Vec::new()),
        (0, behind) => {
            let removed = removed_tickets(board);
            match git::merge_ff_only(repo) {
                Ok(()) => {
                    let mut events = vec![Event::Info(format!(
                        "caught up: {behind} new change{} from origin",
                        plural(behind)
                    ))];
                    events.extend(removed);
                    Ok(events)
                }
                Err(err) => stale(format!(
                    "{} is {behind} commit{} behind origin and can't fast-forward ({err:#}) — \
                     commit or stash the local edits in it first",
                    repo.display(),
                    plural(behind)
                )),
            }
        }
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
        // Both moved on (ticket 197): ours on top of theirs when that's
        // safe, else stop for a person.
        (ahead, behind) => match replay(board, false)? {
            Replayed::Done(events) => Ok(events),
            Replayed::Stopped => stale(format!(
                "{} has {ahead} commit{} origin doesn't, and origin has {behind} commit{} it doesn't — \
                 run `git pull --rebase` there to combine them",
                repo.display(),
                plural(ahead),
                plural(behind)
            )),
        },
    }
}

/// How many changes are waiting to push because origin couldn't be reached
/// at rite's last try (ticket 197); `None` when it could, or the board
/// doesn't sync. Reads only: no fetch.
pub fn offline(board: &Board) -> Option<u32> {
    let repo = board.repo_root();
    (syncing(board) && git::last_fetch_failed(repo))
        .then(|| git::ahead_behind(repo).map_or(0, |(ahead, _)| ahead))
}

/// What [`replay`] did.
enum Replayed {
    /// Ours are on top of origin's (and pushed, unless a warning says not).
    Done(Vec<Event>),
    /// Left exactly as it was, for a person.
    Stopped,
}

/// Puts this clone's own commits on top of origin's (ticket 197): `git
/// rebase @{u}`, settling a conflict on the ticket counter alone by taking
/// the higher number, then renumbering any ticket made here whose number
/// origin had already given out, then pushing. Any other conflict, a clash
/// involving a ticket origin already has, or (unless `any_files`) a local
/// commit touching anything but the board leaves the clone as it was.
fn replay(board: &Board, any_files: bool) -> Result<Replayed> {
    let repo = board.repo_root();
    let Some(before) = git::head(repo) else {
        return Ok(Replayed::Stopped);
    };
    // A board inside a code repository: the other commits are someone's
    // code, not rite's to move.
    if !any_files && !only_board_files(board) {
        return Ok(Replayed::Stopped);
    }
    let ours = git::ahead_behind(repo).map_or(0, |(ahead, _)| ahead);
    let mut finished = git::rebase_upstream(repo)?;
    while !finished && git::rebase_in_progress(repo) {
        let conflicts = git::conflicted(repo);
        let counter = match conflicts.as_slice() {
            [only] if is_counter(only) => [2, 3]
                .into_iter()
                .filter_map(|stage| git::stage_text(repo, stage, only))
                .filter_map(|text| text.trim().parse::<u32>().ok())
                .max()
                .map(|higher| (only.clone(), higher)),
            _ => None,
        };
        let Some((path, higher)) = counter else {
            git::rebase_abort(repo)?;
            return Ok(Replayed::Stopped);
        };
        std::fs::write(repo.join(&path), format!("{higher}\n"))?;
        git::add(repo, &path)?;
        finished = git::rebase_continue(repo)?;
    }
    if !finished {
        // It never started (local edits in the way): nothing changed.
        return Ok(Replayed::Stopped);
    }
    let mut events = vec![Event::Info(format!(
        "your {ours} local change{} went on top of what origin had meanwhile",
        plural(ours)
    ))];
    if !board.duplicate_ids()?.is_empty() {
        let made_here = git::added_between(repo, "@{u}", "HEAD", &board.root);
        let here =
            |task: &crate::task::Task| made_here.iter().any(|path| task.path.ends_with(path));
        let clashes = crate::renumber::plan(board, None)?;
        let movable = clashes
            .iter()
            .all(|c| !here(&c.keep) && c.moves.iter().all(|(t, _)| here(t)));
        if !movable {
            git::reset_keep(repo, &before)?;
            return Ok(Replayed::Stopped);
        }
        let snapshot = git::dirty_paths(repo)?;
        let report = match crate::renumber::apply(board, &clashes) {
            Ok(report) => report,
            Err(_) => {
                // Back as it was. The rebase needed a clean tree, so any
                // edit now is the renumbering's own.
                git::reset_hard(repo, "HEAD")?;
                git::reset_keep(repo, &before)?;
                return Ok(Replayed::Stopped);
            }
        };
        let changed = git::changed_paths(&snapshot, &git::dirty_paths(repo)?);
        let names: Vec<String> = report
            .renamed
            .iter()
            .map(|(old, new)| format!("{} to {}", number(old), number(new)))
            .collect();
        git::commit_paths_locally(
            repo,
            &format!("renumber: {} (taken on origin)", names.join(", ")),
            &changed,
        )?;
        for (old, new) in &report.renamed {
            events.push(Event::Info(format!(
                "ticket {} is now {}: origin had already given out its number",
                number(old),
                number(new)
            )));
        }
        for (ticket, what) in &report.ambiguous {
            events.push(Event::Warn(format!("{ticket}: check {what}")));
        }
    }
    if let Err(err) = git::push(repo) {
        events.push(Event::Warn(format!("not pushed yet: {err:#}")));
    }
    Ok(Replayed::Done(events))
}

/// Whether every path this branch changed since its upstream is the board's
/// own: under its folder, or the counter and settings beside it.
fn only_board_files(board: &Board) -> bool {
    let repo = board.repo_root();
    let folder = board
        .root
        .strip_prefix(repo)
        .map(Path::to_path_buf)
        .unwrap_or_default();
    git::changed_since_upstream(repo)
        .iter()
        .all(|path| Path::new(path).starts_with(&folder) || is_counter(path) || path == ".settings")
}

/// The ticket counter's file (ticket 150), at the repository root or, on
/// an older board, in the board folder.
fn is_counter(path: &str) -> bool {
    Path::new(path)
        .file_name()
        .is_some_and(|name| name == ".next-id")
}

/// `263` from a ticket stem like `263-some-title`.
fn number(stem: &str) -> &str {
    stem.split_once('-').map_or(stem, |(id, _)| id)
}

/// How many commits this clone had from origin that origin's history no
/// longer contains, if any: some earlier position of the upstream (from
/// its reflog) is still in this clone but not in the upstream now. Only a
/// rewritten history (a force push) does that; ordinary pushes only add.
fn rewritten(repo: &Path) -> Option<u32> {
    let upstream = git::upstream_ref(repo)?;
    git::reflog(repo, &upstream, 50)
        .iter()
        .skip(1)
        .find(|old| git::is_ancestor(repo, old, "HEAD") && !git::is_ancestor(repo, old, "@{u}"))
        .map(|old| {
            git::count_commits(repo, &[old, "^@{u}"])
                .unwrap_or(1)
                .max(1)
        })
}

/// A warning when catching up would remove [`MASS_REMOVAL`] tickets or more
/// (ticket 145): how many, which, in which commits and by whom, and that
/// `rite recover` brings them back. Read before the fast-forward.
fn removed_tickets(board: &Board) -> Option<Event> {
    let repo = board.repo_root();
    let mut ids: Vec<String> = git::deleted_between(repo, "HEAD", "@{u}", &board.root)
        .iter()
        .filter_map(|path| crate::recover::ticket_file(path))
        .map(|(id, _)| format!("{id:03}"))
        .collect();
    if ids.len() < MASS_REMOVAL {
        return None;
    }
    let count = ids.len();
    ids.sort();
    let shown = if count > 8 {
        format!("{}, …", ids[..8].join(", "))
    } else {
        ids.join(", ")
    };
    let commits = git::deleting_commits(repo, "HEAD..@{u}", &board.root);
    let mut authors: Vec<&str> = Vec::new();
    for (_, who) in &commits {
        if !authors.contains(&who.as_str()) {
            authors.push(who);
        }
    }
    let shas: Vec<&str> = commits.iter().map(|(sha, _)| sha.as_str()).collect();
    Some(Event::Warn(format!(
        "origin removed {count} tickets ({shown}) in {} by {}. If that wasn't meant, \
         `rite recover` lists them and brings them back",
        shas.join(", "),
        authors.join(", ")
    )))
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

    // Replay (ticket 197): on top of theirs, the counter settled and a
    // ticket number taken meanwhile given the next free one.
    if let Replayed::Done(events) = replay(board, true)? {
        let mut note = vec![
            "someone else changed the board at the same time; your change was replayed on top \
             of theirs"
                .to_string(),
        ];
        note.extend(events.into_iter().skip(1).map(|event| match event {
            Event::Info(text) | Event::Warn(text) => text,
        }));
        return Ok(Some(note.join("; ")));
    }

    // Undo: keep a copy of what was written, drop the commit, catch up.
    let recovered = save_recovered(repo, changed);
    git::reset_keep(repo, pre)?;
    git::merge_ff_only(repo)?;
    Err(ConcurrentChange { recovered }.into())
}

/// Copies whichever of `changed` still exist into a fresh directory under
/// `$XDG_STATE_HOME/kanrite/recovered/`, so text typed into an undone
/// change isn't lost. `None` if there was nothing to keep or nowhere to put it.
fn save_recovered(repo: &Path, changed: &BTreeSet<PathBuf>) -> Option<PathBuf> {
    let state = crate::places::state()?;
    let stamp = time::OffsetDateTime::now_utc().unix_timestamp_nanos();
    let dir = state.join("recovered").join(stamp.to_string());
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

    /// Ticket 145: `a` pushes tickets 002..=007, `b` catches up.
    fn six_more(a: &Path, b: &Path) {
        for n in 2..=7 {
            commit_file(a, &format!("{n:03}-t.md"), false);
        }
        git(a, &["push", "-q"]);
        catch_up(&board(b), Purpose::Write).unwrap();
    }

    #[test]
    fn a_catch_up_removing_many_tickets_says_so() {
        let (_root, _origin, a, b) = two_clones();
        six_more(&a, &b);
        for n in 2..=7 {
            git(&a, &["rm", "-q", &format!("tasks/backlog/{n:03}-t.md")]);
        }
        git(&a, &["commit", "-q", "-m", "trash"]);
        git(&a, &["push", "-q"]);
        let events = catch_up(&board(&b), Purpose::Write).unwrap();
        assert!(
            matches!(&events[..], [Event::Info(_), Event::Warn(text)]
                if text.contains("origin removed 6 tickets (002, 003")
                    && text.contains("by a")
                    && text.contains("rite recover")),
            "{events:?}"
        );
        // It still caught up: the removal is recoverable, not refused.
        assert!(!b.join("tasks/backlog/002-t.md").exists());
    }

    #[test]
    fn removing_a_few_tickets_is_ordinary() {
        let (_root, _origin, a, b) = two_clones();
        six_more(&a, &b);
        git(&a, &["rm", "-q", "tasks/backlog/002-t.md"]);
        git(&a, &["commit", "-q", "-m", "tidy"]);
        git(&a, &["push", "-q"]);
        let events = catch_up(&board(&b), Purpose::Write).unwrap();
        assert!(matches!(&events[..], [Event::Info(_)]), "{events:?}");
    }

    #[test]
    fn a_rewritten_history_is_named_and_nothing_is_pushed_or_reset() {
        let (_root, origin, a, b) = two_clones();
        commit_file(&a, "002-two.md", true);
        commit_file(&a, "003-three.md", true);
        catch_up(&board(&b), Purpose::Write).unwrap();
        let before = git::head(&b);
        // Someone with push access throws the last two commits away.
        git(&a, &["reset", "-q", "--hard", "HEAD~2"]);
        git(&a, &["push", "-q", "--force"]);

        let err = catch_up(&board(&b), Purpose::Write)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("history was rewritten") && err.contains("2 commits"),
            "{err}"
        );
        let events = catch_up(&board(&b), Purpose::Read).unwrap();
        assert!(
            matches!(&events[..], [Event::Warn(text)] if text.contains("rewritten")),
            "{events:?}"
        );
        // b keeps its history, and origin isn't quietly given it back.
        assert_eq!(git::head(&b), before);
        let origin_head = Command::new("git")
            .arg("--git-dir")
            .arg(&origin)
            .args(["rev-list", "--count", "main"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&origin_head.stdout).trim(), "1");
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

    /// A real ticket, and the counter moved on past it, committed.
    fn new_ticket(repo: &Path, id: u32, title: &str, push: bool) {
        let slug = title.to_lowercase().replace(' ', "-");
        std::fs::write(
            repo.join(format!("tasks/backlog/{id:03}-{slug}.md")),
            format!("---\nid: \"{id:03}\"\ntitle: \"{title}\"\n---\n\nbody\n"),
        )
        .unwrap();
        std::fs::write(repo.join(".next-id"), format!("{}\n", id + 1)).unwrap();
        git(repo, &["add", "-A"]);
        git(repo, &["commit", "-q", "-m", title]);
        if push {
            git(repo, &["push", "-q"]);
        }
    }

    fn on_origin(origin: &Path) -> String {
        let out = Command::new("git")
            .args(["ls-tree", "-r", "--name-only", "main"])
            .current_dir(origin)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Ticket 197: two clones made tickets while apart (offline, say). The
    /// one catching up second goes on top, its clashing ticket renumbered
    /// (nobody else had seen it), the counter settled, and all pushed.
    #[test]
    fn a_diverged_clone_replays_its_tickets_and_renumbers_a_clash() {
        let (_root, origin, a, b) = two_clones();
        new_ticket(&a, 2, "From a", true);
        new_ticket(&b, 2, "From b", false);
        new_ticket(&b, 3, "Also b", false);
        let events = catch_up(&board(&b), Purpose::Write).unwrap();
        let said: Vec<String> = events
            .iter()
            .map(|e| match e {
                Event::Info(t) | Event::Warn(t) => t.clone(),
            })
            .collect();
        assert!(
            said.iter().any(|t| t.contains("ticket 002 is now 004")),
            "{said:?}"
        );
        assert_eq!(git::ahead_behind(&b), Some((0, 0)), "pushed: {said:?}");
        let tree = on_origin(&origin);
        for file in ["002-from-a.md", "003-also-b.md", "004-from-b.md"] {
            assert!(tree.contains(file), "{file} on origin:\n{tree}");
        }
        assert!(!tree.contains("002-from-b.md"), "{tree}");
        let moved = crate::eol::read(b.join("tasks/backlog/004-from-b.md")).unwrap();
        assert!(moved.contains("id: \"004\""), "{moved}");
        let counter = crate::eol::read(b.join(".next-id")).unwrap();
        assert_eq!(counter.trim(), "5");
    }

    /// A local commit outside the board (a board inside a code
    /// repository): someone's code, so nothing is moved.
    #[test]
    fn a_diverged_clone_with_other_files_still_stops() {
        let (_root, _origin, a, b) = two_clones();
        new_ticket(&a, 2, "From a", true);
        std::fs::write(b.join("main.rs"), "fn main() {}\n").unwrap();
        git(&b, &["add", "-A"]);
        git(&b, &["commit", "-q", "-m", "code"]);
        let err = catch_up(&board(&b), Purpose::Write)
            .unwrap_err()
            .to_string();
        assert!(err.contains("git pull --rebase"), "{err}");
    }

    /// Origin out of reach: the change waits, and rite says how many are
    /// waiting, until a fetch gets through.
    #[test]
    fn unreachable_origin_says_how_many_changes_are_waiting() {
        let (root, origin, _a, b) = two_clones();
        new_ticket(&b, 2, "Offline", false);
        let gone = root.path().join("gone.git");
        std::fs::rename(&origin, &gone).unwrap();
        let events = catch_up(&board(&b), Purpose::Read).unwrap();
        assert!(
            matches!(&events[..], [Event::Warn(t)] if t.contains("1 change waiting to push")),
            "{events:?}"
        );
        assert_eq!(offline(&board(&b)), Some(1));
        std::fs::rename(&gone, &origin).unwrap();
        catch_up(&board(&b), Purpose::Write).unwrap();
        assert_eq!(offline(&board(&b)), None);
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
