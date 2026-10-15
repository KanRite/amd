//! "Done waits" on the page (ticket 213): which tickets a card must wait
//! for before it can go to done. The same rule as `agile_md::links::
//! waiting_on`, which `Board::move_task` enforces; this is only so a card
//! can say so and a drag onto done is refused before the drop. A ticket
//! waits for those it requires (its own `requires:` link, or another's
//! `required-by:` naming it) and for its children (their `parent`), unless
//! they are done. A ticket that isn't on the board holds nothing back.

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
    let links_to = |entries: &[String], kind: &str, id: Option<u32>| {
        id.is_some()
            && entries
                .iter()
                .filter_map(|e| split_link(e))
                .any(|(k, i)| k == kind && number(i) == id)
    };
    let mut out: Vec<&TicketSummary> = all
        .into_iter()
        .filter(|other| {
            other.repo == ticket.repo
                && other.file != ticket.file
                && other.column != Column::Done
                && (links_to(&ticket.links, "requires", number(&other.id))
                    || links_to(&other.links, "required-by", me)
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
}
