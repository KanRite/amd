//! "Done waits" on the page (ticket 213): which tickets a card must wait
//! for before it can go to done. The same rule as `agile_md::links::
//! waiting_on`, which `Board::move_task` enforces; this is only so a card
//! can say so and a drag onto done is refused before the drop. A ticket
//! waits for those it requires (its own `requires:` link, or another's
//! `required-by:` naming it) and for its children (their `parent`), unless
//! they are done. A ticket that isn't on the board holds nothing back.
//! The same reading of "requires" gives `sprint_gaps` (ticket 214): what
//! stops a sprint starting.

use crate::{Board, Column, TicketSummary, split_link};

fn number(id: &str) -> Option<u32> {
    id.trim().trim_start_matches('#').parse().ok()
}

/// Every ticket on the board, wherever it sits: the columns and the epics.
pub fn all_tickets(board: &Board) -> Vec<&TicketSummary> {
    board
        .columns
        .iter()
        .flat_map(|c| c.tickets.iter())
        .chain(board.epics.iter().flat_map(|e| e.tickets.iter()))
        .collect()
}

/// The tickets in `all` that `ticket` waits on: on its own board, not done,
/// and required by it or its children. Sorted by number.
pub fn waiting_on<'a>(
    ticket: &TicketSummary,
    all: impl IntoIterator<Item = &'a TicketSummary>,
) -> Vec<&'a TicketSummary> {
    let me = number(&ticket.id);
    let all: Vec<&TicketSummary> = all.into_iter().collect();
    let required = requires(ticket, all.iter().copied());
    let mut out: Vec<&TicketSummary> = all
        .into_iter()
        .filter(|other| {
            other.repo == ticket.repo
                && other.file != ticket.file
                && other.column != Column::Done
                && (required.iter().any(|r| r.file == other.file)
                    || (me.is_some() && number(&other.parent) == me))
        })
        .collect();
    out.sort_by_key(|t| number(&t.id));
    out.dedup_by(|a, b| a.file == b.file);
    out
}

/// Whether a ticket may be dropped in `to`: refused into done while it
/// waits on others.
pub fn done_rule(to: Column, waiting: &[&TicketSummary], ticket: &str) -> Result<(), String> {
    if to != Column::Done || waiting.is_empty() {
        return Ok(());
    }
    let ids: Vec<&str> = waiting.iter().map(|t| t.id.as_str()).collect();
    let them = if ids.len() == 1 { "it" } else { "they" };
    Err(format!(
        "{ticket} waits on {}: {them} must be done first, or the link changed or removed.",
        ids.join(", ")
    ))
}

/// The tickets `ticket` requires, of those in `all` (on its own board):
/// its `requires:` links, and others' `required-by:` naming it. Done or not.
pub fn requires<'a>(
    ticket: &TicketSummary,
    all: impl IntoIterator<Item = &'a TicketSummary>,
) -> Vec<&'a TicketSummary> {
    let me = number(&ticket.id);
    let names = |entries: &[String], kind: &str, id: Option<u32>| {
        id.is_some()
            && entries
                .iter()
                .filter_map(|e| split_link(e))
                .any(|(k, i)| k == kind && number(i) == id)
    };
    all.into_iter()
        .filter(|other| {
            other.repo == ticket.repo
                && other.file != ticket.file
                && (names(&ticket.links, "requires", number(&other.id))
                    || names(&other.links, "required-by", me))
        })
        .collect()
}

/// Whether a card belongs to the epic or sprint `name`: its backlog
/// folder, or its `epic` label once it has moved on.
fn in_group(ticket: &TicketSummary, name: &str) -> bool {
    match ticket.column {
        Column::Backlog => ticket.epic.as_deref() == Some(name),
        _ => ticket.epic_label == name,
    }
}

/// Where a card is, for a message: `Backlog`, `Backlog › s13`, `To do`.
pub fn place(ticket: &TicketSummary) -> String {
    match (&ticket.epic, ticket.column) {
        (Some(group), Column::Backlog) => format!("Backlog › {group}"),
        (_, column) => column.title().to_string(),
    }
}

/// A sprint ticket that requires one outside the sprint, not done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gap {
    pub id: String,
    pub file: String,
    pub required_id: String,
    pub required_place: String,
}

/// What stops sprint `name` in `repo` starting (ticket 214): the same rule
/// as `agile_md::requires::sprint_gaps`, from the board's cards (`all`, as
/// [`all_tickets`] gives them).
pub fn sprint_gaps(all: &[&TicketSummary], repo: &str, name: &str) -> Vec<Gap> {
    let mut gaps = Vec::new();
    for ticket in all.iter().filter(|t| t.repo == repo && in_group(t, name)) {
        for needed in requires(ticket, all.iter().copied()) {
            if needed.column == Column::Done || in_group(needed, name) {
                continue;
            }
            gaps.push(Gap {
                id: ticket.id.clone(),
                file: ticket.file.clone(),
                required_id: needed.id.clone(),
                required_place: place(needed),
            });
        }
    }
    gaps.sort_by(|a, b| (&a.id, &a.required_id).cmp(&(&b.id, &b.required_id)));
    gaps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str, column: Column) -> TicketSummary {
        TicketSummary {
            repo: "acme-tickets".into(),
            file: format!("{id}-t.md"),
            id: id.into(),
            column,
            ..TicketSummary::default()
        }
    }

    #[test]
    fn a_ticket_waits_for_what_it_requires_and_its_children_until_they_are_done() {
        let mut parent = card("001", Column::Doing);
        parent.links = vec!["requires:002".into()];
        let required = card("002", Column::Todo);
        let mut child = card("003", Column::Backlog);
        child.parent = "001".into();
        let mut finished = card("004", Column::Done);
        finished.parent = "001".into();
        let mut requiring = card("005", Column::Todo);
        requiring.links = vec!["required-by:001".into()];
        let unrelated = card("006", Column::Todo);
        let all = [
            &parent, &required, &child, &finished, &requiring, &unrelated,
        ];
        let waits: Vec<&str> = waiting_on(&parent, all)
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(waits, ["002", "003", "005"]);
        let w = waiting_on(&parent, all);
        assert!(done_rule(Column::Done, &w, "001").is_err());
        assert!(done_rule(Column::Doing, &w, "001").is_ok());
        assert!(done_rule(Column::Done, &[], "001").is_ok());
        // Another board's 002 is a different ticket.
        let mut elsewhere = card("002", Column::Todo);
        elsewhere.repo = "acme-web".into();
        let mut lone = card("001", Column::Doing);
        lone.links = vec!["requires:002".into()];
        assert!(waiting_on(&lone, [&elsewhere]).is_empty());
    }

    #[test]
    fn a_sprint_cannot_start_while_its_tickets_require_ones_outside_it() {
        let in_sprint = |id: &str| {
            let mut t = card(id, Column::Backlog);
            t.epic = Some("s12".into());
            t
        };
        let mut a = in_sprint("001");
        a.links = vec!["requires:002".into(), "requires:003".into()];
        let b = in_sprint("002"); // in the sprint: fine
        let loose = card("003", Column::Backlog); // outside: a gap
        let mut c = in_sprint("004");
        c.links = vec!["requires:005".into()];
        let finished = card("005", Column::Done); // done: fine
        let mut started_on = card("006", Column::Todo);
        started_on.epic_label = "s12".into(); // moved on, still the sprint's
        let mut needs = card("007", Column::Backlog);
        needs.links = vec!["required-by:006".into()]; // recorded on the other end
        let all = [&a, &b, &loose, &c, &finished, &started_on, &needs];
        let gaps: Vec<(String, String, String)> = sprint_gaps(&all, "acme-tickets", "s12")
            .into_iter()
            .map(|g| (g.id, g.required_id, g.required_place))
            .collect();
        assert_eq!(
            gaps,
            [
                ("001".into(), "003".into(), "Backlog".into()),
                ("006".into(), "007".into(), "Backlog".into())
            ]
        );
        assert!(sprint_gaps(&all, "acme-tickets", "s13").is_empty());
    }
}
