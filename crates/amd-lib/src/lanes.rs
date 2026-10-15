//! Swim lanes (ticket 218): other projects' boards shown under the one on
//! screen in amdui, view only. Which projects, and whether lanes show at
//! all, belong to you on this machine: `ui.lanes` (off unless on) and
//! `ui.lane-projects` (board repository paths, comma separated, as
//! `ui.hidden-projects` is) in agile-md's own config, never committed.

use std::path::{Path, PathBuf};

use crate::my_work::{self, Project};
use crate::settings::Config;

const ENABLED: &str = "ui.lanes";
const CHOSEN: &str = "ui.lane-projects";

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

/// The projects chosen as lanes, by their board repository's path.
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

/// Adds `root`'s project to the lanes, or takes it off. A path that no
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

/// The projects to draw as lanes under `current` (the board repository
/// on screen): those chosen that are still registered here, in the
/// registry's order, never `current` itself. Empty while lanes are off.
pub fn lane_projects(current: &Path) -> Vec<Project> {
    if !enabled() {
        return Vec::new();
    }
    pick(my_work::projects(), &chosen(), current)
}

fn pick(projects: Vec<Project>, chosen: &[PathBuf], current: &Path) -> Vec<Project> {
    projects
        .into_iter()
        .filter(|p| p.root != current && chosen.contains(&p.root))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(root: &str) -> Project {
        Project {
            name: root.trim_start_matches('/').to_string(),
            root: PathBuf::from(root),
            dir: PathBuf::from("/"),
            hidden: false,
            archived: false,
        }
    }

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
    fn the_project_on_screen_is_never_its_own_lane() {
        let all = vec![project("/one"), project("/two"), project("/three")];
        let chosen = [PathBuf::from("/three"), PathBuf::from("/one")];
        let names: Vec<String> = pick(all, &chosen, Path::new("/one"))
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["three"]);
    }
}
