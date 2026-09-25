//! `amd log` and `amd changelog`: a generated view over git history, and a
//! `CHANGELOG.md` written from it. Neither depends on the commit message
//! format — both read `git log --name-status`, which works retroactively
//! over history from before either command existed, and reads the same way
//! in a tickets repository (whose top-level directories are board columns)
//! or a documents repository (`adr/`, `rfc/`, `docs/`) — see
//! `012-have-amd-commit-board-and-document-changes` and
//! `041-rename-init-file-name` in `agile-md-tickets`. Deliberately not
//! `lite-actions/simple-commits-log`: checked directly, that repository (and
//! `simple-commit`, which the commit-message design once assumed) doesn't
//! exist anywhere in this org.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use agile_md::git;

/// One line of history: when, what happened, to which file, and the commit
/// that recorded it.
pub struct Entry {
    pub date: String,
    pub action: String,
    pub id: String,
    pub title: String,
    pub sha: String,
}

const UNIT_SEP: char = '\u{1f}';
const HEADER_PREFIX: &str = "commit\u{1f}";

/// Every entry in `repo`'s history, newest first — parsed from `git log
/// --name-status -M`, so this works over history from before either command
/// existed and is indifferent to whatever the commit message says.
pub fn entries(repo: &Path) -> Result<Vec<Entry>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "log",
            "--name-status",
            "-M",
            "--date=format:%Y-%m-%d %H:%M",
            &format!("--pretty=format:{HEADER_PREFIX}%h{UNIT_SEP}%ad"),
        ])
        .output()
        .context("running git log")?;
    if !out.status.success() {
        bail!("git log failed: {}", String::from_utf8_lossy(&out.stderr));
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let mut entries = Vec::new();
    let mut current: Option<(String, String)> = None;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(HEADER_PREFIX) {
            let mut parts = rest.splitn(2, UNIT_SEP);
            let sha = parts.next().unwrap_or_default().to_string();
            let date = parts.next().unwrap_or_default().to_string();
            current = Some((sha, date));
            continue;
        }
        let Some((sha, date)) = &current else {
            continue;
        };
        if let Some(entry) = parse_status_line(line, sha, date) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

/// One `git log --name-status` file line: `STATUS\tpath` or, for a detected
/// rename, `R100\told\tnew`. A rename between two different top-level
/// directories is read as a transition to the destination directory's name
/// (a board column, or a documents-repo directory like `adr/`) — the same
/// reading ticket 012 designed for board columns specifically, generalised
/// here to any top-level directory so it works in a documents repository
/// too, without needing board/column knowledge in this module at all.
fn parse_status_line(line: &str, sha: &str, date: &str) -> Option<Entry> {
    if line.is_empty() {
        return None;
    }
    let mut fields = line.split('\t');
    let status = fields.next()?;
    let first_path = fields.next()?;
    // CHANGELOG.md's own updates are never an entry — without this, `refresh`
    // would never converge: each regeneration adds an entry documenting
    // itself, which the next regeneration would then also need to include.
    if first_path == "CHANGELOG.md" {
        return None;
    }

    let (action, path) = if let Some(second_path) = fields.next() {
        let old_dir = top_dir(first_path);
        let new_dir = top_dir(second_path);
        let action = if old_dir != new_dir {
            new_dir.to_string()
        } else {
            "rename".to_string()
        };
        (action, second_path)
    } else {
        let action = match status.chars().next()? {
            'A' => "add",
            'M' => "edit",
            'D' => "delete",
            _ => "change",
        }
        .to_string();
        (action, first_path)
    };

    let stem = Path::new(path).file_stem()?.to_string_lossy().into_owned();
    let (id, title) = match stem.split_once('-') {
        Some((id, rest)) if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) => {
            (id.to_string(), rest.replace('-', " "))
        }
        _ => (String::new(), stem.replace('-', " ")),
    };

    Some(Entry {
        date: date.to_string(),
        action,
        id,
        title,
        sha: sha.to_string(),
    })
}

fn top_dir(path: &str) -> &str {
    path.split('/').next().unwrap_or(path)
}

/// Renders `entries` as `date  action  id  title  sha`, one per line — the
/// live view behind `amd log`.
pub fn render(entries: &[Entry]) -> String {
    entries
        .iter()
        .map(|e| {
            format!(
                "{}  {:<8} {:<5} {:<40} {}",
                e.date, e.action, e.id, e.title, e.sha
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_markdown(entries: &[Entry]) -> String {
    let mut out = String::from("# Changelog\n\n");
    for e in entries {
        out.push_str(&format!(
            "- `{}` {} **{}** {} ({})\n",
            e.date, e.action, e.id, e.title, e.sha
        ));
    }
    out
}

/// Regenerates `CHANGELOG.md` at `repo`'s root from git history, and — if it
/// changed — commits and pushes it as its own follow-up commit. Safe to call
/// any time, by a human, by `amd changelog` directly, or from CI: it only
/// ever touches that one file, and it only ever records commits that are
/// already final history by the time it runs, which is what lets it carry a
/// correct sha for every entry without the commit-can't-contain-its-own-sha
/// problem a same-commit changelog would have.
pub fn refresh(repo: &Path) -> Result<bool> {
    let rendered = render_markdown(&entries(repo)?);
    let path = repo.join("CHANGELOG.md");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if existing == rendered {
        return Ok(false);
    }
    std::fs::write(&path, &rendered).with_context(|| format!("writing {}", path.display()))?;
    git::commit(repo, "chore: update changelog", &[path])?;
    if git::remote_url(repo).is_none() {
        return Ok(true);
    }
    match git::push_checked(repo)? {
        git::PushOutcome::Ok => {}
        git::PushOutcome::Rejected => {
            git::pull_rebase(repo)?;
            if let git::PushOutcome::Rejected = git::push_checked(repo)? {
                bail!("push still rejected after a rebase — CHANGELOG.md is committed locally");
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;

    fn git(repo: &Path, args: &[&str]) {
        let status = StdCommand::new("git")
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
    fn an_added_file_reads_as_add() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_repo(&repo);
        std::fs::create_dir_all(repo.join("backlog")).unwrap();
        std::fs::write(repo.join("backlog/001-first-ticket.md"), "x").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "add: 001"]);

        let entries = entries(&repo).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "add");
        assert_eq!(entries[0].id, "001");
        assert_eq!(entries[0].title, "first ticket");
    }

    #[test]
    fn a_rename_across_directories_reads_as_the_destination_directory() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_repo(&repo);
        std::fs::create_dir_all(repo.join("backlog")).unwrap();
        std::fs::write(repo.join("backlog/001-first-ticket.md"), "x").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "add"]);

        std::fs::create_dir_all(repo.join("todo")).unwrap();
        git(
            &repo,
            &[
                "mv",
                "backlog/001-first-ticket.md",
                "todo/001-first-ticket.md",
            ],
        );
        git(&repo, &["commit", "-q", "-m", "todo"]);

        let entries = entries(&repo).unwrap();
        assert_eq!(entries[0].action, "todo");
        assert_eq!(entries[0].id, "001");
    }

    #[test]
    fn a_modified_file_reads_as_edit() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_repo(&repo);
        std::fs::create_dir_all(repo.join("backlog")).unwrap();
        std::fs::write(repo.join("backlog/001-first-ticket.md"), "x").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "add"]);

        std::fs::write(repo.join("backlog/001-first-ticket.md"), "y").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "edit"]);

        let entries = entries(&repo).unwrap();
        assert_eq!(entries[0].action, "edit");
    }

    #[test]
    fn a_non_numeric_stem_has_no_id_but_still_renders() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_repo(&repo);
        std::fs::write(repo.join("README.md"), "x").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "add readme"]);

        let entries = entries(&repo).unwrap();
        assert_eq!(entries[0].id, "");
        assert_eq!(entries[0].title, "README");
    }

    #[test]
    fn refresh_writes_the_file_and_reports_true_the_first_time() {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("origin.git");
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "-q", "--bare"]);

        let repo = dir.path().join("repo");
        init_repo(&repo);
        git(&repo, &["remote", "add", "origin", bare.to_str().unwrap()]);
        std::fs::write(repo.join("README.md"), "x").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "seed"]);
        git(&repo, &["push", "-q", "-u", "origin", "HEAD"]);

        assert!(refresh(&repo).unwrap());
        assert!(repo.join("CHANGELOG.md").exists());
    }

    #[test]
    fn refresh_is_a_no_op_when_nothing_changed() {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("origin.git");
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "-q", "--bare"]);

        let repo = dir.path().join("repo");
        init_repo(&repo);
        git(&repo, &["remote", "add", "origin", bare.to_str().unwrap()]);
        std::fs::write(repo.join("README.md"), "x").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "seed"]);
        git(&repo, &["push", "-q", "-u", "origin", "HEAD"]);

        assert!(refresh(&repo).unwrap());
        assert!(!refresh(&repo).unwrap(), "second call should be a no-op");
    }
}
