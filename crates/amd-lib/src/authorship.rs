//! Who created a ticket and who last edited it (ticket 155), so when a
//! ticket isn't clear you know who to ask.
//!
//! **Created** is stored: `created-by: "Name <email>"`, written once when the
//! ticket is made and never changed after, so it can't go stale. A ticket
//! from before this has none; then the first commit that added it answers,
//! and the field is written in the next time the ticket is saved.
//!
//! **Last edited** is never stored, because an edit made in a text editor
//! wouldn't update it: it's read from git, the latest commit that changed
//! the ticket's text. A move between columns that changes nothing else
//! doesn't count.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::board::Board;
use crate::board_settings;
use crate::git;
use crate::task::{self, Task};

/// The frontmatter key.
pub const CREATED_BY: &str = "created-by";

/// Someone, and when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    /// The person's name as the project knows them.
    pub name: String,
    pub email: String,
    /// `YYYY-MM-DD`; empty when only the stored field said who.
    pub date: String,
}

/// `Name <email>` for whoever is using amd in `board`'s repository: their
/// name from the project's users list when their email is on it (that's how
/// the team knows them), else git's `user.name`. `None` without a git email.
pub fn current_person(board: &Board) -> Option<String> {
    let repo = board.repo_root();
    let email = git::config(repo, "user.email")?;
    let name = board_settings::load(board)
        .users()
        .into_iter()
        .find(|u| u.email.eq_ignore_ascii_case(&email))
        .map(|u| u.name)
        .or_else(|| git::config(repo, "user.name"))
        .unwrap_or_else(|| email.clone());
    Some(format!("{name} <{email}>"))
}

/// `text` with `created-by` set to `person` when it has none (a custom
/// template that doesn't write it, or an older ticket); unchanged
/// otherwise. A ticket's creator, once recorded, never changes.
pub fn ensure_created_by(text: &str, person: &str) -> anyhow::Result<String> {
    if !text.starts_with("---") {
        return Ok(text.to_string());
    }
    let has = task::meta_in(text, CREATED_BY)
        .map(|v| v.trim().trim_matches('"'))
        .is_some_and(|v| !v.is_empty());
    if has {
        return Ok(text.to_string());
    }
    task::set_meta(text, CREATED_BY, &format!("{person:?}"))
}

/// Who created `task`: its `created-by`, else the author of the first commit
/// that added it.
pub fn created(board: &Board, task: &Task) -> Option<Mark> {
    let first = first_commit(board.repo_root(), &task.path);
    // The ticket's own `created:` date is its record; git's is the fallback.
    let date = |git: Option<&Mark>| {
        task.meta("created")
            .map(|d| d.trim().trim_matches('"').to_string())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| git.map(|m| m.date.clone()).unwrap_or_default())
    };
    if let Some(stored) = task.meta(CREATED_BY).filter(|v| !v.trim().is_empty()) {
        let (name, email) = split_person(&stored);
        let date = date(first.as_ref());
        return Some(Mark { name, email, date });
    }
    let first = first?;
    let date = date(Some(&first));
    Some(Mark {
        date,
        ..known_as(board, first)
    })
}

/// Who last changed `task`'s text, and when: the latest commit touching it
/// that wasn't a pure move between columns. `None` for a ticket never
/// committed.
pub fn last_edited(board: &Board, task: &Task) -> Option<Mark> {
    let repo = board.repo_root();
    let path = task.path.strip_prefix(repo).unwrap_or(&task.path);
    let out = git_out(
        repo,
        &[
            "log",
            "--follow",
            "-M",
            "--name-status",
            "--date=short",
            "--format=>>>%an%x1f%ae%x1f%ad",
            "--",
        ],
        Some(path),
    )?;
    let mut current: Option<Mark> = None;
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix(">>>") {
            let mut parts = rest.split('\u{1f}');
            current = Some(Mark {
                name: parts.next().unwrap_or_default().to_string(),
                email: parts.next().unwrap_or_default().to_string(),
                date: parts.next().unwrap_or_default().to_string(),
            });
            continue;
        }
        // `R100` is a rename with nothing changed: a move, not an edit.
        let status = line.split('\t').next().unwrap_or_default();
        if status.is_empty() || status == "R100" {
            continue;
        }
        if let Some(mark) = current.take() {
            return Some(known_as(board, mark));
        }
    }
    None
}

/// "Name <email>" as (name, email); a bare email is its own name.
fn split_person(value: &str) -> (String, String) {
    let value = value.trim().trim_matches('"');
    match value.split_once('<') {
        Some((name, rest)) => (
            name.trim().to_string(),
            rest.trim_end_matches('>').trim().to_string(),
        ),
        None => (value.to_string(), value.to_string()),
    }
}

/// `mark` with the name the project's users list gives its email.
fn known_as(board: &Board, mut mark: Mark) -> Mark {
    if let Some(user) = board_settings::load(board)
        .users()
        .into_iter()
        .find(|u| u.email.eq_ignore_ascii_case(&mark.email))
    {
        mark.name = user.name;
    }
    mark
}

/// The first commit adding a file with `path`'s name anywhere in the
/// repository: a move between columns keeps the name. Not `--follow`, which
/// traces template-alike tickets back to the first ticket (see renumber).
fn first_commit(repo: &Path, path: &Path) -> Option<Mark> {
    let name = path.file_name()?.to_str()?;
    let spec = format!(":(glob)**/{name}");
    let out = git_out(
        repo,
        &[
            "log",
            "--diff-filter=A",
            "--date=short",
            "--format=%an%x1f%ae%x1f%ad",
            "--",
            &spec,
        ],
        None,
    )?;
    let line = out.lines().last()?;
    let mut parts = line.split('\u{1f}');
    Some(Mark {
        name: parts.next()?.to_string(),
        email: parts.next().unwrap_or_default().to_string(),
        date: parts.next().unwrap_or_default().to_string(),
    })
}

fn git_out(repo: &Path, args: &[&str], path: Option<&Path>) -> Option<String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo).args(args);
    if let Some(path) = path {
        command.arg(path);
    }
    let out = command.stderr(Stdio::null()).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// "created by Priya Shah on 2026-10-02; last edited by Sam Lee on
/// 2026-10-08", or the half there is; `None` when neither is known.
pub fn summary(board: &Board, task: &Task) -> Option<String> {
    let on = |m: &Mark| {
        if m.date.is_empty() {
            m.name.clone()
        } else {
            format!("{} on {}", m.name, m.date)
        }
    };
    match (created(board, task), last_edited(board, task)) {
        (Some(c), Some(e)) => Some(format!("created by {}; last edited by {}", on(&c), on(&e))),
        (Some(c), None) => Some(format!("created by {}", on(&c))),
        (None, Some(e)) => Some(format!("last edited by {}", on(&e))),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Column;
    use std::fs;

    fn git(dir: &Path, args: &[&str]) {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .unwrap()
                .status
                .success(),
            "git {args:?}"
        );
    }

    fn as_person(dir: &Path, name: &str, email: &str) {
        git(dir, &["config", "user.name", name]);
        git(dir, &["config", "user.email", email]);
    }

    #[test]
    fn created_by_is_filled_once_and_never_replaced() {
        let text = "---\nid: \"001\"\n---\n\nbody\n";
        let filled = ensure_created_by(text, "Priya Shah <priya@example.com>").unwrap();
        assert!(filled.contains("created-by: \"Priya Shah <priya@example.com>\""));
        assert_eq!(
            ensure_created_by(&filled, "Someone Else <x@example.com>").unwrap(),
            filled
        );
    }

    #[test]
    fn created_and_last_edited_come_from_the_field_and_git() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(dir.path(), &["config", "commit.gpgsign", "false"]);
        let board = Board::at(dir.path().join("tasks"));
        board.create().unwrap();
        fs::write(
            dir.path().join(".settings"),
            "users = Priya Shah <priya@example.com>, Sam Lee <sam@example.com>\n",
        )
        .unwrap();

        // Priya creates two tickets: one with created-by, one (older) without.
        as_person(dir.path(), "priya", "priya@example.com");
        let new = board.dir(Column::Backlog).join("001-new.md");
        let old = board.dir(Column::Backlog).join("002-old.md");
        fs::write(&new, "---\nid: \"001\"\ncreated-by: \"Priya Shah <priya@example.com>\"\ncreated: \"2026-10-01\"\n---\n\nnew\n").unwrap();
        fs::write(&old, "---\nid: \"002\"\n---\n\nold\n").unwrap();
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-q", "-m", "create"]);

        // Sam edits 001; Alex only moves it.
        as_person(dir.path(), "sam", "sam@example.com");
        fs::write(&new, fs::read_to_string(&new).unwrap() + "more\n").unwrap();
        git(dir.path(), &["commit", "-qam", "edit"]);
        as_person(dir.path(), "alex", "alex@example.com");
        fs::create_dir_all(board.dir(Column::Todo)).unwrap();
        git(
            dir.path(),
            &["mv", "tasks/backlog/001-new.md", "tasks/todo/001-new.md"],
        );
        git(dir.path(), &["commit", "-qm", "move"]);

        let t1 = board.find("001").unwrap();
        let created1 = created(&board, &t1).unwrap();
        assert_eq!(
            (created1.name.as_str(), created1.date.as_str()),
            ("Priya Shah", "2026-10-01")
        );
        let edited = last_edited(&board, &t1).unwrap();
        assert_eq!(
            edited.name, "Sam Lee",
            "a move isn't an edit; the users list names him"
        );

        // The older ticket falls back to git's first commit.
        let t2 = board.find("002").unwrap();
        assert_eq!(created(&board, &t2).unwrap().name, "Priya Shah");
        assert!(
            summary(&board, &t2)
                .unwrap()
                .starts_with("created by Priya Shah on ")
        );
    }
}
