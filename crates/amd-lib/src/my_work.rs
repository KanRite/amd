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
//!
//! **Nothing to do** (ticket 160): when none of that is yours, [`to_pick`]
//! offers work you could take from every project, hidden ones included:
//! with "include unassigned" on (the default), unassigned todo and doing
//! tickets first, then backlog tickets unassigned or yours; off, only
//! backlog tickets assigned to you. From the backlog, all the high-priority
//! ones, else all the medium, else the low.
//! A hidden project with something to pick is shown again
//! ([`show_projects_of`]), since that's where the work is.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::board::{Board, Column, Kind};
use crate::current_project;
use crate::git;
use crate::registry::{self, Registry};
use crate::settings::Config;

/// Where the hidden projects are kept, in agile-md's own per-machine config.
const HIDDEN: &str = "ui.hidden-projects";
/// Whether work to pick includes unassigned tickets (ticket 160).
const UNASSIGNED: &str = "ui.include-unassigned";

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
    /// Archived for everyone (ticket 157): never in My work or work to pick.
    pub archived: bool,
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
    /// The backlog folder it's filed in (an epic or sprint), which a front
    /// end needs to find it there; `None` outside the backlog.
    pub folder: Option<String>,
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
            archived: crate::archive::is_archived(&Board::in_repo(&e.root)),
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
    let mut items = Vec::new();
    each_board(include_hidden, email, name, |project, repo, board, mine| {
        // Todo- and doing-kind columns (ticket 223).
        let columns: Vec<Column> = board
            .workflow()
            .columns()
            .filter(|c| c.is(Kind::Todo) || c.is(Kind::Doing))
            .collect();
        for column in columns {
            let Ok(tasks) = board.tasks_in(column) else {
                continue;
            };
            for task in tasks {
                let assignee = task.assignee().unwrap_or_default().to_lowercase();
                if !assignee.is_empty() && mine.contains(&assignee) {
                    items.push(item(project, repo, &task));
                }
            }
        }
    });
    sort_items(&mut items, sort);
    items
}

/// Work you could pick up when nothing is yours (ticket 160), from every
/// project, hidden ones included, never a draft (a draft can't leave the
/// backlog) or a ticket locked by someone else.
///
/// With `unassigned`: unassigned todo and doing tickets first, highest
/// priority then oldest; failing those, backlog tickets that are
/// unassigned or yours. Without: only backlog tickets assigned to you.
/// Backlog tickets come only from the highest priority there is any of:
/// every high one, else every medium, else every low; oldest first.
pub fn to_pick(email: &str, name: Option<&str>, unassigned: bool) -> Vec<WorkItem> {
    let mut started = Vec::new();
    let mut backlog = Vec::new();
    each_board(true, email, name, |project, repo, board, mine| {
        let me = git::config(board.repo_root(), "user.email").unwrap_or_else(|| email.to_string());
        // Started work first, then the backlog-kind columns (ticket 223).
        let workflow = board.workflow();
        let columns: Vec<Column> = workflow
            .columns()
            .filter(|c| c.is(Kind::Todo) || c.is(Kind::Doing))
            .chain(workflow.of_kind(Kind::Backlog))
            .collect();
        for column in columns {
            if !column.is(Kind::Backlog) && !unassigned {
                continue;
            }
            let Ok(tasks) = board.tasks_in(column) else {
                continue;
            };
            for task in tasks {
                let assignee = task.assignee().unwrap_or_default();
                // In todo and doing, only work nobody has taken: your own
                // there is already My work.
                if !column.is(Kind::Backlog) && !assignee.trim().is_empty() {
                    continue;
                }
                let lock = task.lease_active().then(|| task.locked_by()).flatten();
                if pickable(
                    &assignee,
                    task.is_draft(),
                    lock.as_deref(),
                    mine,
                    &me,
                    unassigned,
                ) {
                    let found = item(project, repo, &task);
                    if column.is(Kind::Backlog) {
                        backlog.push(found);
                    } else {
                        started.push(found);
                    }
                }
            }
        }
    });
    if !started.is_empty() {
        sort_items(&mut started, Sort::Priority);
        return started;
    }
    top_tier(backlog)
}

/// Whether a ticket can be offered to you: yours (`mine`, lowercased) or,
/// with `unassigned`, nobody's; not a draft; and not under a lease someone
/// else holds (`lock`, the holder of a lease in force; `me`, your email).
fn pickable(
    assignee: &str,
    draft: bool,
    lock: Option<&str>,
    mine: &[String],
    me: &str,
    unassigned: bool,
) -> bool {
    let assignee = assignee.trim().to_lowercase();
    let free = (unassigned && assignee.is_empty()) || mine.contains(&assignee);
    let locked_by_other = lock.is_some_and(|who| !who.eq_ignore_ascii_case(me));
    free && !draft && !locked_by_other
}

/// Whether work to pick includes tickets nobody is assigned to (ticket
/// 160): `ui.include-unassigned` in your own config, on unless set off.
pub fn include_unassigned() -> bool {
    !matches!(
        Config::load()
            .get(UNASSIGNED)
            .map(|v| v.trim().to_lowercase())
            .as_deref(),
        Some("off" | "false" | "0" | "no")
    )
}

/// Sets [`include_unassigned`], on this machine only.
pub fn set_include_unassigned(on: bool) -> anyhow::Result<()> {
    let mut config = Config::load();
    config.set(UNASSIGNED, if on { "on" } else { "off" });
    config.save()
}

/// Every item of the highest priority there is any of, oldest first.
fn top_tier(items: Vec<WorkItem>) -> Vec<WorkItem> {
    for priority in ["high", "medium", "low"] {
        let mut tier: Vec<WorkItem> = items
            .iter()
            .filter(|w| w.priority == priority)
            .cloned()
            .collect();
        if !tier.is_empty() {
            sort_items(&mut tier, Sort::Age);
            return tier;
        }
    }
    Vec::new()
}

/// Shows again every hidden project `items` come from (ticket 160): work
/// to pick is in it, so it's back on the landing page and in My work.
/// Returns the names of the projects shown again.
pub fn show_projects_of(items: &[WorkItem]) -> anyhow::Result<Vec<String>> {
    let hidden = hidden();
    let mut shown = Vec::new();
    for item in items {
        if hidden.contains(&item.project_root) && !shown.contains(&item.project) {
            set_hidden(&item.project_root, false)?;
            shown.push(item.project.clone());
        }
    }
    Ok(shown)
}

/// Calls `f` once for every board of every project on this machine (hidden
/// ones only with `include_hidden`) with the names "you" go by there. "You"
/// is git's identity in each board's own repository (people use a different
/// email per organisation, as leases already allow), else `email`/`name`.
/// A repository two projects declare is visited once.
fn each_board(
    include_hidden: bool,
    email: &str,
    name: Option<&str>,
    mut f: impl FnMut(&Project, &str, &Board, &[String]),
) {
    let registry = Registry::load();
    let projects = projects_in(&registry, &hidden());
    let mut seen: Vec<PathBuf> = Vec::new();
    for project in projects
        .iter()
        .filter(|p| !p.archived && (include_hidden || !p.hidden))
    {
        for (repo, board) in boards_of(project, &registry) {
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
            f(project, &repo, &board, &mine);
        }
    }
}

fn item(project: &Project, repo: &str, task: &crate::task::Task) -> WorkItem {
    WorkItem {
        project: project.name.clone(),
        project_root: project.root.clone(),
        repo: repo.to_string(),
        column: task.column.to_string(),
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
        folder: task.epic.clone(),
    }
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
            folder: None,
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
    fn only_the_highest_priority_there_is_is_offered() {
        let all = vec![
            item("low", "2026-01-01", "", "001"),
            item("medium", "2026-03-01", "", "002"),
            item("medium", "2026-02-01", "", "003"),
        ];
        assert_eq!(
            ids(&top_tier(all.clone())),
            ["003", "002"],
            "no high: the mediums, oldest first"
        );
        let mut with_high = all.clone();
        with_high.push(item("high", "", "", "004"));
        assert_eq!(ids(&top_tier(with_high)), ["004"]);
        assert_eq!(ids(&top_tier(vec![item("low", "", "", "005")])), ["005"]);
        assert!(top_tier(Vec::new()).is_empty());
    }

    #[test]
    fn unassigned_or_mine_and_not_a_draft_or_locked_by_someone_else() {
        let mine = vec!["sam lee".to_string(), "sam@x.co".to_string()];
        let me = "sam@x.co";
        assert!(pickable("", false, None, &mine, me, true));
        assert!(pickable("Sam Lee", false, None, &mine, me, false));
        assert!(
            !pickable("", false, None, &mine, me, false),
            "unassigned left out"
        );
        assert!(
            !pickable("Jo", false, None, &mine, me, true),
            "someone else's"
        );
        assert!(
            !pickable("", true, None, &mine, me, true),
            "a draft can't leave the backlog"
        );
        assert!(
            !pickable("", false, Some("jo@x.co"), &mine, me, true),
            "locked by Jo"
        );
        assert!(
            pickable("", false, Some("SAM@x.co"), &mine, me, true),
            "my own lock"
        );
    }

    #[test]
    fn scratch_boards_are_skipped() {
        let temp = Path::new("/tmp");
        assert!(is_scratch(Path::new("/tmp/tmp.abc/s"), temp));
        assert!(!is_scratch(Path::new("/home/u/work/p-tickets"), temp));
    }
}
