//! The board's data model, free of any front end: tickets as cards in lanes,
//! the repositories a project spans, and the filters and ordering over them.
//!
//! Lifted out of the desktop board (`gui/`) so the terminal board reads and
//! orders tickets exactly as it does. Nothing here draws or prints; every
//! write still goes through `Board`/`Task`, never a file.

use std::collections::BTreeSet;
use std::path::PathBuf;

use anyhow::Result;

use crate::board::{Board, Column};
use crate::group::{Group, Kind};
use crate::registry::Registry;
use crate::task::Task;

pub struct Card {
    pub task: Task,
    pub title: String,
    pub id: String,
    pub assignee: Option<String>,
    pub points: Option<String>,
    /// Sort key: the hand-set order when there is one, otherwise the id, so a
    /// board nobody has dragged still reads in id order.
    pub rank: f64,
}

pub struct RepoBoard {
    pub name: String,
    pub board: Board,
    pub lanes: Vec<Vec<Card>>,
    /// Backlog tickets grouped by folder. `None` is the loose backlog —
    /// everything not filed under an epic or sprint yet.
    pub groups: Vec<(Option<Group>, Vec<Card>)>,
    pub visible: bool,
    /// Which list the epic/sprint dropdown shows. Per-repo, like `groups`
    /// itself — each registered repository has its own independent set.
    pub selected_kind: Kind,
    /// The chosen group, if any. `None` means no filter: the loose backlog
    /// and every column, unfiltered, exactly as before this feature existed.
    /// Switching `selected_kind` clears this — a name from the other kind's
    /// list doesn't carry over into the new one.
    pub selected_group: Option<String>,
}

/// Whether `card` should be drawn under the current tag filter. An empty
/// filter matches everything, same as no filter at all.
pub fn card_visible(card: &Card, filter: &BTreeSet<String>) -> bool {
    filter.is_empty() || card.task.tags().iter().any(|tag| filter.contains(tag))
}

/// Whether `card`'s task currently belongs to the group named `name`. Inside
/// `backlog/` the folder decides (`task.epic`, set from where the file was
/// found); outside it there's no group directory, so the frontmatter key is
/// the only carrier (`epic_meta()`, a fresh read) — same split
/// `Board::tasks_in_group` already makes.
pub fn card_in_group(card: &Card, name: &str) -> bool {
    if card.task.column == Column::Backlog {
        card.task.epic.as_deref() == Some(name)
    } else {
        card.task.epic_meta().as_deref() == Some(name)
    }
}

/// The rank a card dropped at `index` in a lane takes: the midpoint of its
/// neighbours, so one drop rewrites one file. `lane` is every other card's
/// rank in order — the card being moved must already be left out, so
/// dragging within a lane doesn't measure against its old position.
pub fn rank_between(lane: &[f64], index: usize) -> f64 {
    let index = index.min(lane.len());
    match (
        index.checked_sub(1).and_then(|i| lane.get(i)),
        lane.get(index),
    ) {
        (Some(before), Some(after)) => (before + after) / 2.0,
        (Some(before), None) => before + 1.0,
        (None, Some(after)) => after - 1.0,
        (None, None) => 0.0,
    }
}

/// The backlog, split into the loose tickets and one group per epic folder.
/// The loose group comes first and is always present, so there is somewhere to
/// drop a ticket you want to take *out* of an epic.
pub fn read_groups(board: &Board) -> Result<Vec<(Option<Group>, Vec<Card>)>> {
    let all = board.tasks_in(Column::Backlog)?;
    let mut groups: Vec<(Option<Group>, Vec<Card>)> = vec![(None, Vec::new())];
    for group in board.groups()? {
        groups.push((Some(group), Vec::new()));
    }
    for task in all {
        let slot = groups
            .iter_mut()
            .find(|(group, _)| group.as_ref().map(|g| g.name.as_str()) == task.epic.as_deref());
        if let Some((_, cards)) = slot {
            cards.push(card_from(task));
        }
    }
    for (_, cards) in &mut groups {
        sort_cards(cards);
    }
    Ok(groups)
}

pub fn card_from(task: Task) -> Card {
    let rank = task
        .order()
        .unwrap_or_else(|| task.id.map(f64::from).unwrap_or(f64::MAX));
    Card {
        title: task.title(),
        id: task.id_display(),
        assignee: task.assignee(),
        points: task.points(),
        rank,
        task,
    }
}

pub fn sort_cards(cards: &mut [Card]) {
    cards.sort_by(|a, b| {
        a.rank
            .partial_cmp(&b.rank)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// Put a typed body into a rendered ticket, keeping its frontmatter.
///
/// An empty body leaves the template's own, so clearing the box gets you the
/// default ticket rather than an empty file that `write` would refuse.
pub fn splice_body(rendered: &str, body: &str) -> String {
    if body.trim().is_empty() {
        return rendered.to_string();
    }
    match crate::task::split_body(rendered) {
        Some((head, _)) => format!("{head}{body}"),
        // No frontmatter to keep: the template is not one we understand, so
        // don't pretend — write what it rendered.
        None => rendered.to_string(),
    }
}

pub fn read_lanes(board: &Board) -> Result<Vec<Vec<Card>>> {
    let mut lanes = Vec::new();
    for column in Column::ALL {
        let mut cards: Vec<Card> = board.tasks_in(column)?.into_iter().map(card_from).collect();
        sort_cards(&mut cards);
        lanes.push(cards);
    }
    Ok(lanes)
}

/// Builds a `RepoBoard` from a repository root and the `Board` inside it.
/// The directory's own basename names it, not `Board::name()` — a board's
/// root is `<repo>/tasks`, so the latter would label every repository
/// "tasks".
pub fn open_repo_board(root: PathBuf, board: Board) -> Result<RepoBoard> {
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());
    Ok(RepoBoard {
        name,
        lanes: read_lanes(&board)?,
        groups: read_groups(&board)?,
        board,
        visible: true,
        selected_kind: Kind::Epic,
        selected_group: None,
    })
}

/// Everything the repositories of one project resolve to on this machine.
pub struct Repos {
    /// The current project's own board first, then each declared repository
    /// that is registered here and has a board.
    pub repos: Vec<RepoBoard>,
    /// Declared names with no usable local clone, for a front end to show
    /// greyed out rather than silently omit.
    pub unregistered: Vec<String>,
    /// The git identity (`user.email`) leases are compared against.
    pub identity: Option<String>,
}

/// Open the current project's board and every declared, registered
/// repository beside it.
pub fn load_repos() -> Result<Repos> {
    // A hard error now, not a `.ok()` folded away — `amdui` operates on
    // exactly one project (the one launched from), never a personal
    // cross-project stack the way the old machine-wide registry allowed.
    let current = Board::discover()?;
    let current_root = current.repo_root().to_path_buf();
    let declared = crate::board_settings::load(&current).repos();
    let registry = Registry::load();

    let mut repos = vec![open_repo_board(current_root.clone(), current)?];
    // A name this project declares but that isn't registered locally
    // (or is, but its board has since gone missing) — rendered greyed
    // out in the repo-visibility row, not silently omitted, so a
    // teammate who's added a repository sees it exists before they've
    // cloned it.
    let mut unregistered = Vec::new();
    for name in &declared {
        match registry.find(name) {
            Some(entry) if entry.root == current_root => {} // already added, above
            Some(entry) if entry.has_board() => {
                repos.push(open_repo_board(entry.root.clone(), entry.board())?);
            }
            _ => unregistered.push(name.clone()),
        }
    }
    // `user.email` is ordinarily global, so any registered repo's
    // context resolves the same identity — the first one is as good as
    // any other.
    let identity = crate::git::config(&repos[0].board.root, "user.email");
    Ok(Repos {
        repos,
        unregistered,
        identity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_between_takes_the_midpoint_of_its_neighbours() {
        assert_eq!(rank_between(&[1.0, 3.0], 1), 2.0);
    }

    #[test]
    fn rank_between_runs_off_either_end_by_one() {
        assert_eq!(rank_between(&[1.0, 3.0], 0), 0.0);
        assert_eq!(rank_between(&[1.0, 3.0], 2), 4.0);
        assert_eq!(rank_between(&[1.0, 3.0], 9), 4.0, "past the end clamps");
    }

    #[test]
    fn rank_between_in_an_empty_lane_is_zero() {
        assert_eq!(rank_between(&[], 0), 0.0);
    }

    #[test]
    fn lanes_come_back_in_rank_order_with_the_hand_set_order_winning() {
        let dir = tempfile::tempdir().unwrap();
        let board = Board::at(dir.path().join("tasks"));
        board.create().unwrap();
        let write = |name: &str, extra: &str| {
            let id = &name[..3];
            let body = format!("---\nid: \"{id}\"\ntitle: \"T{id}\"\n{extra}---\n\nbody\n");
            std::fs::write(board.dir(Column::Todo).join(name), body).unwrap();
        };
        write("001-a.md", "order: \"5\"\n");
        write("002-b.md", "");
        let lanes = read_lanes(&board).unwrap();
        let todo: Vec<&str> = lanes[Column::Todo as usize]
            .iter()
            .map(|card| card.id.as_str())
            .collect();
        assert_eq!(todo, ["002", "001"]);
    }
}
