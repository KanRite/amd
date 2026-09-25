//! Thin wrappers over the `git` CLI.
//!
//! agile-md deliberately shells out rather than linking a git library: the
//! behaviour is then exactly what the user would get typing the command, and
//! the binary stays small and dependency-free on the git side.

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
}
