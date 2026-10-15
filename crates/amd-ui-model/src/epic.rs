//! Epics: named containers hanging off the backlog.
//!
//! The layout is `amd`'s, so `amd group list` and the board see the same
//! epics: an epic is a directory `tasks/backlog/<name>/` described by a
//! `_group.md` inside it, and each ticket filed under it lives in that
//! directory with `epic: "<name>"` in its frontmatter.

use crate::{Column, Kind, Workflow};

/// Where a ticket sits: which repository's board, a column, and for the
/// backlog optionally an epic. An empty `repo` means the board's own
/// (primary) repository.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Place {
    #[serde(default)]
    pub repo: String,
    pub column: Column,
    pub epic: Option<String>,
}

impl Place {
    pub fn column(column: Column) -> Place {
        Place {
            repo: String::new(),
            column,
            epic: None,
        }
    }

    pub fn epic(name: &str) -> Place {
        Place {
            repo: String::new(),
            column: Column::Backlog,
            epic: Some(name.to_string()),
        }
    }

    /// The same place on another repository's board.
    pub fn in_repo(mut self, repo: &str) -> Place {
        self.repo = repo.to_string();
        self
    }
}

/// Whether a ticket may be dragged from `from` to `to`.
///
/// The backlog is the default container, and each epic is another container
/// at the same level, so every one of them feeds todo:
///
/// - between columns, one step at a time;
/// - from the loose backlog into an epic, and from an epic back out to it;
/// - from an epic straight on to todo, and from todo back into an epic;
/// - never from one repository's board to another's (as in `amd`).
///
/// An epic's ticket cannot skip ahead to doing or done, nor go straight into
/// another epic.
///
/// "Next to" is the board's own workflow (ticket 224): a team's columns,
/// in its order.
pub fn can_move(workflow: &Workflow, from: &Place, to: &Place) -> bool {
    if from.repo != to.repo {
        return false;
    }
    // Epics and sprints live in the standard backlog: the loose backlog
    // beside them, and a todo-kind column next to the backlog, are at
    // their level.
    let backlog_level = |c: Column| {
        c == Column::Backlog || (c.is(Kind::Todo) && workflow.next_to(Column::Backlog, c))
    };
    match (&from.epic, &to.epic) {
        (None, None) => workflow.next_to(from.column, to.column),
        (None, Some(_)) => backlog_level(from.column),
        (Some(_), None) => backlog_level(to.column),
        (Some(_), Some(_)) => false,
    }
}

/// The two kinds of container `amd` keeps in the backlog: an epic is a body
/// of work, a sprint a slice of time.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum GroupKind {
    #[default]
    Epic,
    Sprint,
}

/// A sprint's length when nobody says otherwise (`amd`'s default).
pub const DEFAULT_SPRINT_DAYS: u32 = 14;

/// What the sprint rules need to know about a container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupState {
    pub kind: GroupKind,
    pub started: bool,
}

impl GroupState {
    fn started_sprint(self) -> bool {
        self.kind == GroupKind::Sprint && self.started
    }
}

/// The sprint rules, on top of [`can_move`]:
///
/// - a ticket needs points before it can go into a sprint (as `amd` says,
///   an unsized ticket makes a sprint's total a lie);
/// - a started sprint is a commitment: no ticket can be added to it, and
///   none can go back from it to the backlog — moving on to todo is fine.
///
/// `from_group` and `to_group` describe the containers named by the places'
/// `epic`, if any; `ticket` names the ticket in messages.
pub fn sprint_rule(
    from: &Place,
    to: &Place,
    from_group: Option<GroupState>,
    to_group: Option<GroupState>,
    has_points: bool,
    ticket: &str,
) -> Result<(), String> {
    if let (Some(group), Some(name)) = (to_group, &to.epic)
        && group.kind == GroupKind::Sprint
    {
        if group.started {
            return Err(format!(
                "{name} has started: no more tickets can be added to it."
            ));
        }
        if !has_points {
            return Err(format!("{ticket} needs points before it can go in {name}."));
        }
    }
    if let (Some(group), Some(name)) = (from_group, &from.epic)
        && group.started_sprint()
        && !to.column.is(Kind::Todo)
    {
        return Err(format!(
            "{name} has started: its tickets can only move on to todo."
        ));
    }
    Ok(())
}

/// The tag that makes a ticket a draft (ticket 114), as `amd` writes it.
pub const DRAFT_TAG: &str = "DRAFT";

/// Whether a ticket with these tags is a draft. Case is ignored, as
/// everywhere tags are matched.
pub fn is_draft(tags: &[String]) -> bool {
    tags.iter()
        .any(|tag| tag.trim().eq_ignore_ascii_case(DRAFT_TAG))
}

/// The draft rule (ticket 114), mirroring `amd`'s: a draft can't leave the
/// backlog, and can't go into a sprint. Filing it into an epic, or moving
/// it between the loose backlog and an epic, is fine.
pub fn draft_rule(
    from: &Place,
    to: &Place,
    to_group: Option<GroupState>,
    draft: bool,
    ticket: &str,
) -> Result<(), String> {
    if !draft {
        return Ok(());
    }
    if from.column.is(Kind::Backlog) && !to.column.is(Kind::Backlog) {
        return Err(format!(
            "{ticket} is a draft: remove {DRAFT_TAG} to take it out of the backlog."
        ));
    }
    if let (Some(group), Some(name)) = (to_group, &to.epic)
        && group.kind == GroupKind::Sprint
    {
        return Err(format!(
            "{ticket} is a draft: remove {DRAFT_TAG} before it can go in {name}."
        ));
    }
    Ok(())
}

/// Whether a ticket's `points:` value is a usable size (a number, 0 or more).
pub fn has_points(points: &str) -> bool {
    points.trim().parse::<f64>().is_ok_and(|p| p >= 0.0)
}

/// Picks a colour (an index into a palette of `palette` colours) for each
/// epic, in the order they are shown. Each epic's colour comes from its name,
/// so it stays put as other epics come and go — except that an epic never
/// shares a colour with the one directly above it: on a clash it takes the
/// next colour along. Neighbouring epics therefore always differ.
pub fn colours(names: &[String], palette: usize) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::with_capacity(names.len());
    for name in names {
        let mut colour = (fnv1a(&name.to_lowercase()) % palette.max(1) as u64) as usize;
        if palette > 1 && out.last() == Some(&colour) {
            colour = (colour + 1) % palette;
        }
        out.push(colour);
    }
    out
}

/// FNV-1a: a fixed hash, so a name maps to the same colour on every run and
/// every Rust version (std's hasher makes no such promise).
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Where a ticket is, said in a line (ticket 163): `acme-api › Backlog ›
/// Sprint s12 (started)`, or `Doing · from epic checkout` for one that has
/// moved on from its group. `repo` is named only when there's more than one
/// board in view; `group` is the epic or sprint (filed in its folder when
/// `filed`, else only labelled), with its kind and whether it's started.
pub fn where_it_is(
    repo: Option<&str>,
    column: &str,
    group: Option<(&str, GroupKind, bool)>,
    filed: bool,
) -> String {
    let mut out = String::new();
    if let Some(repo) = repo.filter(|r| !r.is_empty()) {
        out.push_str(repo);
        out.push_str(" › ");
    }
    out.push_str(column);
    if let Some((name, kind, started)) = group {
        let kind = match kind {
            GroupKind::Epic => "epic",
            GroupKind::Sprint => "sprint",
        };
        let started = if started { " (started)" } else { "" };
        if filed {
            let mut kind = kind.to_string();
            kind[..1].make_ascii_uppercase();
            out.push_str(&format!(" › {kind} {name}{started}"));
        } else {
            out.push_str(&format!(" · from {kind} {name}{started}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_draft_stays_in_the_backlog_and_out_of_sprints() {
        let place = |column, epic: Option<&str>| Place {
            repo: String::new(),
            column,
            epic: epic.map(str::to_string),
        };
        let loose = place(Column::Backlog, None);
        let todo = place(Column::Todo, None);
        let in_epic = place(Column::Backlog, Some("e"));
        let in_sprint = place(Column::Backlog, Some("s"));
        let epic = Some(GroupState {
            kind: GroupKind::Epic,
            started: false,
        });
        let sprint = Some(GroupState {
            kind: GroupKind::Sprint,
            started: false,
        });

        assert!(draft_rule(&loose, &todo, None, true, "001").is_err());
        assert!(draft_rule(&in_epic, &todo, None, true, "001").is_err());
        assert!(draft_rule(&loose, &in_sprint, sprint, true, "001").is_err());
        assert!(draft_rule(&loose, &in_epic, epic, true, "001").is_ok());
        assert!(draft_rule(&in_epic, &loose, None, true, "001").is_ok());
        assert!(draft_rule(&loose, &todo, None, false, "001").is_ok());
        assert!(is_draft(&names(&["cli", "draft"])));
        assert!(!is_draft(&names(&["cli"])));
    }

    #[test]
    fn neighbouring_epics_never_share_a_colour() {
        // Many epics, including ones whose names hash alike.
        let many: Vec<String> = (0..200).map(|i| format!("Epic {i}")).collect();
        for palette in [2, 3, 6] {
            let c = colours(&many, palette);
            assert!(
                c.windows(2).all(|w| w[0] != w[1]),
                "palette {palette}: {c:?}"
            );
            assert!(c.iter().all(|&i| i < palette));
        }
        // Same name twice in a row (case aside) still alternates.
        let c = colours(&names(&["Same", "same", "SAME"]), 6);
        assert!(c.windows(2).all(|w| w[0] != w[1]), "{c:?}");
    }

    #[test]
    fn an_epic_keeps_its_colour_when_others_change() {
        let before = colours(&names(&["UserInterface", "UserPreferences"]), 6);
        let after = colours(&names(&["Backend", "UserInterface", "UserPreferences"]), 6);
        // The first epic's colour depends only on its name.
        assert_eq!(colours(&names(&["UserInterface"]), 6)[0], before[0]);
        assert_eq!(colours(&names(&["Backend"]), 6)[0], after[0]);
    }

    #[test]
    fn a_palette_of_one_is_not_a_panic() {
        assert_eq!(colours(&names(&["A", "B"]), 1), [0, 0]);
        assert_eq!(colours(&names(&["A"]), 0), [0]);
    }

    #[test]
    fn sprints_need_points_and_lock_once_started() {
        let backlog = Place::column(Column::Backlog);
        let todo = Place::column(Column::Todo);
        let sprint = Place::epic("Sprint 1");
        let pending = Some(GroupState {
            kind: GroupKind::Sprint,
            started: false,
        });
        let started = Some(GroupState {
            kind: GroupKind::Sprint,
            started: true,
        });
        let epic = Some(GroupState {
            kind: GroupKind::Epic,
            started: false,
        });

        // Into a pending sprint: only with points.
        assert!(sprint_rule(&backlog, &sprint, None, pending, true, "005").is_ok());
        let err = sprint_rule(&backlog, &sprint, None, pending, false, "005").unwrap_err();
        assert_eq!(err, "005 needs points before it can go in Sprint 1.");
        // Epics need no points.
        assert!(sprint_rule(&backlog, &Place::epic("UI"), None, epic, false, "005").is_ok());

        // A started sprint takes nothing more, even sized tickets, from anywhere.
        let err = sprint_rule(&backlog, &sprint, None, started, true, "005").unwrap_err();
        assert!(
            err.contains("has started: no more tickets can be added"),
            "{err}"
        );
        assert!(sprint_rule(&todo, &sprint, None, started, true, "005").is_err());
        // ...and gives nothing back to the backlog, but its tickets go on to todo.
        let err = sprint_rule(&sprint, &backlog, started, None, true, "005").unwrap_err();
        assert!(err.contains("can only move on to todo"), "{err}");
        assert!(sprint_rule(&sprint, &todo, started, None, true, "005").is_ok());
        // A pending sprint lets tickets back out.
        assert!(sprint_rule(&sprint, &backlog, pending, None, true, "005").is_ok());
    }

    #[test]
    fn points_are_checked() {
        assert!(has_points("3") && has_points(" 0.5 ") && has_points("0"));
        assert!(!has_points("") && !has_points("lots") && !has_points("-1"));
    }

    #[test]
    fn moves_in_and_out_of_epics() {
        let backlog = Place::column(Column::Backlog);
        let todo = Place::column(Column::Todo);
        let e1 = Place::epic("One");
        let e2 = Place::epic("Two");
        assert!(can_move(&Workflow::standard(), &backlog, &e1));
        assert!(can_move(&Workflow::standard(), &e1, &backlog));
        assert!(
            can_move(&Workflow::standard(), &e1, &todo),
            "an epic feeds todo like the backlog does"
        );
        assert!(
            can_move(&Workflow::standard(), &todo, &e1),
            "and todo can send a ticket back into it"
        );
        assert!(
            !can_move(&Workflow::standard(), &e1, &Place::column(Column::Doing)),
            "no skipping ahead"
        );
        assert!(!can_move(
            &Workflow::standard(),
            &e1,
            &Place::column(Column::Done)
        ));
        assert!(!can_move(
            &Workflow::standard(),
            &Place::column(Column::Doing),
            &e1
        ));
        assert!(!can_move(&Workflow::standard(), &e1, &e2));
        assert!(
            can_move(&Workflow::standard(), &backlog, &todo)
                && !can_move(
                    &Workflow::standard(),
                    &backlog,
                    &Place::column(Column::Doing)
                )
        );
    }

    #[test]
    fn never_between_repositories() {
        let here = Place::column(Column::Backlog).in_repo("amd-tickets");
        let there = Place::column(Column::Todo).in_repo("amd-site");
        assert!(!can_move(&Workflow::standard(), &here, &there));
        assert!(!can_move(
            &Workflow::standard(),
            &here,
            &Place::epic("UI").in_repo("amd-site")
        ));
        assert!(can_move(
            &Workflow::standard(),
            &here,
            &Place::column(Column::Todo).in_repo("amd-tickets")
        ));
    }

    #[test]
    fn where_a_ticket_is_reads_as_a_path() {
        assert_eq!(where_it_is(None, "Doing", None, false), "Doing");
        assert_eq!(
            where_it_is(
                Some("acme-api"),
                "Backlog",
                Some(("s12", GroupKind::Sprint, true)),
                true
            ),
            "acme-api › Backlog › Sprint s12 (started)"
        );
        assert_eq!(
            where_it_is(
                Some(""),
                "Done",
                Some(("checkout", GroupKind::Epic, false)),
                false
            ),
            "Done · from epic checkout"
        );
    }

    /// Ticket 224: a team's board with Triage, Review and Released.
    #[test]
    fn drags_follow_a_teams_columns_and_kinds() {
        use crate::{ColumnInfo, Kind};
        let col = |folder: &str, kind| Column::new(folder, kind);
        let (triage, review, released) = (
            col("triage", Kind::Backlog),
            col("review", Kind::Doing),
            col("released", Kind::Done),
        );
        let w = Workflow {
            columns: [
                triage,
                Column::Backlog,
                Column::Todo,
                Column::Doing,
                review,
                Column::Done,
                released,
            ]
            .into_iter()
            .map(|column| ColumnInfo {
                column,
                label: column.dir_name().to_string(),
                wip: None,
            })
            .collect(),
        };
        let at = Place::column;
        assert!(can_move(&w, &at(Column::Doing), &at(review)));
        assert!(can_move(&w, &at(review), &at(Column::Done)));
        assert!(!can_move(&w, &at(Column::Doing), &at(Column::Done)));
        assert!(can_move(&w, &at(triage), &at(Column::Backlog)));
        // Epic tickets reach the loose backlog and todo, not Triage.
        let in_epic = Place::epic("e");
        assert!(can_move(&w, &in_epic, &at(Column::Todo)));
        assert!(!can_move(&w, &in_epic, &at(triage)));
        // A draft may go back to Triage (still the backlog), not on.
        assert!(draft_rule(&at(Column::Backlog), &at(triage), None, true, "001").is_ok());
        assert!(draft_rule(&at(Column::Backlog), &at(Column::Todo), None, true, "001").is_err());
        // A started sprint's tickets only move on to a todo-kind column.
        let started = Some(GroupState {
            kind: GroupKind::Sprint,
            started: true,
        });
        let sprint = Place::epic("s");
        assert!(sprint_rule(&sprint, &at(Column::Todo), started, None, true, "001").is_ok());
        assert!(sprint_rule(&sprint, &at(triage), started, None, true, "001").is_err());
        assert!(crate::waiting::done_rule(released, &[], "001").is_ok());
    }
}
