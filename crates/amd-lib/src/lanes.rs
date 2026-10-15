//! Swim lanes (tickets 218, 220): the project's other boards (declared
//! repositories with a board of their own) shown in amdui as rows under its
//! tickets board, view only, instead of merged into its columns. Which
//! boards, and whether lanes show at all, belong to you on this machine:
//! `ui.lanes` (off unless on) and `ui.lane-boards` (board repository paths,
//! comma separated, as `ui.hidden-projects` is) in agile-md's own config,
//! never committed.

use std::path::{Path, PathBuf};

use crate::settings::Config;

const ENABLED: &str = "ui.lanes";
const CHOSEN: &str = "ui.lane-boards";

/// Whether swim lanes are switched on.
pub fn enabled() -> bool {
    enabled_in(&Config::load())
}

fn enabled_in(config: &Config) -> bool {
    config.get(ENABLED).is_some_and(|v| v.trim() == "on")
}

/// Switches swim lanes on or off; the chosen projects are kept either way.
pub fn set_enabled(on: bool) -> anyhow::Result<()> {
    let mut config = Config::load();
    if on {
        config.set(ENABLED, "on");
    } else {
        config.remove(ENABLED);
    }
    config.save()
}

/// The boards chosen as lanes, by their board repository's path.
pub fn chosen() -> Vec<PathBuf> {
    chosen_in(&Config::load())
}

fn chosen_in(config: &Config) -> Vec<PathBuf> {
    config
        .get(CHOSEN)
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Adds `root`'s board to the lanes, or takes it off. A path that no
/// longer exists is dropped while the list is rewritten.
pub fn set_chosen(root: &Path, on: bool) -> anyhow::Result<()> {
    let mut config = Config::load();
    set_chosen_in(&mut config, root, on);
    config.save()
}

fn set_chosen_in(config: &mut Config, root: &Path, on: bool) {
    let mut list = chosen_in(config);
    list.retain(|p| p != root && p.exists());
    if on {
        list.push(root.to_path_buf());
    }
    let joined: Vec<String> = list.iter().map(|p| p.display().to_string()).collect();
    if joined.is_empty() {
        config.remove(CHOSEN);
    } else {
        config.set(CHOSEN, &joined.join(","));
    }
}

/// Which of `boards` (repository roots, the project's own tickets board
/// first) are drawn as lanes: the chosen ones after the first, while lanes
/// are on.
pub fn lane_roots<'a>(boards: impl IntoIterator<Item = &'a Path>) -> Vec<PathBuf> {
    if !enabled() {
        return Vec::new();
    }
    pick(boards, &chosen())
}

fn pick<'a>(boards: impl IntoIterator<Item = &'a Path>, chosen: &[PathBuf]) -> Vec<PathBuf> {
    boards
        .into_iter()
        .skip(1)
        .filter(|root| chosen.iter().any(|c| c == root))
        .map(Path::to_path_buf)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lanes_are_off_until_switched_on_and_keep_their_projects() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config");
        let mut config = Config::load_from(&file);
        assert!(!enabled_in(&config));
        config.set(ENABLED, "on");
        assert!(enabled_in(&config));

        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        set_chosen_in(&mut config, &a, true);
        set_chosen_in(&mut config, &b, true);
        set_chosen_in(&mut config, &a, true); // not twice
        assert_eq!(chosen_in(&config), [b.clone(), a.clone()]);
        config.save_to(&file).unwrap();
        assert_eq!(chosen_in(&Config::load_from(&file)), [b.clone(), a.clone()]);

        // One gone from disk is dropped the next time the list is written.
        std::fs::remove_dir(&b).unwrap();
        set_chosen_in(&mut config, &a, false);
        assert!(chosen_in(&config).is_empty());
        assert!(config.get(CHOSEN).is_none());
    }

    #[test]
    fn the_tickets_board_is_never_a_lane() {
        let boards = [Path::new("/t"), Path::new("/api"), Path::new("/web")];
        let chosen = [PathBuf::from("/web"), PathBuf::from("/t")];
        assert_eq!(pick(boards, &chosen), [PathBuf::from("/web")]);
    }
}
