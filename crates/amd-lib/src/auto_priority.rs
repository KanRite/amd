//! Auto priority (ticket 215): a ticket is at least as urgent as the most
//! urgent ticket that requires it, directly or down a chain, so the
//! priorities set on the end goals flow down to the work they wait on.
//!
//! Off unless the board's `auto-priority` setting is on. The result is
//! written into `priority`, so everything that reads a priority sees it,
//! and a raise is recorded beside it: `priority-from` (the ticket it
//! inherits from) and `priority-own` (its own priority before the raise).
//! Switching it off puts raised tickets back.
//!
//! With `priority-override` on (the default), a priority set by hand pins
//! its ticket (`priority-pinned`) and auto leaves it alone; setting it to
//! `auto` unpins it. With it off, a hand-set priority is only the ticket's
//! own, a floor auto can still raise.
//!
//! [`reconcile`] runs inside `transition::with_auto_commit` (and the two
//! ticket-creating paths), so any change carries its priority updates in
//! the same commit. Done tickets neither push nor change.

use std::cell::RefCell;

use anyhow::{Result, bail};

use crate::board::{Board, Column};
use crate::board_settings;
use crate::links::{LinkType, same_id};
use crate::task::{PRIORITIES, Task, meta_in};

const FROM: &str = "priority-from";
const OWN: &str = "priority-own";
const PINNED: &str = "priority-pinned";

thread_local! {
    /// What reconcile changed since a front end last asked.
    static NOTES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// What auto priority changed since this was last called, for a front end
/// to say (`010 raised to high: 012 requires it`).
pub fn take_notes() -> Vec<String> {
    NOTES.with(|n| std::mem::take(&mut *n.borrow_mut()))
}

fn rank(priority: &str) -> usize {
    PRIORITIES.iter().position(|p| *p == priority).unwrap_or(0)
}

/// One ticket as auto priority sees it, read once.
struct Seen {
    task: Task,
    priority: String,
    from: Option<String>,
    own: Option<String>,
    pinned: bool,
    done: bool,
}

impl Seen {
    fn read(task: Task) -> Option<Seen> {
        let text = std::fs::read_to_string(&task.path).ok()?;
        let get = |key: &str| {
            meta_in(&text, key)
                .map(|v| v.trim().trim_matches('"').to_string())
                .filter(|v| !v.is_empty())
        };
        Some(Seen {
            priority: get("priority").unwrap_or_else(|| "low".to_string()),
            from: get(FROM),
            own: get(OWN),
            pinned: get(PINNED).is_some_and(|v| v != "no" && v != "false"),
            done: task.column == Column::Done,
            task,
        })
    }

    /// Its own priority: what it had before auto raised it.
    fn own(&self) -> &str {
        match (&self.from, &self.own) {
            (Some(_), Some(own)) => own,
            _ => &self.priority,
        }
    }
}

/// Bring every ticket on `board` in line with its auto priority setting.
/// Returns (and queues for [`take_notes`]) a line for each change.
pub fn reconcile(board: &Board) -> Result<Vec<String>> {
    let settings = board_settings::load(board);
    let on = settings.auto_priority();
    let override_on = settings.priority_override();
    let seen: Vec<Seen> = board.tasks()?.into_iter().filter_map(Seen::read).collect();
    let mut lines = Vec::new();
    let mut write =
        |s: &Seen, priority: &str, from: Option<&str>, own: Option<&str>, why: String| {
            let result = s
                .task
                .hash()
                .and_then(|h| s.task.write_priority(h, priority, from, own, s.pinned));
            match result {
                Ok(()) => lines.push(why),
                Err(err) => lines.push(format!(
                    "auto priority left {} alone: {err:#}",
                    s.task.id_display()
                )),
            }
        };

    if !on {
        for s in seen.iter().filter(|s| s.from.is_some()) {
            let own = s.own().to_string();
            write(
                s,
                &own,
                None,
                None,
                format!("{} back to {own} (auto priority off)", s.task.id_display()),
            );
        }
        return Ok(queue(lines));
    }

    // Who requires whom: `requirers[j]` are the tickets that require j.
    let mut requirers: Vec<Vec<usize>> = vec![Vec::new(); seen.len()];
    let index = |id: &str| seen.iter().position(|s| same_id(id, &s.task));
    for (i, s) in seen.iter().enumerate() {
        for link in s.task.links() {
            let Some(j) = index(&link.id) else { continue };
            match link.kind {
                LinkType::Requires if i != j => requirers[j].push(i),
                LinkType::RequiredBy if i != j => requirers[i].push(j),
                _ => {}
            }
        }
    }
    for list in &mut requirers {
        list.sort_by_key(|&i| seen[i].task.id);
        list.dedup();
    }

    let pinned = |s: &Seen| override_on && s.pinned;
    // Start from each ticket's own; raise until nothing changes. Three
    // steps of priority, so this ends, cycles included.
    let mut effective: Vec<usize> = seen
        .iter()
        .map(|s| {
            if pinned(s) {
                rank(&s.priority)
            } else {
                rank(s.own())
            }
        })
        .collect();
    let mut source: Vec<Option<usize>> = vec![None; seen.len()];
    loop {
        let mut changed = false;
        for j in 0..seen.len() {
            if seen[j].done || pinned(&seen[j]) {
                continue;
            }
            for &i in &requirers[j] {
                if !seen[i].done && effective[i] > effective[j] {
                    effective[j] = effective[i];
                    source[j] = Some(i);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    for (j, s) in seen.iter().enumerate() {
        if s.done || pinned(s) {
            continue;
        }
        let own = s.own().to_string();
        match source[j].filter(|_| effective[j] > rank(&own)) {
            Some(i) => {
                let to = PRIORITIES[effective[j]];
                let by = seen[i].task.id_display();
                if s.priority != to
                    || s.from.as_deref() != Some(by.as_str())
                    || s.own.as_deref() != Some(own.as_str())
                {
                    write(
                        s,
                        to,
                        Some(&by),
                        Some(&own),
                        format!("{} raised to {to}: {by} requires it", s.task.id_display()),
                    );
                }
            }
            None if s.from.is_some() || s.priority != own => write(
                s,
                &own,
                None,
                None,
                format!(
                    "{} back to {own}: nothing more urgent requires it",
                    s.task.id_display()
                ),
            ),
            None => {}
        }
    }
    Ok(queue(lines))
}

/// Records a reconcile that failed outright, for [`take_notes`].
pub fn note_failure(err: &anyhow::Error) {
    NOTES.with(|n| {
        n.borrow_mut()
            .push(format!("auto priority couldn't run: {err:#}"))
    });
}

fn queue(lines: Vec<String>) -> Vec<String> {
    NOTES.with(|n| n.borrow_mut().extend(lines.iter().cloned()));
    lines
}

/// Set a ticket's priority by hand (ticket 215): `amd set <ref> priority`,
/// the editor, the AI tool. It becomes the ticket's own priority, any raise
/// is cleared, and with auto priority and override both on it's pinned.
/// `auto` unpins instead, leaving the priority to auto. Doesn't commit and
/// doesn't reconcile: the caller's `with_auto_commit` does both.
pub fn set_by_hand(board: &Board, task: &Task, value: &str) -> Result<String> {
    let settings = board_settings::load(board);
    let value = value.trim().to_lowercase();
    let id = task.id_display();
    if value == "auto" {
        if !settings.auto_priority() {
            bail!("auto priority is off for this board (`amd auto-priority on`)");
        }
        let Some(s) = Seen::read(task.clone()) else {
            bail!("couldn't read {id}");
        };
        let own = s.own().to_string();
        task.write_priority(task.hash()?, &own, None, None, false)?;
        return Ok(format!("{id} priority follows auto priority again"));
    }
    let pin = settings.auto_priority() && settings.priority_override();
    task.write_priority(task.hash()?, &value, None, None, pin)?;
    Ok(if pin {
        format!("{id} priority {value} (pinned: auto priority leaves it)")
    } else {
        format!("{id} priority {value}")
    })
}

/// What `amd show` and the editor say about a ticket's priority, if auto
/// priority has touched it: raised, or pinned.
pub fn note(board: &Board, task: &Task) -> Option<String> {
    let s = Seen::read(task.clone())?;
    let settings = board_settings::load(board);
    if let (Some(from), Some(own)) = (&s.from, &s.own) {
        return Some(format!(
            "priority {}: raised by auto priority, {from} requires it (own: {own})",
            s.priority
        ));
    }
    (s.pinned && settings.auto_priority() && settings.priority_override()).then(|| {
        format!(
            "priority {} pinned by hand: auto priority leaves it",
            s.priority
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create::{Draft, NewTicket};
    use crate::links::apply;
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

    fn requires(board: &Board, from: &str, to: &str) {
        apply(
            &find(board, from),
            LinkType::Requires,
            &find(board, to),
            false,
            false,
        )
        .unwrap();
    }

    fn settings(board: &Board, auto: bool, override_on: bool) {
        let mut s = board_settings::load(board);
        s.set_auto_priority(auto);
        s.set_priority_override(override_on);
        s.save(board).unwrap();
    }

    fn priorities(board: &Board) -> Vec<String> {
        let mut tasks = board.tasks().unwrap();
        tasks.sort_by_key(|t| t.id);
        tasks.iter().map(Task::priority).collect()
    }

    fn set(board: &Board, id: &str, value: &str) {
        let t = find(board, id);
        t.set_priority(t.hash().unwrap(), value).unwrap();
    }

    #[test]
    fn a_chain_inherits_the_most_urgent_priority_and_says_why() {
        let (_dir, board) = board_with(&["Goal", "Middle", "Base", "Other"]);
        set(&board, "001", "high");
        requires(&board, "001", "002");
        requires(&board, "002", "003");
        // Off: nothing moves.
        reconcile(&board).unwrap();
        assert_eq!(priorities(&board), ["high", "low", "low", "low"]);

        settings(&board, true, true);
        let lines = reconcile(&board).unwrap();
        assert_eq!(priorities(&board), ["high", "high", "high", "low"]);
        assert!(
            lines.contains(&"003 raised to high: 002 requires it".to_string()),
            "{lines:?}"
        );
        assert_eq!(
            find(&board, "003").meta("priority-from").as_deref(),
            Some("002")
        );
        assert_eq!(
            find(&board, "003").meta("priority-own").as_deref(),
            Some("low")
        );
        assert!(
            note(&board, &find(&board, "003"))
                .unwrap()
                .contains("002 requires it")
        );
        // Settled: running again changes nothing.
        assert!(reconcile(&board).unwrap().is_empty());

        // The link goes: 003 is back to its own.
        apply(
            &find(&board, "002"),
            LinkType::Requires,
            &find(&board, "003"),
            true,
            false,
        )
        .unwrap();
        reconcile(&board).unwrap();
        assert_eq!(priorities(&board), ["high", "high", "low", "low"]);
        assert!(find(&board, "003").meta("priority-from").is_none());
    }

    #[test]
    fn a_done_requirer_pushes_nothing_and_a_cycle_ends() {
        let (_dir, board) = board_with(&["Finished", "Needed", "A", "B"]);
        set(&board, "001", "high");
        // Done first (done waits for what it requires, ticket 213), linked
        // after.
        for column in [Column::Todo, Column::Doing, Column::Done] {
            board.move_task(&find(&board, "001"), column, true).unwrap();
        }
        requires(&board, "001", "002");
        set(&board, "003", "medium");
        requires(&board, "003", "004");
        requires(&board, "004", "003");
        settings(&board, true, true);
        reconcile(&board).unwrap();
        assert_eq!(priorities(&board), ["high", "low", "medium", "medium"]);
    }

    #[test]
    fn override_pins_a_hand_set_priority_and_auto_unpins_it() {
        let (_dir, board) = board_with(&["Goal", "Needed"]);
        set(&board, "001", "high");
        requires(&board, "001", "002");
        settings(&board, true, true);
        reconcile(&board).unwrap();
        assert_eq!(priorities(&board), ["high", "high"]);

        // By hand, with override on: pinned, and it stays low.
        let line = set_by_hand(&board, &find(&board, "002"), "low").unwrap();
        assert!(line.contains("pinned"), "{line}");
        reconcile(&board).unwrap();
        assert_eq!(priorities(&board), ["high", "low"]);
        assert!(
            note(&board, &find(&board, "002"))
                .unwrap()
                .contains("pinned")
        );

        // `auto` hands it back.
        set_by_hand(&board, &find(&board, "002"), "auto").unwrap();
        reconcile(&board).unwrap();
        assert_eq!(priorities(&board), ["high", "high"]);

        // Override off: a hand-set priority is only a floor.
        settings(&board, true, false);
        set_by_hand(&board, &find(&board, "002"), "low").unwrap();
        reconcile(&board).unwrap();
        assert_eq!(priorities(&board), ["high", "high"]);
        assert!(set_by_hand(&board, &find(&board, "002"), "urgent").is_err());
    }

    #[test]
    fn switching_off_puts_raised_tickets_back_and_keeps_pins() {
        let (_dir, board) = board_with(&["Goal", "Needed", "Pinned"]);
        set(&board, "001", "high");
        requires(&board, "001", "002");
        requires(&board, "001", "003");
        settings(&board, true, true);
        set_by_hand(&board, &find(&board, "003"), "medium").unwrap();
        reconcile(&board).unwrap();
        assert_eq!(priorities(&board), ["high", "high", "medium"]);

        settings(&board, false, true);
        let lines = reconcile(&board).unwrap();
        assert_eq!(priorities(&board), ["high", "low", "medium"]);
        assert_eq!(lines, ["002 back to low (auto priority off)"]);
        assert!(
            find(&board, "003").meta("priority-pinned").is_some(),
            "the pin is kept"
        );
        assert!(set_by_hand(&board, &find(&board, "002"), "auto").is_err());
    }

    #[test]
    fn a_ticket_someone_else_has_locked_is_left_alone_with_a_note() {
        let (_dir, board) = board_with(&["Goal", "Locked"]);
        set(&board, "001", "high");
        requires(&board, "001", "002");
        let t = find(&board, "002");
        let until = (time::OffsetDateTime::now_utc() + time::Duration::hours(2))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        let text = std::fs::read_to_string(&t.path).unwrap();
        let text = crate::task::set_meta(&text, "locked-by", "\"someone@else.example\"").unwrap();
        let text = crate::task::set_meta(&text, "locked-until", &format!("{until:?}")).unwrap();
        std::fs::write(&t.path, text).unwrap();
        settings(&board, true, true);
        let lines = reconcile(&board).unwrap();
        assert_eq!(priorities(&board), ["high", "low"]);
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("auto priority left 002 alone")),
            "{lines:?}"
        );
    }
}
