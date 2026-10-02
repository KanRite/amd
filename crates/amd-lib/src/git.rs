//! Thin wrappers over the `git` CLI.
//!
//! agile-md deliberately shells out rather than linking a git library: the
//! behaviour is then exactly what the user would get typing the command, and
//! the binary stays small and dependency-free on the git side.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether git's own output is kept off the terminal. The command line wants
/// to show it (`Switched to a new branch ...` is part of what `amd todo`
/// says); a full-screen front end can't have a child process writing over its
/// display, so it turns this on and gets the failure text back in the error
/// instead.
static QUIET: AtomicBool = AtomicBool::new(false);

/// Keep git's stdout and stderr off the terminal from now on, for the whole
/// process. Failures still carry git's own message.
pub fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

/// Run a git command whose output a person might want to see. Returns whether
/// it succeeded and, when output is being kept quiet, git's stderr (so the
/// reason a push failed isn't lost along with the noise).
fn run(command: &mut Command) -> std::io::Result<(bool, String)> {
    if QUIET.load(Ordering::Relaxed) {
        let out = command.stdin(Stdio::null()).output()?;
        let detail = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Ok((out.status.success(), detail))
    } else {
        Ok((command.status()?.success(), String::new()))
    }
}

/// `"git X failed"`, plus git's own reason when there is one.
fn failed(what: &str, detail: &str) -> String {
    match detail.lines().last() {
        Some(reason) if !reason.is_empty() => format!("{what} failed: {reason}"),
        _ => format!("{what} failed"),
    }
}
use std::time::SystemTime;

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
    let (ok, detail) = run(Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("mv")
        .arg(from)
        .arg(to))
    .context("running git mv")?;
    if !ok {
        bail!(failed(
            &format!("git mv {} {}", from.display(), to.display()),
            &detail
        ));
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
    let (ok, detail) =
        run(command.arg(name)).with_context(|| format!("running git switch {name}"))?;
    if !ok {
        bail!(failed(&format!("git switch {name}"), &detail));
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
    let (ok, detail) = run(Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["push", "-u", "origin", "HEAD"]))
    .context("running git push")?;
    if !ok {
        bail!(failed("git push", &detail));
    }
    Ok(())
}

/// Every path git currently considers changed, mapped to a cheap signature
/// of its current on-disk state (`None` if nothing is there — a rename's
/// old side, or a path that was deleted). `git status --porcelain=v1
/// --untracked-files=all` finds the paths; a rename line ("R  old -> new")
/// contributes both. The `=all` matters: git's default collapses a wholly-
/// untracked directory into a single line for the directory itself (real
/// case, hit directly: a board created with `amd init --local-only` has
/// never been committed, so `tasks/` is one untracked directory — without
/// `=all`, a brand new ticket inside it never appears as its own path).
///
/// The signature matters for the same underlying reason: an untracked
/// file's status is always "??" regardless of content, so a path already
/// dirty in one snapshot that changes *again* by the next snapshot is
/// otherwise indistinguishable from one that never changed at all — hit
/// directly too, by `.settings` on that same never-committed board:
/// `record_id` bumps it every time `amd new` runs, but it was already
/// untracked before the second run, so presence alone can't tell the two
/// states apart. Modified-time is enough; this isn't trying to be a
/// content hash.
///
/// Used to snapshot a repo before and after a mutation
/// (`changed_paths` computes the diff; `commit_paths` is the caller),
/// rather than a hand-maintained paths list that would miss an incidental
/// write like this one.
pub fn dirty_paths(repo: &Path) -> Result<BTreeMap<PathBuf, Option<SystemTime>>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["status", "--porcelain=v1", "--untracked-files=all"])
        .output()
        .context("running git status")?;
    if !out.status.success() {
        bail!("git status failed");
    }
    let text = String::from_utf8(out.stdout).context("git status produced non-utf8 output")?;
    let mut paths = BTreeSet::new();
    for line in text.lines() {
        // Two status characters, a space, then the path (or "old -> new"
        // for a rename).
        let Some(rest) = line.get(3..) else { continue };
        match rest.split_once(" -> ") {
            Some((old, new)) => {
                paths.insert(PathBuf::from(old));
                paths.insert(PathBuf::from(new));
            }
            None => {
                paths.insert(PathBuf::from(rest));
            }
        }
    }
    Ok(paths
        .into_iter()
        .map(|path| {
            let mtime = std::fs::metadata(repo.join(&path))
                .and_then(|meta| meta.modified())
                .ok();
            (path, mtime)
        })
        .collect())
}

/// Which paths actually changed between two `dirty_paths` snapshots, and
/// are still something `after`'s own `git status` currently knows about —
/// not just newly dirty, but also a path already dirty in `before` that
/// changed again by `after` (see `dirty_paths`'s own doc for why presence
/// alone isn't enough for an untracked file).
///
/// The `after.contains_key` filter matters on its own, confirmed live
/// against a real repo rather than reasoned out in advance: a ticket that
/// already had an *uncommitted* rename sitting from before this operation
/// (`backlog` staged to `todo`, say) and is then moved again before either
/// side is ever committed collapses, in git's own index, straight from
/// `backlog` to the final destination — `todo` never existed in any commit,
/// so it simply stops being mentioned anywhere in `after`'s status at all.
/// Without this filter, `todo` still shows up as "changed" (dirty in
/// `before`, absent from `after`) and gets handed to `git commit` as a
/// pathspec it has no memory of, which fails outright ("did not match any
/// files known to git") — a real failure, hit directly, not a hypothetical
/// this filter guards against speculatively.
pub fn changed_paths(
    before: &BTreeMap<PathBuf, Option<SystemTime>>,
    after: &BTreeMap<PathBuf, Option<SystemTime>>,
) -> BTreeSet<PathBuf> {
    let mut all: BTreeSet<&PathBuf> = before.keys().collect();
    all.extend(after.keys());
    all.into_iter()
        .filter(|path| before.get(*path) != after.get(*path) && after.contains_key(*path))
        .cloned()
        .collect()
}

/// Commits exactly `paths` (relative to `repo`'s root, as `dirty_paths`
/// returns them) with `message`, then pushes. Never touches anything else
/// staged, even if something else already was — the pathspec is on both
/// `add` and `commit` itself, since a pathspec on `add` alone still lets a
/// bare `git commit -m` sweep in the whole index. A no-op on an empty set,
/// so a caller doesn't need its own check first.
pub fn commit_paths(repo: &Path, message: &str, paths: &BTreeSet<PathBuf>) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    // Only `add` paths that still exist on disk. A path that's gone — the
    // old side of a `git mv`-based rename, or a `git rm --cached`'d
    // archived ticket — is already fully staged by whatever produced that
    // rename or removal; re-adding it errors ("did not match any files"),
    // since there's nothing left for `add` to reconcile. The commit's own
    // pathspec below still needs every path, gone or not, to capture the
    // whole change — same reason a rename needs both sides there, not just
    // on `add`.
    let existing: Vec<&PathBuf> = paths
        .iter()
        .filter(|path| repo.join(path).exists())
        .collect();
    if !existing.is_empty() {
        let (ok, detail) = run(Command::new("git")
            .arg("-C")
            .arg(repo)
            .arg("add")
            .arg("--")
            .args(existing))
        .context("running git add")?;
        if !ok {
            bail!(failed("git add", &detail));
        }
    }
    let (ok, detail) = run(Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["commit", "-m", message])
        .arg("--")
        .args(paths))
    .context("running git commit")?;
    if !ok {
        bail!(failed("git commit", &detail));
    }
    push(repo)
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

    #[test]
    fn a_failure_carries_gits_own_reason_when_there_is_one() {
        assert_eq!(failed("git push", ""), "git push failed");
        assert_eq!(
            failed("git push", "hint: x\nfatal: no remote"),
            "git push failed: fatal: no remote"
        );
    }

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
    fn dirty_paths_reports_untracked_modified_and_renamed_files() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_repo(&repo);
        std::fs::write(repo.join("tracked.md"), "one\n").unwrap();
        std::fs::write(repo.join("renamed.md"), "two\n").unwrap();
        commit_all(&repo, "seed").unwrap();

        std::fs::write(repo.join("tracked.md"), "one, edited\n").unwrap();
        std::fs::write(repo.join("untracked.md"), "new\n").unwrap();
        git(&repo, &["mv", "renamed.md", "moved.md"]);

        let dirty = dirty_paths(&repo).unwrap();
        assert!(dirty.contains_key(&PathBuf::from("tracked.md")));
        assert!(dirty.contains_key(&PathBuf::from("untracked.md")));
        assert!(
            dirty.contains_key(&PathBuf::from("renamed.md")),
            "old side of the rename"
        );
        assert!(
            dirty.contains_key(&PathBuf::from("moved.md")),
            "new side of the rename"
        );
    }

    #[test]
    fn dirty_paths_lists_files_inside_a_wholly_untracked_directory() {
        // The real case that broke this: `amd init --local-only` never
        // commits, so `tasks/` starts life entirely untracked. Without
        // `--untracked-files=all`, git collapses that whole directory into
        // one line and a file created inside it later never looks "new".
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_repo(&repo);
        std::fs::create_dir_all(repo.join("tasks/backlog")).unwrap();
        std::fs::write(repo.join("tasks/backlog/001-first.md"), "one\n").unwrap();

        let dirty = dirty_paths(&repo).unwrap();
        assert!(
            dirty.contains_key(&PathBuf::from("tasks/backlog/001-first.md")),
            "{dirty:?}"
        );
    }

    #[test]
    fn changed_paths_notices_a_file_already_dirty_that_changed_again() {
        // The other real case that broke this: `.settings` is untracked
        // (never committed) from `amd init --local-only`, so its git status
        // is "??" both before and after a second `amd new` bumps its
        // counter — presence in the dirty set alone can't tell those two
        // moments apart, only its on-disk signature can.
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_repo(&repo);
        std::fs::write(repo.join(".settings"), "next-id = 1\n").unwrap();
        let before = dirty_paths(&repo).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(repo.join(".settings"), "next-id = 2\n").unwrap();
        let after = dirty_paths(&repo).unwrap();

        let changed = changed_paths(&before, &after);
        assert!(changed.contains(&PathBuf::from(".settings")), "{changed:?}");
    }

    #[test]
    fn changed_paths_excludes_a_file_dirty_both_times_but_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_repo(&repo);
        std::fs::write(repo.join("untouched.md"), "same\n").unwrap();
        let before = dirty_paths(&repo).unwrap();
        // Something else becomes dirty in between, but untouched.md itself
        // never changes.
        std::fs::write(repo.join("other.md"), "new\n").unwrap();
        let after = dirty_paths(&repo).unwrap();

        let changed = changed_paths(&before, &after);
        assert!(changed.contains(&PathBuf::from("other.md")));
        assert!(
            !changed.contains(&PathBuf::from("untouched.md")),
            "{changed:?}"
        );
    }

    #[test]
    fn changed_paths_drops_an_intermediate_hop_git_has_already_forgotten() {
        // The real bug, hit directly against a live repo: a ticket already
        // mid-rename (staged, never committed) before this operation even
        // starts, then moved again before either side lands in a commit.
        // Git's own index collapses the whole chain straight from the
        // original committed path to the final one — the middle hop never
        // existed in any commit, so `after`'s status stops mentioning it
        // entirely, even though it was genuinely dirty in `before`.
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("origin.git");
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "-q", "--bare"]);

        let repo = dir.path().join("repo");
        init_repo(&repo);
        git(&repo, &["remote", "add", "origin", bare.to_str().unwrap()]);
        std::fs::write(repo.join("start.md"), "content\n").unwrap();
        commit_all(&repo, "seed").unwrap();
        push(&repo).unwrap();

        // First hop, staged but never committed.
        git(&repo, &["mv", "start.md", "middle.md"]);
        let before = dirty_paths(&repo).unwrap();
        assert!(before.contains_key(&PathBuf::from("middle.md")));

        // Second hop, on top of the still-uncommitted first one — git
        // collapses this to start.md -> end.md directly.
        git(&repo, &["mv", "middle.md", "end.md"]);
        let after = dirty_paths(&repo).unwrap();
        assert!(
            !after.contains_key(&PathBuf::from("middle.md")),
            "git should have forgotten the intermediate hop: {after:?}"
        );

        let changed = changed_paths(&before, &after);
        assert!(
            !changed.contains(&PathBuf::from("middle.md")),
            "a path git has no memory of must never reach commit_paths: {changed:?}"
        );
        assert!(changed.contains(&PathBuf::from("end.md")));

        // The real proof: committing this must not error the way it did
        // live ("did not match any files known to git").
        commit_paths(&repo, "test: chained rename", &changed).unwrap();
    }

    #[test]
    fn dirty_paths_is_empty_on_a_clean_repo() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_repo(&repo);
        std::fs::write(repo.join("README.md"), "hello\n").unwrap();
        commit_all(&repo, "seed").unwrap();
        assert!(dirty_paths(&repo).unwrap().is_empty());
    }

    #[test]
    fn commit_paths_commits_only_the_given_paths() {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("origin.git");
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "-q", "--bare"]);

        let repo = dir.path().join("repo");
        init_repo(&repo);
        git(&repo, &["remote", "add", "origin", bare.to_str().unwrap()]);
        std::fs::write(repo.join("README.md"), "hello\n").unwrap();
        commit_all(&repo, "seed").unwrap();
        push(&repo).unwrap();

        std::fs::write(repo.join("wanted.md"), "wanted\n").unwrap();
        std::fs::write(
            repo.join("unrelated.md"),
            "already staged by someone else\n",
        )
        .unwrap();
        git(&repo, &["add", "unrelated.md"]);

        let mut paths = BTreeSet::new();
        paths.insert(PathBuf::from("wanted.md"));
        commit_paths(&repo, "test: commit", &paths).unwrap();

        let show = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["show", "--name-only", "--pretty=format:"])
            .output()
            .unwrap();
        let committed = String::from_utf8(show.stdout).unwrap();
        assert!(committed.contains("wanted.md"));
        assert!(!committed.contains("unrelated.md"), "{committed}");

        // The unrelated file is still staged, untouched, not swept in.
        let status = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["status", "--porcelain=v1"])
            .output()
            .unwrap();
        let status = String::from_utf8(status.stdout).unwrap();
        assert!(status.contains("unrelated.md"));
    }

    #[test]
    fn commit_paths_handles_a_rename_whose_old_side_git_mv_already_staged() {
        // The real bug: `git mv` fully stages both sides of a rename in the
        // index by itself. Re-`add`ing the now-gone old path afterward
        // fails ("did not match any files") — commit_paths must skip `add`
        // for a path that no longer exists, while still passing both paths
        // to `commit` itself (the commit needs both to record the rename;
        // omitting the old side is the same gotcha a plain two-commit
        // rename has).
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("origin.git");
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "-q", "--bare"]);

        let repo = dir.path().join("repo");
        init_repo(&repo);
        git(&repo, &["remote", "add", "origin", bare.to_str().unwrap()]);
        std::fs::write(repo.join("old.md"), "content\n").unwrap();
        commit_all(&repo, "seed").unwrap();
        push(&repo).unwrap();

        git(&repo, &["mv", "old.md", "new.md"]);

        let mut paths = BTreeSet::new();
        paths.insert(PathBuf::from("old.md"));
        paths.insert(PathBuf::from("new.md"));
        commit_paths(&repo, "test: rename", &paths).unwrap();

        let show = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["show", "--name-status", "--pretty=format:"])
            .output()
            .unwrap();
        let committed = String::from_utf8(show.stdout).unwrap();
        assert!(
            committed.contains("old.md") && committed.contains("new.md"),
            "{committed}"
        );
        assert!(
            dirty_paths(&repo).unwrap().is_empty(),
            "nothing left uncommitted"
        );
    }

    #[test]
    fn commit_paths_pushes_to_origin() {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("origin.git");
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "-q", "--bare"]);

        let repo = dir.path().join("repo");
        init_repo(&repo);
        git(&repo, &["remote", "add", "origin", bare.to_str().unwrap()]);
        std::fs::write(repo.join("README.md"), "hello\n").unwrap();
        commit_all(&repo, "seed").unwrap();
        push(&repo).unwrap();

        std::fs::write(repo.join("new.md"), "new\n").unwrap();
        let mut paths = BTreeSet::new();
        paths.insert(PathBuf::from("new.md"));
        commit_paths(&repo, "test: push", &paths).unwrap();

        let clone = dir.path().join("clone");
        let status = Command::new("git")
            .args(["clone", "-q", bare.to_str().unwrap()])
            .arg(&clone)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(clone.join("new.md").exists());
    }

    #[test]
    fn commit_paths_is_a_noop_on_an_empty_set() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_repo(&repo);
        std::fs::write(repo.join("README.md"), "hello\n").unwrap();
        commit_all(&repo, "seed").unwrap();
        commit_paths(&repo, "should not run", &BTreeSet::new()).unwrap();
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
