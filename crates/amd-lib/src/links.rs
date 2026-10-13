//! Typed links between tickets (ticket 207): a ticket says how it stands to
//! another — it blocks it, depends on it, supersedes it — and the other
//! ticket says the opposite, so either end shows the link.
//!
//! Where a link is recorded depends on what it is. Four kinds already had a
//! home and keep it, so there is one record of each fact and `amd board`,
//! `amd duplicate` and the editor go on reading the same keys:
//!
//! - `RelatesTo` is `related` on both tickets,
//! - `ChildOf` is the child's `parent`, with `ParentOf` the parent's
//!   `## Children` wikilink,
//! - `CloneOf` is the copy's `clone-of`, with `ClonedBy` the original's
//!   `clones`.
//!
//! The rest are new and live in a `links` flow list, one `kind:id` entry
//! each (`links: [blocks:012,depends-on:003]`). Ids rather than paths, like
//! `parent` and `related`, so a link survives a rename or a move between
//! columns.
//!
//! Only tickets on the same board can be linked: a bare id can't name a
//! ticket on another, the same limit `amd duplicate` has.

use anyhow::{Result, bail};

use crate::task::Task;

/// How one ticket stands to another, read from the ticket that records it
/// (`Blocks` on ticket 12 means 12 blocks the other ticket).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LinkType {
    DependsOn,
    DependedOnBy,
    ParentOf,
    ChildOf,
    RelatesTo,
    CloneOf,
    ClonedBy,
    BlockedBy,
    Blocks,
    Supersedes,
    SupersededBy,
}

impl LinkType {
    /// Every kind, in the order a picker lists them.
    pub const ALL: [LinkType; 11] = [
        LinkType::RelatesTo,
        LinkType::Blocks,
        LinkType::BlockedBy,
        LinkType::DependsOn,
        LinkType::DependedOnBy,
        LinkType::ParentOf,
        LinkType::ChildOf,
        LinkType::CloneOf,
        LinkType::ClonedBy,
        LinkType::Supersedes,
        LinkType::SupersededBy,
    ];

    /// The kebab-case name used on the command line and in `links`.
    pub fn slug(self) -> &'static str {
        match self {
            LinkType::DependsOn => "depends-on",
            LinkType::DependedOnBy => "depended-on-by",
            LinkType::ParentOf => "parent-of",
            LinkType::ChildOf => "child-of",
            LinkType::RelatesTo => "relates-to",
            LinkType::CloneOf => "clone-of",
            LinkType::ClonedBy => "cloned-by",
            LinkType::BlockedBy => "blocked-by",
            LinkType::Blocks => "blocks",
            LinkType::Supersedes => "supersedes",
            LinkType::SupersededBy => "superseded-by",
        }
    }

    /// The kind as the other end records it.
    pub fn inverse(self) -> LinkType {
        match self {
            LinkType::DependsOn => LinkType::DependedOnBy,
            LinkType::DependedOnBy => LinkType::DependsOn,
            LinkType::ParentOf => LinkType::ChildOf,
            LinkType::ChildOf => LinkType::ParentOf,
            LinkType::RelatesTo => LinkType::RelatesTo,
            LinkType::CloneOf => LinkType::ClonedBy,
            LinkType::ClonedBy => LinkType::CloneOf,
            LinkType::BlockedBy => LinkType::Blocks,
            LinkType::Blocks => LinkType::BlockedBy,
            LinkType::Supersedes => LinkType::SupersededBy,
            LinkType::SupersededBy => LinkType::Supersedes,
        }
    }

    /// A phrase that reads after a ticket's number: `012 blocks 013`.
    pub fn phrase(self) -> &'static str {
        match self {
            LinkType::DependsOn => "depends on",
            LinkType::DependedOnBy => "is depended on by",
            LinkType::ParentOf => "is the parent of",
            LinkType::ChildOf => "is a child of",
            LinkType::RelatesTo => "relates to",
            LinkType::CloneOf => "is a clone of",
            LinkType::ClonedBy => "is cloned by",
            LinkType::BlockedBy => "is blocked by",
            LinkType::Blocks => "blocks",
            LinkType::Supersedes => "supersedes",
            LinkType::SupersededBy => "is superseded by",
        }
    }

    /// The name a person sees in a picker: `Blocked by`.
    pub fn label(self) -> &'static str {
        match self {
            LinkType::DependsOn => "Depends on",
            LinkType::DependedOnBy => "Depended on by",
            LinkType::ParentOf => "Parent of",
            LinkType::ChildOf => "Child of",
            LinkType::RelatesTo => "Relates to",
            LinkType::CloneOf => "Clone of",
            LinkType::ClonedBy => "Cloned by",
            LinkType::BlockedBy => "Blocked by",
            LinkType::Blocks => "Blocks",
            LinkType::Supersedes => "Supersedes",
            LinkType::SupersededBy => "Superseded by",
        }
    }

    /// A kind from its slug, ignoring case and treating `_` and spaces as
    /// `-`, so `BlockedBy`-style spellings written as `blocked_by` still work.
    pub fn parse(text: &str) -> Option<LinkType> {
        let wanted: String = text
            .trim()
            .chars()
            .map(|c| if c == '_' || c == ' ' { '-' } else { c })
            .collect::<String>()
            .to_lowercase();
        LinkType::ALL.into_iter().find(|kind| kind.slug() == wanted)
    }
}

impl std::fmt::Display for LinkType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.slug())
    }
}

/// One entry of a ticket's `links`: a kind and the id of the other ticket.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Link {
    pub kind: LinkType,
    pub id: String,
}

impl Link {
    /// The entry as written in `links`: `blocks:012`.
    pub fn entry(&self) -> String {
        format!("{}:{}", self.kind.slug(), self.id)
    }

    /// An entry of `links`; `None` if it isn't one.
    pub fn parse(entry: &str) -> Option<Link> {
        let (kind, id) = entry.split_once(':')?;
        let id = id.trim();
        if id.is_empty() {
            return None;
        }
        Some(Link {
            kind: LinkType::parse(kind)?,
            id: id.to_string(),
        })
    }
}

/// Refuse what would leave a link half made, before anything is written.
/// A ticket has one parent and is the copy of one original; naming a
/// second would silently replace the first.
fn preflight(from: &Task, kind: LinkType, to: &Task, one_way: bool) -> Result<()> {
    let (child, parent) = match kind {
        LinkType::ChildOf => (from, to),
        LinkType::ParentOf => (to, from),
        _ => (from, to),
    };
    let (copy, original) = match kind {
        LinkType::CloneOf => (from, to),
        LinkType::ClonedBy => (to, from),
        _ => (from, to),
    };
    // With `one_way`, a ParentOf records only the parent's list; the child's
    // `parent` isn't touched, so it can't clash.
    let sets_parent = match kind {
        LinkType::ChildOf => true,
        LinkType::ParentOf => !one_way,
        _ => false,
    };
    if sets_parent
        && let Some(existing) = child.parent()
        && existing != parent.id_display()
    {
        bail!(
            "{} already has parent {existing}; remove that first",
            child.id_display()
        );
    }
    let sets_clone_of = match kind {
        LinkType::CloneOf => true,
        LinkType::ClonedBy => !one_way,
        _ => false,
    };
    if sets_clone_of
        && let Some(existing) = copy.clone_of()
        && existing != original.id_display()
    {
        bail!(
            "{} is already a clone of {existing}; remove that first",
            copy.id_display()
        );
    }
    Ok(())
}

/// Record (or, with `remove`, take away) on `task` that it stands to `other`
/// as `kind`. Only `task`'s own file changes. Returns whether it did.
fn write_end(task: &Task, kind: LinkType, other: &Task, remove: bool) -> Result<bool> {
    let expected = task.hash()?;
    let other_id = other.id_display();
    match kind {
        LinkType::RelatesTo if remove => task.remove_related(expected, &other_id),
        LinkType::RelatesTo => task.add_related(expected, std::slice::from_ref(&other_id)),
        LinkType::ChildOf if remove => {
            if task.parent().as_deref() == Some(other_id.as_str()) {
                task.set_parent(expected, "")?;
                Ok(true)
            } else {
                Ok(false)
            }
        }
        LinkType::ChildOf => {
            if task.parent().as_deref() == Some(other_id.as_str()) {
                return Ok(false);
            }
            task.set_parent(expected, &other_id)?;
            Ok(true)
        }
        LinkType::ParentOf if remove => task.remove_child_link(expected, &other.stem),
        LinkType::ParentOf => task.add_child_link(expected, &other.stem),
        LinkType::CloneOf if remove => {
            if task.clone_of().as_deref() == Some(other_id.as_str()) {
                task.set_clone_of(expected, "")?;
                Ok(true)
            } else {
                Ok(false)
            }
        }
        LinkType::CloneOf => {
            if task.clone_of().as_deref() == Some(other_id.as_str()) {
                return Ok(false);
            }
            task.set_clone_of(expected, &other_id)?;
            Ok(true)
        }
        LinkType::ClonedBy if remove => task.remove_clone(expected, &other_id),
        LinkType::ClonedBy => task.add_clone(expected, &other_id),
        _ => {
            let link = Link { kind, id: other_id };
            if remove {
                task.remove_link(expected, &link)
            } else {
                task.add_link(expected, &link)
            }
        }
    }
}

/// Link `from` to `to` as `kind` — or, with `remove`, unlink them — and
/// record the opposite on `to` unless `one_way`. Returns a line for each end
/// that changed (`012 blocks 013`), or one saying nothing did. Each file is
/// hash-checked on its own; the caller wraps the call in one auto-commit so
/// the two ends are one change in history.
pub fn apply(
    from: &Task,
    kind: LinkType,
    to: &Task,
    remove: bool,
    one_way: bool,
) -> Result<Vec<String>> {
    if from.path == to.path {
        bail!("a ticket can't be linked to itself");
    }
    if !remove {
        preflight(from, kind, to, one_way)?;
    }
    let (from_id, to_id) = (from.id_display(), to.id_display());
    let mut lines = Vec::new();
    let verb = |kind: LinkType, remove: bool| {
        if remove {
            format!("no longer {}", kind.phrase())
        } else {
            kind.phrase().to_string()
        }
    };
    if write_end(from, kind, to, remove)? {
        lines.push(format!("{from_id} {} {to_id}", verb(kind, remove)));
    }
    if !one_way {
        let inverse = kind.inverse();
        if write_end(to, inverse, from, remove)? {
            lines.push(format!("{to_id} {} {from_id}", verb(inverse, remove)));
        }
    }
    if lines.is_empty() {
        lines.push(if remove {
            format!("{from_id} wasn't linked to {to_id} as {kind}")
        } else {
            format!("{from_id} already {} {to_id}", kind.phrase())
        });
    }
    Ok(lines)
}

/// Every link a ticket has, whichever key records it: `links`, plus
/// `related`, `parent`, `clone-of` and `clones`. A parent's children aren't
/// here: they are found by scanning the board, and the page already does.
pub fn all(task: &Task) -> Vec<Link> {
    let mut out = task.links();
    out.extend(task.related().into_iter().map(|id| Link {
        kind: LinkType::RelatesTo,
        id,
    }));
    out.extend(task.parent().map(|id| Link {
        kind: LinkType::ChildOf,
        id,
    }));
    out.extend(task.clone_of().map(|id| Link {
        kind: LinkType::CloneOf,
        id,
    }));
    out.extend(task.clones().into_iter().map(|id| Link {
        kind: LinkType::ClonedBy,
        id,
    }));
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board;
    use crate::create::{Draft, NewTicket};
    use crate::templates::Templates;

    fn board_with(titles: &[&str]) -> (tempfile::TempDir, Board) {
        let dir = tempfile::tempdir().unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@t.co"],
            vec!["config", "user.name", "t"],
        ] {
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .status()
                .unwrap();
        }
        let board = Board {
            root: dir.path().join("tasks"),
        };
        board.create().unwrap();
        let templates = Templates::load(&board).unwrap();
        for title in titles {
            let ticket = NewTicket {
                title: (*title).to_string(),
                template: crate::templates::DEFAULT_TEMPLATE.to_string(),
                branch_type: "feature".to_string(),
                ..NewTicket::default()
            };
            Draft::prepare(&board, &templates, ticket)
                .unwrap()
                .write(&board, None)
                .unwrap();
        }
        (dir, board)
    }

    fn find(board: &Board, id: &str) -> Task {
        board.find(id).unwrap()
    }

    #[test]
    fn every_kind_round_trips_through_its_slug_and_has_an_inverse_that_points_back() {
        for kind in LinkType::ALL {
            assert_eq!(LinkType::parse(kind.slug()), Some(kind));
            assert_eq!(kind.inverse().inverse(), kind);
        }
        assert_eq!(LinkType::parse("Blocked_By"), Some(LinkType::BlockedBy));
        assert_eq!(LinkType::parse("nope"), None);
        assert_eq!(LinkType::ALL.len(), 11);
    }

    #[test]
    fn an_entry_reads_back_as_written_and_a_bad_one_is_skipped() {
        let link = Link {
            kind: LinkType::Blocks,
            id: "012".into(),
        };
        assert_eq!(link.entry(), "blocks:012");
        assert_eq!(Link::parse("blocks:012"), Some(link));
        assert_eq!(Link::parse("blocks:"), None);
        assert_eq!(Link::parse("flies-to:012"), None);
        assert_eq!(Link::parse("012"), None);
    }

    #[test]
    fn a_blocking_link_is_written_on_both_tickets() {
        let (_dir, board) = board_with(&["First", "Second"]);
        let lines = apply(
            &find(&board, "001"),
            LinkType::Blocks,
            &find(&board, "002"),
            false,
            false,
        )
        .unwrap();
        assert_eq!(lines, ["001 blocks 002", "002 is blocked by 001"]);
        assert_eq!(
            find(&board, "001").links(),
            [Link {
                kind: LinkType::Blocks,
                id: "002".into()
            }]
        );
        assert_eq!(
            find(&board, "002").links(),
            [Link {
                kind: LinkType::BlockedBy,
                id: "001".into()
            }]
        );
    }

    #[test]
    fn linking_again_changes_nothing_and_says_so() {
        let (_dir, board) = board_with(&["First", "Second"]);
        let (a, b) = (find(&board, "001"), find(&board, "002"));
        apply(&a, LinkType::DependsOn, &b, false, false).unwrap();
        let lines = apply(&a, LinkType::DependsOn, &b, false, false).unwrap();
        assert_eq!(lines, ["001 already depends on 002"]);
        assert_eq!(a.links().len(), 1);
    }

    #[test]
    fn one_ticket_can_be_linked_to_several_others_in_several_ways() {
        let (_dir, board) = board_with(&["A", "B", "C"]);
        let a = find(&board, "001");
        apply(&a, LinkType::Blocks, &find(&board, "002"), false, false).unwrap();
        apply(&a, LinkType::Blocks, &find(&board, "003"), false, false).unwrap();
        apply(&a, LinkType::Supersedes, &find(&board, "003"), false, false).unwrap();
        let links = find(&board, "001").links();
        assert_eq!(links.len(), 3, "{links:?}");
        assert_eq!(find(&board, "003").links().len(), 2);
    }

    #[test]
    fn the_older_fields_keep_their_kinds() {
        let (_dir, board) = board_with(&["Parent", "Child", "Other", "Copy"]);
        let t = |id: &str| find(&board, id);
        apply(&t("002"), LinkType::ChildOf, &t("001"), false, false).unwrap();
        assert_eq!(t("002").parent().as_deref(), Some("001"));
        let parent = std::fs::read_to_string(&t("001").path).unwrap();
        assert!(parent.contains("- [[002-child]]"), "{parent}");

        apply(&t("001"), LinkType::RelatesTo, &t("003"), false, false).unwrap();
        assert_eq!(t("001").related(), ["003"]);
        assert_eq!(t("003").related(), ["001"]);

        apply(&t("004"), LinkType::CloneOf, &t("003"), false, false).unwrap();
        assert_eq!(t("004").clone_of().as_deref(), Some("003"));
        assert_eq!(t("003").clones(), ["004"]);
        // Nothing went into the new key.
        assert!(t("004").links().is_empty());
        assert_eq!(all(&t("004")).len(), 1);
    }

    #[test]
    fn parent_of_is_the_same_link_seen_from_the_parent() {
        let (_dir, board) = board_with(&["Parent", "Child"]);
        apply(
            &find(&board, "001"),
            LinkType::ParentOf,
            &find(&board, "002"),
            false,
            false,
        )
        .unwrap();
        assert_eq!(find(&board, "002").parent().as_deref(), Some("001"));
        let parent = std::fs::read_to_string(&find(&board, "001").path).unwrap();
        assert!(parent.contains("- [[002-child]]"), "{parent}");
    }

    #[test]
    fn a_second_parent_is_refused_before_anything_is_written() {
        let (_dir, board) = board_with(&["One", "Two", "Child"]);
        let t = |id: &str| find(&board, id);
        apply(&t("003"), LinkType::ChildOf, &t("001"), false, false).unwrap();
        let before = std::fs::read_to_string(&t("002").path).unwrap();
        let err = apply(&t("002"), LinkType::ParentOf, &t("003"), false, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("already has parent 001"), "{err}");
        assert_eq!(std::fs::read_to_string(&t("002").path).unwrap(), before);
        assert_eq!(t("003").parent().as_deref(), Some("001"));
    }

    #[test]
    fn a_ticket_cannot_be_the_copy_of_two_originals() {
        let (_dir, board) = board_with(&["One", "Two", "Copy"]);
        let t = |id: &str| find(&board, id);
        apply(&t("003"), LinkType::CloneOf, &t("001"), false, false).unwrap();
        let err = apply(&t("003"), LinkType::CloneOf, &t("002"), false, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("already a clone of 001"), "{err}");
    }

    #[test]
    fn removing_clears_both_ends_whatever_key_holds_the_link() {
        let (_dir, board) = board_with(&["A", "B"]);
        let t = |id: &str| find(&board, id);
        for kind in [
            LinkType::Blocks,
            LinkType::RelatesTo,
            LinkType::ChildOf,
            LinkType::CloneOf,
        ] {
            apply(&t("001"), kind, &t("002"), false, false).unwrap();
            let lines = apply(&t("001"), kind, &t("002"), true, false).unwrap();
            assert_eq!(lines.len(), 2, "{kind}: {lines:?}");
            assert!(all(&t("001")).is_empty(), "{kind}");
            assert!(all(&t("002")).is_empty(), "{kind}");
        }
        let a = std::fs::read_to_string(&t("001").path).unwrap();
        assert!(!a.contains("[["), "{a}");
    }

    #[test]
    fn removing_a_link_that_is_not_there_says_so() {
        let (_dir, board) = board_with(&["A", "B"]);
        let lines = apply(
            &find(&board, "001"),
            LinkType::Blocks,
            &find(&board, "002"),
            true,
            false,
        )
        .unwrap();
        assert_eq!(lines, ["001 wasn't linked to 002 as blocks"]);
    }

    #[test]
    fn one_way_leaves_the_other_ticket_alone() {
        let (_dir, board) = board_with(&["A", "B"]);
        apply(
            &find(&board, "001"),
            LinkType::Supersedes,
            &find(&board, "002"),
            false,
            true,
        )
        .unwrap();
        assert_eq!(find(&board, "001").links().len(), 1);
        assert!(find(&board, "002").links().is_empty());
    }

    #[test]
    fn a_ticket_cannot_link_to_itself() {
        let (_dir, board) = board_with(&["A"]);
        let a = find(&board, "001");
        assert!(apply(&a, LinkType::Blocks, &a, false, false).is_err());
    }
}
