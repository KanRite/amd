//! Archiving a project (ticket 157): `archived = on` in the tickets
//! `.settings`, committed and pushed like any setting, so every clone sees
//! it. An archived project is read-only for amd — the CLI and amdui refuse
//! changes until it's unarchived — and is left out of My work.
//!
//! Optionally it's archived on GitHub too (`gh repo archive`): the tickets
//! repository, the project's own documents repository and every repository
//! it declares; never the shared documents repository, which other projects
//! read. Order matters, since GitHub refuses pushes to an archived
//! repository: archiving pushes the setting first and archives after;
//! unarchiving unarchives first and pushes after.

use anyhow::{Result, bail};

use crate::board::Board;
use crate::board_settings;
use crate::current_project;
use crate::import_issues::{github_slug, on_github};
use crate::transition::{self, Event};

/// Whether `board`'s project is archived.
pub fn is_archived(board: &Board) -> bool {
    board_settings::load(board).archived()
}

/// Refuses a change to an archived project, naming the way back; and to a
/// project whose team requires a newer agile-md than this one (ticket 193,
/// [`crate::compat::ensure_version`]).
pub fn ensure_writable(board: &Board) -> Result<()> {
    if is_archived(board) {
        bail!(
            "{} is archived, so it can't be changed; `amd project unarchive` brings it back",
            current_project::project_name(board.repo_root())
        );
    }
    crate::compat::ensure_version(board)
}

/// The project's repositories on GitHub, as `owner/repo`: its tickets
/// repository, its own documents repository and each declared one.
pub fn github_repos(board: &Board) -> Vec<String> {
    let project = current_project::project_name(board.repo_root());
    let mut names = vec![format!("{project}-documents")];
    names.extend(board_settings::load(board).repos());
    let mut slugs: Vec<String> = github_slug(board.repo_root()).into_iter().collect();
    for (_, slug) in on_github(board, &names) {
        if !slugs.contains(&slug) {
            slugs.push(slug);
        }
    }
    slugs
}

/// Runs `gh repo archive|unarchive <owner/repo> --yes`.
pub fn gh(verb: &str, slug: &str) -> Result<()> {
    let out = std::process::Command::new("gh")
        .args(["repo", verb, slug, "--yes"])
        .output()
        .map_err(|e| anyhow::anyhow!("couldn't run gh: {e}"))?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Archives (`on`) or unarchives the project, and with `github` its
/// repositories on GitHub through `gh` (`gh(verb, slug)`, [`gh`] for
/// real). Already in that state, only the GitHub half runs, so a project
/// archived in amd alone can be archived on GitHub later. What happened
/// comes back as events; a GitHub failure is a warning, not an error.
pub fn set_archived(
    board: &Board,
    on: bool,
    github: bool,
    gh: &mut dyn FnMut(&str, &str) -> Result<()>,
) -> Result<Vec<Event>> {
    let project = current_project::project_name(board.repo_root());
    let verb = if on { "archive" } else { "unarchive" };
    let mut events = Vec::new();
    let slugs = if github {
        github_repos(board)
    } else {
        Vec::new()
    };
    if github && slugs.is_empty() {
        events.push(Event::Warn(format!(
            "none of {project}'s repositories is on GitHub here, so nothing was {verb}d there"
        )));
    }
    let mut on_github = |events: &mut Vec<Event>| {
        for slug in &slugs {
            match gh(verb, slug) {
                Ok(()) => events.push(Event::Info(format!("{verb}d {slug} on GitHub"))),
                Err(e) => events.push(Event::Warn(format!(
                    "couldn't {verb} {slug} on GitHub: {e:#}"
                ))),
            }
        }
    };

    // Unarchived on GitHub first, or the setting's push is refused.
    if !on {
        on_github(&mut events);
    }
    let settings = board_settings::load(board);
    let mut pushed = true;
    if settings.archived() == on {
        events.push(Event::Info(format!("{project} is already {verb}d")));
    } else {
        let detail = if on { "on" } else { "off" };
        let ((), warning) =
            transition::with_auto_commit(board, "settings", "archived", detail, || {
                let mut settings = board_settings::load(board);
                settings.set_archived(on);
                settings.save(board)
            })?;
        events.push(Event::Info(format!("{verb}d {project}")));
        if let Some(warning) = warning {
            pushed = false;
            events.push(Event::Warn(warning));
        }
        if !settings.auto_commit() {
            pushed = false;
        }
    }
    // Archived on GitHub only once the setting is there, or no clone could
    // ever push it.
    if on && !slugs.is_empty() {
        if pushed {
            on_github(&mut events);
        } else {
            events.push(Event::Warn("not archived on GitHub: the change isn't pushed yet; push it, then `amd project archive --github`".to_string()));
        }
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    /// A tickets repository with a board, pushed to a bare origin.
    fn project(tmp: &std::path::Path) -> Board {
        let origin = tmp.join("origin.git");
        std::fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "-q", "--bare", "-b", "main"]);
        let repo = tmp.join("demo-tickets");
        std::fs::create_dir_all(repo.join("tasks/backlog")).unwrap();
        std::fs::write(repo.join("tasks/backlog/.gitkeep"), "").unwrap();
        std::fs::write(repo.join(".settings"), "project = demo\n").unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        git(&repo, &["config", "user.name", "T"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        git(
            &repo,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        git(&repo, &["push", "-q", "-u", "origin", "main"]);
        Board::in_repo(&repo)
    }

    #[test]
    fn the_setting_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let board = project(tmp.path());
        assert!(!is_archived(&board));
        assert!(ensure_writable(&board).is_ok());
        let mut never = |_: &str, _: &str| -> Result<()> { panic!("no GitHub asked for") };
        set_archived(&board, true, false, &mut never).unwrap();
        assert!(is_archived(&board));
        let err = ensure_writable(&board).unwrap_err().to_string();
        assert!(err.contains("amd project unarchive"), "{err}");
        set_archived(&board, false, false, &mut never).unwrap();
        assert!(!is_archived(&board));
        let settings = std::fs::read_to_string(board.repo_root().join(".settings")).unwrap();
        assert!(!settings.contains("archived"), "{settings}");
    }

    #[test]
    fn github_comes_after_the_push_and_before_the_unarchive() {
        let tmp = tempfile::tempdir().unwrap();
        let board = project(tmp.path());
        // A GitHub origin for the slug; pushes still go to the bare repo.
        let bare = tmp.path().join("origin.git");
        git(
            board.repo_root(),
            &[
                "remote",
                "set-url",
                "origin",
                "git@github.com:acme/demo-tickets.git",
            ],
        );
        git(
            board.repo_root(),
            &[
                "remote",
                "set-url",
                "--push",
                "origin",
                bare.to_str().unwrap(),
            ],
        );
        let log = std::cell::RefCell::new(Vec::<String>::new());
        let archived_at_call = std::cell::RefCell::new(Vec::<bool>::new());
        let mut gh = |verb: &str, slug: &str| -> Result<()> {
            log.borrow_mut().push(format!("{verb} {slug}"));
            archived_at_call.borrow_mut().push(is_archived(&board));
            Ok(())
        };
        set_archived(&board, true, true, &mut gh).unwrap();
        set_archived(&board, false, true, &mut gh).unwrap();
        let log = log.into_inner();
        assert!(
            log.contains(&"archive acme/demo-tickets".to_string()),
            "{log:?}"
        );
        assert!(
            log.contains(&"unarchive acme/demo-tickets".to_string()),
            "{log:?}"
        );
        // Archive ran with the setting already on; unarchive with it still on.
        let at = archived_at_call.into_inner();
        assert!(at.iter().all(|on| *on), "{at:?}");
    }
}
