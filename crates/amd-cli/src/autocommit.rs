//! Commits and pushes exactly the paths a command touched, deliberately
//! outside `amd-lib`: `board.rs`'s `move_task` already draws this line for a
//! single move ("Deliberately does not commit... a library that commits is
//! a library that has decided when things are recorded"), and this extends
//! the same reasoning to every command. Orchestrating *when* something is
//! recorded is the CLI's job; the library only ever stages a rename.
//!
//! Built for a dedicated, single-writer-at-a-time tickets repository
//! (`rfc/0003-ticket-repository-topology-and-team-collaboration.md`), where
//! nothing else in the repository changes for a ticket move to ride
//! alongside — see ticket `012-have-amd-commit-board-and-document-changes`.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use agile_md::git;

/// Commits `touched` with a message built from `verb`, then pushes —
/// retrying once via a rebase if the remote moved since the last pull. A
/// second rejection is a real conflict, not just a stale ref, so it's
/// surfaced as an error rather than retried again: the commit stays local,
/// nothing is lost, and the caller resolves it the way any other rejected
/// push is resolved.
///
/// A repository with no `origin` configured at all commits locally and
/// stops there — not a failure, the same "isn't visible to a team yet"
/// reading `amd init` already gives a locally-found tickets repo with no
/// remote. Trying and failing to push would only turn every ordinary
/// command in a genuinely local-only board (or a test fixture) into an
/// error over something that was never going to be pushed anywhere.
pub fn commit_and_push(repo: &Path, verb: &str, touched: &[PathBuf]) -> Result<()> {
    let message = format!("{verb}: {}", subject(repo, touched));
    git::commit(repo, &message, touched)?;
    if git::remote_url(repo).is_none() {
        return Ok(());
    }
    match git::push_checked(repo)? {
        git::PushOutcome::Ok => Ok(()),
        git::PushOutcome::Rejected => {
            git::pull_rebase(repo)?;
            match git::push_checked(repo)? {
                git::PushOutcome::Ok => Ok(()),
                git::PushOutcome::Rejected => bail!(
                    "push still rejected after a rebase — the commit is local \
                     (see `git log -1`), resolve the conflict manually and push it yourself"
                ),
            }
        }
    }
}

/// A short, human-readable stand-in for "what this command touched" — the
/// touched files' own stems, since a ticket filename is already `id-slug`.
/// Deliberately not threaded through from each command handler: every
/// handler would need its own bespoke subject, and the files a command
/// actually wrote already say the same thing.
fn subject(repo: &Path, touched: &[PathBuf]) -> String {
    let mut stems: Vec<String> = touched
        .iter()
        .filter_map(|path| path.strip_prefix(repo).unwrap_or(path).file_stem())
        .map(|stem| stem.to_string_lossy().into_owned())
        .collect();
    stems.sort();
    stems.dedup();
    match stems.len() {
        0 => "(no files)".to_string(),
        1..=3 => stems.join(", "),
        n => format!("{}, +{} more", stems[..2].join(", "), n - 2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    fn seeded_clone(dir: &Path) -> (PathBuf, PathBuf) {
        let bare = dir.join("origin.git");
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "-q", "--bare"]);

        let working = dir.join("working");
        std::fs::create_dir_all(&working).unwrap();
        git(&working, &["init", "-q"]);
        git(&working, &["config", "user.email", "t@t.co"]);
        git(&working, &["config", "user.name", "t"]);
        git(
            &working,
            &["remote", "add", "origin", bare.to_str().unwrap()],
        );
        std::fs::write(working.join("README.md"), "hello\n").unwrap();
        git(&working, &["add", "README.md"]);
        git(&working, &["commit", "-q", "-m", "seed"]);
        git(&working, &["push", "-q", "-u", "origin", "HEAD"]);
        (bare, working)
    }

    #[test]
    fn subject_uses_the_bare_file_stem() {
        let repo = Path::new("/repo");
        let touched = vec![PathBuf::from("/repo/backlog/041-rename-init-file-name.md")];
        assert_eq!(subject(repo, &touched), "041-rename-init-file-name");
    }

    #[test]
    fn subject_joins_up_to_three_stems() {
        let repo = Path::new("/repo");
        let touched = vec![
            PathBuf::from("/repo/backlog/001-a.md"),
            PathBuf::from("/repo/backlog/002-b.md"),
        ];
        assert_eq!(subject(repo, &touched), "001-a, 002-b");
    }

    #[test]
    fn subject_truncates_beyond_three_with_a_count() {
        let repo = Path::new("/repo");
        let touched = vec![
            PathBuf::from("/repo/backlog/001-a.md"),
            PathBuf::from("/repo/backlog/002-b.md"),
            PathBuf::from("/repo/backlog/003-c.md"),
            PathBuf::from("/repo/backlog/004-d.md"),
        ];
        assert_eq!(subject(repo, &touched), "001-a, 002-b, +2 more");
    }

    #[test]
    fn commit_and_push_lands_a_clean_push() {
        let dir = tempfile::tempdir().unwrap();
        let (_bare, working) = seeded_clone(dir.path());
        std::fs::write(working.join("backlog.md"), "x\n").unwrap();
        commit_and_push(&working, "add", &[working.join("backlog.md")]).unwrap();

        let log = Command::new("git")
            .arg("-C")
            .arg(&working)
            .args(["log", "-1", "--format=%s"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&log.stdout).trim(), "add: backlog");
    }

    #[test]
    fn commit_and_push_recovers_once_from_a_rejected_push() {
        let dir = tempfile::tempdir().unwrap();
        let (bare, working) = seeded_clone(dir.path());

        let second = dir.path().join("second");
        let status = Command::new("git")
            .args(["clone", "-q", bare.to_str().unwrap()])
            .arg(&second)
            .status()
            .unwrap();
        assert!(status.success());
        git(&second, &["config", "user.email", "t@t.co"]);
        git(&second, &["config", "user.name", "t"]);
        std::fs::write(second.join("elsewhere.md"), "elsewhere\n").unwrap();
        git(&second, &["add", "elsewhere.md"]);
        git(&second, &["commit", "-q", "-m", "elsewhere"]);
        git(&second, &["push", "-q"]);

        std::fs::write(working.join("mine.md"), "mine\n").unwrap();
        commit_and_push(&working, "add", &[working.join("mine.md")]).unwrap();

        assert!(
            working.join("elsewhere.md").exists(),
            "rebased onto the other clone's commit"
        );
        assert!(working.join("mine.md").exists());
    }
}
