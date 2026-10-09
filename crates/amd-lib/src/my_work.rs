//! My work (ticket 020): everything assigned to you, across every project
//! on this machine, in one list. It's what the desktop board's landing page
//! shows, and what `amd mine` prints.
//!
//! **Which projects**: every repository registered on this machine that
//! has a board (`init`, `join` and `register` all register), each with the
//! boards of the repositories its project declares, the way the desktop
//! board loads one project. Scratch boards under the system's temporary
//! directory are skipped, and so is any project you've hidden: a personal
//! list in your own config (`ui.hidden-projects`, by folder path), never
//! committed.
//!
//! **Which tickets**: todo and doing, the work you've committed to.
//! Assignees are stored as names, so "you" is your git email, looked up in
//! each project's users list for the name the team knows you by, plus your
//! git `user.name`. Read on demand, never cached: the markdown is the truth.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::board::{Board, Column};
use crate::current_project;
use crate::git;
use crate::registry::{self, Registry};
use crate::settings::Config;

/// Where the hidden projects are kept, in agile-md's own per-machine config.
const HIDDEN: &str = "ui.hidden-projects";

/// One project on this machine.
#[derive(Clone, Debug, Serialize)]
pub struct Project {
    pub name: String,
    /// Its board's repository (the tickets repository, usually).
    pub root: PathBuf,
    /// The folder holding its repositories.
    pub dir: PathBuf,
    /// Left out of My work by you (`ui.hidden-projects`).
    pub hidden: bool,
}

/// A ticket in My work.
#[derive(Clone, Debug, Serialize)]
pub struct WorkItem {
    pub project: String,
    /// The project's board repository, for opening it.
    pub project_root: PathBuf,
    /// The repository whose board the ticket is on.
    pub repo: String,
    pub column: String,
    pub id: String,
    /// The file name without `.md`.
    pub stem: String,
    pub title: String,
    pub priority: String,
    /// Empty when unsized.
    pub points: String,
    /// `YYYY-MM-DD`, empty when the ticket doesn't say.
    pub created: String,
    /// The epic or sprint it belongs to, by its `epic:` label.
    pub epic: String,
}

/// How to order My work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    /// Highest priority first, then oldest.
    Priority,
    /// Oldest first.
    Age,
    /// Largest first; unsized last.
    Points,
}

/// Every project registered on this machine, sorted by name, with your
/// hidden ones marked.
pub fn projects() -> Vec<Project> {
    projects_in(&Registry::load(), &hidden())
}

fn projects_in(registry: &Registry, hidden: &[PathBuf]) -> Vec<Project> {
    let temp = std::env::temp_dir();
    let mut out: Vec<Project> = registry
        .entries
        .iter()
        .filter(|e| e.has_board())
        .filter(|e| !is_scratch(&e.root, &temp))
        .map(|e| Project {
            name: current_project::project_name(&e.root),
            dir: registry::project_dir_of(&e.root),
            hidden: hidden.iter().any(|h| h == &e.root),
            root: e.root.clone(),
        })
        .collect();
    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.root.cmp(&b.root))
    });
    out
}

/// A board left behind by a test run or a scratch experiment: anything
/// under the system's temporary directory (`TMPDIR`, else `/tmp`).
fn is_scratch(root: &Path, temp: &Path) -> bool {
    let temp = temp.canonicalize().unwrap_or_else(|_| temp.to_path_buf());
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    root.starts_with(&temp)
}

/// The project folders you've hidden from My work.
pub fn hidden() -> Vec<PathBuf> {
    Config::load()
        .get(HIDDEN)
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Hides `root`'s project from My work, or shows it again. Saved in your
/// own config on this machine; nothing is committed.
pub fn set_hidden(root: &Path, hide: bool) -> anyhow::Result<()> {
    let mut list = hidden();
    list.retain(|p| p != root);
    if hide {
        list.push(root.to_path_buf());
    }
    let mut config = Config::load();
    let joined: Vec<String> = list.iter().map(|p| p.display().to_string()).collect();
    config.set(HIDDEN, &joined.join(","));
    config.save()
}

/// The boards of `project`: its own, then each repository it declares
/// that's registered here with a board (as the desktop board loads one
/// project), each with its repository's name.
pub fn boards_of(project: &Project, registry: &Registry) -> Vec<(String, Board)> {
    let own = Board::in_repo(&project.root);
    let declared = crate::board_settings::load(&own).repos();
    let mut boards = vec![(repo_name(&project.root), own)];
    for name in declared {
        if let Some(entry) = registry.find_near(&name, &project.dir)
            && entry.root != project.root
            && entry.has_board()
        {
            boards.push((repo_name(&entry.root), entry.board()));
        }
    }
    boards
}

fn repo_name(root: &Path) -> String {
    root.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string())
}

/// The names `email` goes by on `board`'s project, lowercased for
/// comparing: the name its users list gives that email when it's listed
/// (the list is how the team knows you), else `fallback` (git's
/// `user.name`). The email itself always counts.
fn names_for(board: &Board, email: &str, fallback: Option<&str>) -> Vec<String> {
    let mut names: Vec<String> = crate::board_settings::load(board)
        .users()
        .into_iter()
        .filter(|u| u.email.eq_ignore_ascii_case(email))
        .map(|u| u.name.to_lowercase())
        .collect();
    if names.is_empty() {
        names.extend(fallback.map(str::to_lowercase));
    }
    names.push(email.to_lowercase());
    names
}

/// Your work: todo and doing tickets assigned to you in every project on
/// this machine, hidden ones left out unless `include_hidden`, ordered by
/// `sort`. "You" is git's identity in each board's own repository (people
/// use a different email per organisation, as leases already allow), else
/// `email`/`name`. A board that can't be read is skipped, not fatal.
pub fn my_work(email: &str, name: Option<&str>, include_hidden: bool, sort: Sort) -> Vec<WorkItem> {
    let registry = Registry::load();
    let projects = projects_in(&registry, &hidden());
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut items = Vec::new();
    for project in projects.iter().filter(|p| include_hidden || !p.hidden) {
        for (repo, board) in boards_of(project, &registry) {
            // A repository two projects declare is read once.
            if seen.contains(&board.root) {
                continue;
            }
            seen.push(board.root.clone());
            let repo_email = git::config(board.repo_root(), "user.email");
            let repo_name = git::config(board.repo_root(), "user.name");
            let mine = match &repo_email {
                Some(e) => names_for(&board, e, repo_name.as_deref()),
                None => names_for(&board, email, name),
            };
            for column in [Column::Todo, Column::Doing] {
                let Ok(tasks) = board.tasks_in(column) else {
                    continue;
                };
                for task in tasks {
                    let assignee = task.assignee().unwrap_or_default().to_lowercase();
                    if assignee.is_empty() || !mine.contains(&assignee) {
                        continue;
                    }
                    items.push(WorkItem {
                        project: project.name.clone(),
                        project_root: project.root.clone(),
                        repo: repo.clone(),
                        column: column.to_string(),
                        id: task.id_display(),
                        stem: task.stem.clone(),
                        title: task.title(),
                        priority: task.priority(),
                        points: task.points().unwrap_or_default(),
                        created: task
                            .meta("created")
                            .map(|c| c.trim().trim_matches('"').to_string())
                            .unwrap_or_default(),
                        epic: task.epic_meta().unwrap_or_default(),
                    });
                }
            }
        }
    }
    sort_items(&mut items, sort);
    items
}

/// Orders My work. Ties fall back to oldest, then project and id, so the
/// list never shuffles between runs.
pub fn sort_items(items: &mut [WorkItem], sort: Sort) {
    let rank = |p: &str| match p {
        "high" => 0,
        "medium" => 1,
        _ => 2,
    };
    let points = |p: &str| p.parse::<u32>().ok();
    items.sort_by(|a, b| {
        let first = match sort {
            Sort::Priority => rank(&a.priority).cmp(&rank(&b.priority)),
            Sort::Age => std::cmp::Ordering::Equal,
            // Largest first; unsized (None) after every sized one.
            Sort::Points => match (points(&a.points), points(&b.points)) {
                (Some(x), Some(y)) => y.cmp(&x),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            },
        };
        first
            .then_with(|| oldest(&a.created, &b.created))
            .then_with(|| a.project.cmp(&b.project))
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// Older first; a ticket with no date after every dated one.
fn oldest(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.is_empty(), b.is_empty()) {
        (false, false) => a.cmp(b),
        (false, true) => std::cmp::Ordering::Less,
        (true, false) => std::cmp::Ordering::Greater,
        (true, true) => std::cmp::Ordering::Equal,
    }
}

/// You, as git knows you in `repo` (or anywhere, when it's global):
/// `(user.email, user.name)`.
pub fn me(repo: &Path) -> Option<(String, Option<String>)> {
    let email = git::config(repo, "user.email")?;
    Some((email, git::config(repo, "user.name")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(priority: &str, created: &str, points: &str, id: &str) -> WorkItem {
        WorkItem {
            project: "p".into(),
            project_root: PathBuf::from("/p"),
            repo: "p-tickets".into(),
            column: "todo".into(),
            id: id.into(),
            stem: format!("{id}-x"),
            title: id.into(),
            priority: priority.into(),
            points: points.into(),
            created: created.into(),
            epic: String::new(),
        }
    }

    fn ids(items: &[WorkItem]) -> Vec<&str> {
        items.iter().map(|i| i.id.as_str()).collect()
    }

    #[test]
    fn priority_then_oldest() {
        let mut items = vec![
            item("low", "2026-01-01", "", "001"),
            item("high", "2026-03-01", "", "002"),
            item("high", "2026-02-01", "", "003"),
            item("medium", "", "", "004"),
        ];
        sort_items(&mut items, Sort::Priority);
        assert_eq!(ids(&items), ["003", "002", "004", "001"]);
        sort_items(&mut items, Sort::Age);
        assert_eq!(ids(&items), ["001", "003", "002", "004"]);
    }

    #[test]
    fn points_largest_first_unsized_last() {
        let mut items = vec![
            item("low", "", "", "001"),
            item("low", "", "3", "002"),
            item("low", "", "8", "003"),
        ];
        sort_items(&mut items, Sort::Points);
        assert_eq!(ids(&items), ["003", "002", "001"]);
    }

    #[test]
    fn scratch_boards_are_skipped() {
        let temp = Path::new("/tmp");
        assert!(is_scratch(Path::new("/tmp/tmp.abc/s"), temp));
        assert!(!is_scratch(Path::new("/home/u/work/p-tickets"), temp));
    }
}
