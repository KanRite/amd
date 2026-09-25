//! A lightweight, trust-on-first-use check that a board's declared project
//! identity hasn't changed unexpectedly.
//!
//! `board_settings.rs`'s immutability is enforced by `amd`'s own write path,
//! not a filesystem lock — nothing stops a direct hand-edit of `<repo-root>/.settings`.
//! This is the other half: a per-machine, **not** git-tracked record of the
//! project name `amd` last saw for each board it has visited, at
//! `$XDG_CONFIG_HOME/agile-md/project-trust`. Machine-generated bookkeeping,
//! not a preference, so it gets its own file — the same reasoning
//! `registry.rs` already gives for keeping the repository list out of the
//! settings file.
//!
//! This is deliberately not cryptographic. It catches an unexpected change
//! from what this machine saw before; it does not prove the file wasn't
//! tampered with by someone who also updates this record.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::board::Board;
use crate::board_settings;
use crate::settings::Config;

/// Checks the board's declared project, if it has one, against what this
/// machine last saw for it. The first time a board is seen, its current
/// value is adopted as trusted — that's an ordinary first clone, not drift.
pub fn check(board: &Board) -> Result<()> {
    let Some(store_path) = path() else {
        return Ok(());
    };
    check_at(board, &store_path)
}

/// The half that takes a path, so tests need no environment fiddling.
fn check_at(board: &Board, store_path: &Path) -> Result<()> {
    let Some(project) = board_settings::load(board).project().map(str::to_string) else {
        return Ok(());
    };
    let key = board.root.display().to_string();
    let mut trust = Config::load_from(store_path);
    match trust.get(&key) {
        None => {
            trust.set(&key, &project);
            trust.save_to(store_path)?;
        }
        Some(seen) if seen == project => {}
        Some(seen) => {
            bail!(
                "project identity for {} changed unexpectedly: last seen '{seen}', now '{project}'. \
                 If this is intentional, remove the entry for this path from {} and re-run.",
                board.root.display(),
                store_path.display(),
            );
        }
    }
    Ok(())
}

/// `$XDG_CONFIG_HOME/agile-md/project-trust`, else `~/.config/agile-md/project-trust`.
fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("agile-md").join("project-trust"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn board(dir: &Path) -> Board {
        Board::at(dir.join("tasks"))
    }

    fn set_project(board: &Board, name: &str) {
        fs::create_dir_all(&board.root).unwrap();
        let mut settings = board_settings::load(board);
        settings.set_project(name).unwrap();
        settings.save(board).unwrap();
    }

    #[test]
    fn a_board_with_no_declared_project_is_never_a_problem() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let store = dir.path().join("trust");
        check_at(&board, &store).unwrap();
        assert!(!store.exists(), "nothing to trust, nothing written");
    }

    #[test]
    fn the_first_sighting_is_adopted_silently() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        set_project(&board, "agile-md");
        let store = dir.path().join("trust");

        check_at(&board, &store).unwrap();
        assert_eq!(
            Config::load_from(&store).get(&board.root.display().to_string()),
            Some("agile-md")
        );
    }

    #[test]
    fn a_matching_second_sighting_is_quiet() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        set_project(&board, "agile-md");
        let store = dir.path().join("trust");

        check_at(&board, &store).unwrap();
        check_at(&board, &store).unwrap();
    }

    #[test]
    fn a_changed_value_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let store = dir.path().join("trust");

        set_project(&board, "agile-md");
        check_at(&board, &store).unwrap();

        // Simulate a hand-edit of the board-local settings file: rewrite it
        // directly rather than going through `set_project`, which would
        // itself refuse the change.
        fs::write(board_settings::path(&board), "project = widgets\n").unwrap();

        let err = check_at(&board, &store).unwrap_err().to_string();
        assert!(err.contains("agile-md"), "{err}");
        assert!(err.contains("widgets"), "{err}");
    }
}
