//! Thin wrappers over the `git` CLI.
//!
//! agile-md deliberately shells out rather than linking a git library: the
//! behaviour is then exactly what the user would get typing the command, and
//! the binary stays small and dependency-free on the git side.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

/// The work-tree root of the repository containing the current directory.
pub fn repo_root() -> Option<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let root = String::from_utf8(out.stdout).ok()?;
    let root = root.trim();
    if root.is_empty() {
        None
    } else {
        Some(PathBuf::from(root))
    }
}

/// Is `file` tracked by the repository at `repo`?
pub fn is_tracked(repo: &Path, file: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["ls-files", "--error-unmatch"])
        .arg(file)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// `git mv` — the rename is then part of the history, so `git log --follow`
/// reconstructs when a task started and finished.
pub fn mv(repo: &Path, from: &Path, to: &Path) -> Result<()> {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("mv")
        .arg(from)
        .arg(to)
        .status()
        .context("running git mv")?;
    if !status.success() {
        bail!("git mv {} {} failed", from.display(), to.display());
    }
    Ok(())
}

/// The checked-out branch, or `None` on a detached HEAD.
pub fn current_branch(repo: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["branch", "--show-current"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let branch = String::from_utf8(out.stdout).ok()?;
    let branch = branch.trim();
    if branch.is_empty() {
        None
    } else {
        Some(branch.to_string())
    }
}

pub fn branch_exists(repo: &Path, name: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet"])
        .arg(format!("refs/heads/{name}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Switch to `name`, creating it from the current HEAD when `create` is set.
/// Any staged changes (such as the `git mv` of the task itself) come along.
pub fn switch_branch(repo: &Path, name: &str, create: bool) -> Result<()> {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo).arg("switch");
    if create {
        command.arg("-c");
    }
    let status = command
        .arg(name)
        .status()
        .with_context(|| format!("running git switch {name}"))?;
    if !status.success() {
        bail!("git switch {name} failed");
    }
    Ok(())
}

/// Drop a file from the index while leaving it on disk. Used when a ticket is
/// archived: the archive is gitignored, so `git mv` would refuse, and the
/// history should record the ticket leaving the board.
pub fn untrack(repo: &Path, file: &Path) -> Result<()> {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rm", "--cached", "--quiet", "--"])
        .arg(file)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("running git rm --cached")?;
    if !status.success() {
        bail!("git rm --cached {} failed", file.display());
    }
    Ok(())
}

/// Commits on `name` that aren't reachable from the current `HEAD` — zero
/// means the branch adds nothing beyond wherever `HEAD` already is, which is
/// what "empty" means for cleaning up an abandoned ticket's branch.
pub fn commits_ahead(repo: &Path, name: &str) -> Option<u32> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-list", "--count"])
        .arg(format!("HEAD..{name}"))
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}

/// Whether `name` has a matching branch on `origin` — if so, someone else
/// might be looking at it, so cleanup should leave it alone.
pub fn branch_pushed(repo: &Path, name: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet"])
        .arg(format!("refs/remotes/origin/{name}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Delete a local branch. `-d`, not `-D`: callers are expected to have
/// already checked the branch is empty, and the safe form refusing an
/// unmerged branch is a second line of defence worth keeping.
pub fn delete_branch(repo: &Path, name: &str) -> Result<()> {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["branch", "-d", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("running git branch -d {name}"))?;
    if !status.success() {
        bail!("git branch -d {name} failed");
    }
    Ok(())
}

/// The `origin` remote's URL, if the repository has one.
pub fn remote_url(repo: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["remote", "get-url", "origin"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let url = String::from_utf8(out.stdout).ok()?;
    let url = url.trim();
    if url.is_empty() {
        None
    } else {
        Some(url.to_string())
    }
}

/// Points HEAD at `refs/heads/<name>` without requiring the branch to exist
/// yet — safe on a freshly cloned, still-empty repository, where HEAD is
/// "unborn" and there's nothing to rename. Used to make a fresh clone's
/// first commit land on `main` regardless of the local `init.defaultBranch`
/// git config, which a plain `git clone` of an empty repository otherwise
/// falls back to (still `master` on plenty of machines) — the remote's own
/// default branch name doesn't help here, since nothing has been pushed to
/// it yet for that setting to apply to.
pub fn set_head_branch(repo: &Path, name: &str) -> Result<()> {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["symbolic-ref", "HEAD"])
        .arg(format!("refs/heads/{name}"))
        .status()
        .with_context(|| format!("running git symbolic-ref HEAD refs/heads/{name}"))?;
    if !status.success() {
        bail!("git symbolic-ref HEAD refs/heads/{name} failed");
    }
    Ok(())
}

/// Stages everything and commits — used once, to give a freshly created
/// tickets repository its first commit (an empty GitHub repo has no branch
/// to push to until one exists).
pub fn commit_all(repo: &Path, message: &str) -> Result<()> {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["add", "-A"])
        .status()
        .context("running git add -A")?;
    if !status.success() {
        bail!("git add -A failed");
    }
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["commit", "-m", message])
        .status()
        .context("running git commit")?;
    if !status.success() {
        bail!("git commit failed");
    }
    Ok(())
}

/// `git push -u origin HEAD` — sets the upstream, which a freshly cloned,
/// still-commit-less GitHub repository doesn't have yet.
pub fn push(repo: &Path) -> Result<()> {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["push", "-u", "origin", "HEAD"])
        .status()
        .context("running git push")?;
    if !status.success() {
        bail!("git push failed");
    }
    Ok(())
}

/// Stages exactly `paths` (already absolute, or relative to `repo`) and
/// commits — unlike `commit_all`, this never sweeps in anything else that
/// happens to be staged or dirty. The auto-commit feature's whole safety
/// property rests on this: a command's own writes are committed, and nothing
/// else the working tree happens to be carrying.
///
/// The pathspec goes on `git commit` itself, not only on a preceding `git
/// add` — a plain `git commit -m` with no pathspec commits the *entire*
/// index, including anything staged before this ran for an unrelated
/// reason. `git commit -- <paths>` commits only changes to those paths
/// regardless of what else is staged, which is the actual guarantee this
/// function exists to make.
pub fn commit(repo: &Path, message: &str, paths: &[PathBuf]) -> Result<()> {
    if paths.is_empty() {
        bail!("commit called with no paths");
    }
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("add")
        .arg("--")
        .args(paths)
        .status()
        .context("running git add")?;
    if !status.success() {
        bail!("git add failed");
    }
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["commit", "-m", message, "--"])
        .args(paths)
        .status()
        .context("running git commit")?;
    if !status.success() {
        bail!("git commit failed");
    }
    Ok(())
}

/// What `push_checked` found.
pub enum PushOutcome {
    /// Landed on the remote.
    Ok,
    /// Rejected because the remote moved since the last pull — the one
    /// failure the auto-commit feature retries, once, via `pull_rebase`.
    Rejected,
}

/// `git push -u origin HEAD`, like `push`, but distinguishes a
/// non-fast-forward rejection (the remote moved) from every other failure
/// instead of bailing on both alike — the auto-commit feature needs to tell
/// "retry with a rebase" apart from "something is actually wrong".
pub fn push_checked(repo: &Path) -> Result<PushOutcome> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["push", "-u", "origin", "HEAD"])
        .output()
        .context("running git push")?;
    if out.status.success() {
        return Ok(PushOutcome::Ok);
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("[rejected]")
        || stderr.contains("non-fast-forward")
        || stderr.contains("fetch first")
    {
        return Ok(PushOutcome::Rejected);
    }
    bail!("git push failed: {}", stderr.trim());
}

/// `git pull --rebase` against the already-tracked upstream (set by the
/// first `push`/`push_checked` call). Used exactly once, as the auto-commit
/// feature's single retry after a rejected push — never looped. If this
/// also fails, that's a real conflict, not just "the remote moved", and the
/// commit stays local for the caller to report rather than being retried
/// again automatically.
pub fn pull_rebase(repo: &Path) -> Result<()> {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["pull", "--rebase"])
        .status()
        .context("running git pull --rebase")?;
    if !status.success() {
        bail!("git pull --rebase failed — resolve manually, the commit is still local");
    }
    Ok(())
}

/// The set of paths `git status --porcelain` reports as changed (staged or
/// not) under `repo`. The auto-commit feature snapshots this before a
/// command runs and diffs it against the snapshot after, so only the paths a
/// single command actually touched are ever committed — never whatever else
/// the working tree happens to be carrying.
///
/// Assumes plain, unquoted paths (no embedded spaces or control characters):
/// true for every path this tool itself ever writes, since `task::slugify`
/// already guarantees an ASCII, hyphen-separated filename. A path porcelain
/// would need to quote is simply not one `amd` produces.
pub fn changed_paths(repo: &Path) -> Result<HashSet<PathBuf>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["status", "--porcelain"])
        .output()
        .context("running git status --porcelain")?;
    if !out.status.success() {
        bail!("git status --porcelain failed");
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut paths = HashSet::new();
    for line in text.lines() {
        if line.len() < 4 {
            continue;
        }
        // Porcelain v1: two status columns, a space, then the path — or, for
        // a detected rename, "old -> new".
        let rest = &line[3..];
        let path = rest.split_once(" -> ").map_or(rest, |(_old, new)| new);
        paths.insert(repo.join(path));
    }
    Ok(paths)
}

/// A `git config` value (used to fill `author` in task templates, and to
/// resolve the current identity for a ticket lease). `-C repo` like every
/// other function here, rather than relying on the process's current
/// directory — the caller may not be standing in the repo in question (a
/// desktop board open across several registered repositories, a test), and
/// git itself resolves the repository boundary from any path inside it, so
/// this doesn't need `repo` to be the root.
pub fn config(repo: &Path, key: &str) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["config", "--get", key])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let value = String::from_utf8(out.stdout).ok()?;
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    fn init_repo(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        git(dir, &["init", "-q"]);
        git(dir, &["config", "user.email", "t@t.co"]);
        git(dir, &["config", "user.name", "t"]);
    }

    #[test]
    fn remote_url_reads_back_what_was_configured() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_repo(&repo);
        assert_eq!(remote_url(&repo), None);
        git(&repo, &["remote", "add", "origin", "/tmp/wherever"]);
        assert_eq!(remote_url(&repo).as_deref(), Some("/tmp/wherever"));
    }

    #[test]
    fn commit_all_and_push_round_trip_against_a_local_bare_repo() {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("origin.git");
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "-q", "--bare"]);

        let working = dir.path().join("working");
        init_repo(&working);
        git(
            &working,
            &["remote", "add", "origin", bare.to_str().unwrap()],
        );

        std::fs::write(working.join("README.md"), "hello\n").unwrap();
        commit_all(&working, "seed").unwrap();
        push(&working).unwrap();

        // A plain clone from the same bare repo sees the pushed commit.
        let second = dir.path().join("second");
        let status = Command::new("git")
            .args(["clone", "-q", bare.to_str().unwrap()])
            .arg(&second)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(second.join("README.md").exists());
    }

    #[test]
    fn set_head_branch_forces_the_first_commit_onto_that_branch() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        // Simulate a machine whose local git defaults new repos to
        // something other than `main` — an unborn HEAD, no commits yet.
        git(&repo, &["init", "-q", "--initial-branch=trunk"]);
        git(&repo, &["config", "user.email", "t@t.co"]);
        git(&repo, &["config", "user.name", "t"]);

        set_head_branch(&repo, "main").unwrap();
        std::fs::write(repo.join("README.md"), "hello\n").unwrap();
        commit_all(&repo, "seed").unwrap();

        assert_eq!(current_branch(&repo).as_deref(), Some("main"));
    }

    /// A bare repo plus one clone with an initial commit already pushed —
    /// the fixture every `commit`/`push_checked`/`pull_rebase`/
    /// `changed_paths` test below builds on.
    fn seeded_clone(dir: &Path) -> (PathBuf, PathBuf) {
        let bare = dir.join("origin.git");
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "-q", "--bare"]);

        let working = dir.join("working");
        init_repo(&working);
        git(
            &working,
            &["remote", "add", "origin", bare.to_str().unwrap()],
        );
        std::fs::write(working.join("README.md"), "hello\n").unwrap();
        commit_all(&working, "seed").unwrap();
        push(&working).unwrap();
        (bare, working)
    }

    #[test]
    fn commit_stages_only_the_given_paths_never_other_dirty_or_staged_files() {
        let dir = tempfile::tempdir().unwrap();
        let (_bare, working) = seeded_clone(dir.path());

        // A file the caller wants committed...
        std::fs::write(working.join("wanted.md"), "wanted\n").unwrap();
        // ...and unrelated dirty/staged state that must be left alone,
        // exactly ticket 012's "must never sweep in unrelated work" case.
        std::fs::write(working.join("unrelated.md"), "unrelated\n").unwrap();
        git(&working, &["add", "unrelated.md"]);

        commit(
            &working,
            "add: 001 \"Wanted\"",
            &[working.join("wanted.md")],
        )
        .unwrap();

        let log = Command::new("git")
            .arg("-C")
            .arg(&working)
            .args(["show", "--stat", "--format=", "HEAD"])
            .output()
            .unwrap();
        let stat = String::from_utf8_lossy(&log.stdout);
        assert!(stat.contains("wanted.md"), "{stat}");
        assert!(!stat.contains("unrelated.md"), "{stat}");

        // unrelated.md is still staged, untouched by the commit.
        let status = Command::new("git")
            .arg("-C")
            .arg(&working)
            .args(["status", "--porcelain"])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&status.stdout).contains("A  unrelated.md"),
            "{}",
            String::from_utf8_lossy(&status.stdout)
        );
    }

    #[test]
    fn commit_refuses_an_empty_path_list() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_repo(&repo);
        assert!(commit(&repo, "nothing", &[]).is_err());
    }

    #[test]
    fn push_checked_reports_ok_on_a_clean_push() {
        let dir = tempfile::tempdir().unwrap();
        let (_bare, working) = seeded_clone(dir.path());
        std::fs::write(working.join("two.md"), "two\n").unwrap();
        commit(&working, "add: 002", &[working.join("two.md")]).unwrap();
        assert!(matches!(push_checked(&working).unwrap(), PushOutcome::Ok));
    }

    #[test]
    fn push_checked_reports_rejected_when_the_remote_has_moved() {
        let dir = tempfile::tempdir().unwrap();
        let (bare, working) = seeded_clone(dir.path());

        // A second clone pushes first, moving the remote out from under
        // `working` without it knowing.
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
        commit_all(&second, "elsewhere").unwrap();
        push(&second).unwrap();

        std::fs::write(working.join("mine.md"), "mine\n").unwrap();
        commit(&working, "add: 003", &[working.join("mine.md")]).unwrap();
        assert!(matches!(
            push_checked(&working).unwrap(),
            PushOutcome::Rejected
        ));
    }

    #[test]
    fn pull_rebase_recovers_a_rejected_push_and_a_retry_then_succeeds() {
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
        commit_all(&second, "elsewhere").unwrap();
        push(&second).unwrap();

        std::fs::write(working.join("mine.md"), "mine\n").unwrap();
        commit(&working, "add: 003", &[working.join("mine.md")]).unwrap();
        assert!(matches!(
            push_checked(&working).unwrap(),
            PushOutcome::Rejected
        ));

        pull_rebase(&working).unwrap();
        assert!(matches!(push_checked(&working).unwrap(), PushOutcome::Ok));
        // Both commits made it: the other clone's and this one's, rebased
        // on top.
        assert!(working.join("elsewhere.md").exists());
        assert!(working.join("mine.md").exists());
    }

    #[test]
    fn changed_paths_reports_new_modified_and_renamed_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let (_bare, working) = seeded_clone(dir.path());

        std::fs::write(working.join("new.md"), "new\n").unwrap();
        std::fs::write(working.join("README.md"), "changed\n").unwrap();

        let found = changed_paths(&working).unwrap();
        assert!(found.contains(&working.join("new.md")));
        assert!(found.contains(&working.join("README.md")));
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn changed_paths_is_empty_on_a_clean_tree() {
        let dir = tempfile::tempdir().unwrap();
        let (_bare, working) = seeded_clone(dir.path());
        assert!(changed_paths(&working).unwrap().is_empty());
    }
}
