//! Who gets Admin in the desktop board (ticket 149).
//!
//! Two sources, either of which grants it:
//!
//! - the project's `admins` list in the tickets `.settings` (emails, or
//!   `Name <email>` as `users` holds them), for teams not on GitHub or not
//!   using `gh`;
//! - GitHub: admin or maintain access to the tickets repository, asked
//!   through the `gh` CLI (rite's own git traffic goes over SSH, which can't
//!   ask GitHub's API anything).
//!
//! When neither can say (no list, and no `gh`, offline, or not a GitHub
//! repository), Admin is shown, with a note: hiding it would lock people
//! out of their own boards. This is a convenience, not a lock: anyone who
//! can push to the tickets repository can change `.settings` by hand.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::board::Board;
use crate::board_settings;
use crate::git;

/// Whether to show Admin, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminAccess {
    pub allowed: bool,
    /// Said beside Admin: how this was decided, or why it couldn't be.
    pub note: String,
}

/// What GitHub says about the current user and a repository.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHub {
    /// Admin or maintain access.
    Admin,
    /// Any lesser access.
    NotAdmin,
    /// Couldn't ask: not on GitHub, no `gh`, not signed in, offline.
    Unknown,
}

/// Decides Admin for `board`'s project, asking GitHub through `ask` (so
/// tests needn't). The board's repository is the tickets repository.
pub fn admin_access_with(board: &Board, ask: impl Fn(&Path) -> GitHub) -> AdminAccess {
    let repo = board.repo_root();
    let admins = board_settings::load(board).admins();
    let me = git::config(repo, "user.email").map(|e| e.to_lowercase());
    let listed = me.as_ref().is_some_and(|me| admins.contains(me));
    if listed {
        return AdminAccess {
            allowed: true,
            note: "You're on the project's admins list.".into(),
        };
    }
    match ask(repo) {
        GitHub::Admin => AdminAccess {
            allowed: true,
            note: "You have admin access to the tickets repository on GitHub.".into(),
        },
        GitHub::NotAdmin if admins.is_empty() => AdminAccess {
            allowed: false,
            note: "Admin is for people with admin or maintain access to the tickets repository."
                .into(),
        },
        GitHub::NotAdmin => AdminAccess {
            allowed: false,
            note: "Admin is for the project's admins list, or admin access on GitHub.".into(),
        },
        GitHub::Unknown if admins.is_empty() => AdminAccess {
            allowed: true,
            note: "rite couldn't check your access (it asks GitHub through the gh CLI), so \
                   Admin is open. A project can name its admins in .settings (`admins = …`)."
                .into(),
        },
        GitHub::Unknown => AdminAccess {
            allowed: false,
            note: "Admin is for the project's admins list.".into(),
        },
    }
}

/// [`admin_access_with`], asking GitHub for real.
pub fn admin_access(board: &Board) -> AdminAccess {
    admin_access_with(board, github_access)
}

/// Asks GitHub, through `gh`, for the current user's access to `repo`'s
/// `origin`.
pub fn github_access(repo: &Path) -> GitHub {
    let Some(identity) = git::remote_identity(repo) else {
        return GitHub::Unknown;
    };
    let Some(name) = identity.strip_prefix("github.com/") else {
        return GitHub::Unknown;
    };
    let out = Command::new("gh")
        .args(["api", &format!("repos/{name}"), "--jq", ".permissions"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match out {
        Ok(out) if out.status.success() => permissions(&String::from_utf8_lossy(&out.stdout)),
        _ => GitHub::Unknown,
    }
}

/// Reads `gh api`'s `.permissions` object: `{"admin":true,"maintain":…}`.
fn permissions(json: &str) -> GitHub {
    let granted = |key: &str| {
        json.split(',')
            .any(|part| part.contains(&format!("\"{key}\"")) && part.contains("true"))
    };
    if !json.contains("\"admin\"") {
        return GitHub::Unknown;
    }
    if granted("admin") || granted("maintain") {
        GitHub::Admin
    } else {
        GitHub::NotAdmin
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn board_as(email: &str, settings: &str) -> (tempfile::TempDir, Board) {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap();
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", email]);
        let board = Board::at(dir.path().join("tasks"));
        board.create().unwrap();
        fs::write(dir.path().join(".settings"), settings).unwrap();
        (dir, board)
    }

    #[test]
    fn the_admins_list_grants_it_whatever_github_says() {
        let (_d, board) = board_as("Sam@Example.com", "admins = Sam <sam@example.com>\n");
        assert!(admin_access_with(&board, |_| GitHub::NotAdmin).allowed);
    }

    #[test]
    fn github_admin_or_maintain_grants_it() {
        let (_d, board) = board_as("sam@example.com", "");
        assert!(admin_access_with(&board, |_| GitHub::Admin).allowed);
        assert!(!admin_access_with(&board, |_| GitHub::NotAdmin).allowed);
    }

    #[test]
    fn unknown_opens_it_unless_the_project_has_a_list() {
        let (_d, board) = board_as("sam@example.com", "");
        let open = admin_access_with(&board, |_| GitHub::Unknown);
        assert!(open.allowed && open.note.contains("couldn't check"));
        let (_d, board) = board_as("sam@example.com", "admins = alex@example.com\n");
        assert!(!admin_access_with(&board, |_| GitHub::Unknown).allowed);
    }

    #[test]
    fn gh_permissions_are_read() {
        let json = r#"{"admin":false,"maintain":true,"pull":true,"push":true,"triage":true}"#;
        assert_eq!(permissions(json), GitHub::Admin);
        let json = r#"{"admin":false,"maintain":false,"pull":true,"push":true,"triage":true}"#;
        assert_eq!(permissions(json), GitHub::NotAdmin);
        assert_eq!(permissions(""), GitHub::Unknown);
    }
}
