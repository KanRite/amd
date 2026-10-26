//! Recurring tickets and due dates (ticket 143).
//!
//! A ticket with `recur: monthly` (or `weekly`, `fortnightly`, `quarterly`,
//! `yearly`, `every N days|weeks|months|years`) comes round again: moving
//! it to a done column makes the next one in the same commit, a new ticket
//! (ids are never reused) in the board's landing column, due one interval
//! on, with `recurs-from` naming the one it follows. It carries everything
//! but progress: title, body (its checklist unticked, no children), tags,
//! points, priority, repository, branch type, assignee and the rule; not the
//! lock, epic or sprint, or links.
//!
//! `due: YYYY-MM-DD` works on any ticket. One past its date and not done is
//! overdue, which the card and `rite show` say; nothing makes a second copy,
//! since there's no daemon and the next one only comes from done.

use std::fmt;

use anyhow::{Result, bail};
use time::{Date, Duration, Month, macros::format_description};

use crate::board::{Board, Kind};
use crate::task::Task;

/// How often a ticket repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Every {
    Days(u32),
    Weeks(u32),
    Months(u32),
    Years(u32),
}

impl Every {
    /// `weekly`, `fortnightly`, `monthly`, `quarterly`, `yearly` (or
    /// `daily`, `annually`), or `every N days|weeks|months|years`, case
    /// ignored.
    pub fn parse(rule: &str) -> Result<Every> {
        let rule = rule.trim().to_lowercase();
        let every = match rule.as_str() {
            "daily" => Every::Days(1),
            "weekly" => Every::Weeks(1),
            "fortnightly" => Every::Weeks(2),
            "monthly" => Every::Months(1),
            "quarterly" => Every::Months(3),
            "yearly" | "annually" => Every::Years(1),
            other => {
                let words: Vec<&str> = other.split_whitespace().collect();
                let (n, unit) = match words.as_slice() {
                    ["every", unit] => (1, *unit),
                    ["every", n, unit] => (n.parse::<u32>().map_err(|_| unknown(&rule))?, *unit),
                    _ => return Err(unknown(&rule)),
                };
                if n == 0 || n > 999 {
                    return Err(unknown(&rule));
                }
                match unit.trim_end_matches('s') {
                    "day" => Every::Days(n),
                    "week" => Every::Weeks(n),
                    "month" => Every::Months(n),
                    "year" => Every::Years(n),
                    _ => return Err(unknown(&rule)),
                }
            }
        };
        Ok(every)
    }

    /// The date one interval after `date`. A month that hasn't the day
    /// (31 January plus a month) gives its last day.
    pub fn after(self, date: Date) -> Date {
        match self {
            Every::Days(n) => date + Duration::days(i64::from(n)),
            Every::Weeks(n) => date + Duration::weeks(i64::from(n)),
            Every::Months(n) => add_months(date, n),
            Every::Years(n) => add_months(date, n * 12),
        }
    }
}

impl fmt::Display for Every {
    /// The plain form, as stored: `monthly`, `every 3 weeks`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Every::Days(1) => write!(f, "daily"),
            Every::Weeks(1) => write!(f, "weekly"),
            Every::Weeks(2) => write!(f, "fortnightly"),
            Every::Months(1) => write!(f, "monthly"),
            Every::Months(3) => write!(f, "quarterly"),
            Every::Years(1) => write!(f, "yearly"),
            Every::Days(n) => write!(f, "every {n} days"),
            Every::Weeks(n) => write!(f, "every {n} weeks"),
            Every::Months(n) => write!(f, "every {n} months"),
            Every::Years(n) => write!(f, "every {n} years"),
        }
    }
}

fn unknown(rule: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "'{rule}' isn't a repeat: use weekly, fortnightly, monthly, quarterly, yearly, or every N days|weeks|months|years"
    )
}

fn add_months(date: Date, months: u32) -> Date {
    let total = date.year() * 12 + i32::from(u8::from(date.month())) - 1 + months as i32;
    let (year, month) = (total.div_euclid(12), (total.rem_euclid(12) + 1) as u8);
    let month = Month::try_from(month).unwrap_or(Month::January);
    let last = time::util::days_in_month(month, year);
    Date::from_calendar_date(year, month, date.day().min(last)).unwrap_or(date)
}

/// A `YYYY-MM-DD` date.
pub fn parse_date(value: &str) -> Result<Date> {
    match Date::parse(value.trim(), format_description!("[year]-[month]-[day]")) {
        Ok(date) => Ok(date),
        Err(_) => bail!("'{value}' isn't a date: write it as YYYY-MM-DD"),
    }
}

/// Today, in UTC.
pub fn today() -> Date {
    time::OffsetDateTime::now_utc().date()
}

/// When the next one is due: one interval after this one's due date (else
/// the day it was done), stepped on until it's after `done_on`, so a ticket
/// done late doesn't make a next one that's already overdue.
pub fn next_due(due: Option<Date>, done_on: Date, every: Every) -> Date {
    let mut next = every.after(due.unwrap_or(done_on));
    while next <= done_on {
        next = every.after(next);
    }
    next
}

/// Whether a ticket is overdue on `today`: past its due date and not in a
/// done column.
pub fn overdue(task: &Task, today: Date) -> bool {
    !task.column.is(Kind::Done)
        && task
            .due()
            .and_then(|d| parse_date(&d).ok())
            .is_some_and(|due| due < today)
}

/// What [`make_next`] made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Next {
    pub id: String,
    pub due: String,
}

/// Makes the next occurrence of `done`, a recurring ticket just moved to a
/// done column, on `board` (ticket 143). `None` when it doesn't recur.
/// Writes the ticket and the counter; the caller's commit takes them.
pub fn make_next(board: &Board, done: &Task, today: Date) -> Result<Option<Next>> {
    let Some(rule) = done.recur() else {
        return Ok(None);
    };
    let every = Every::parse(&rule)?;
    let due = next_due(done.due().and_then(|d| parse_date(&d).ok()), today, every);
    let templates = crate::templates::Templates::load(board)?;
    let draft = crate::create::Draft::prepare(
        board,
        &templates,
        crate::create::NewTicket {
            title: done.title(),
            template: crate::templates::DEFAULT_TEMPLATE.to_string(),
            branch_type: done.branch_type().unwrap_or_default(),
            assignees: done.assignees(),
            repository: done.repository().unwrap_or_default(),
            tags: done.tags(),
            ..Default::default()
        },
    )?;
    let body = crate::transition::clone_body(&done.body()?);
    let text = crate::model::splice_body(&draft.body, &body);
    let created = draft.write(board, Some(text))?;
    let task = &created.task;
    if let Some(points) = done.points() {
        task.set_points(task.hash()?, &points)?;
    }
    task.set_priority(task.hash()?, &done.priority())?;
    task.set_recur(task.hash()?, &every.to_string())?;
    task.set_due(task.hash()?, &due.to_string())?;
    task.set_recurs_from(task.hash()?, &done.id_display())?;
    Ok(Some(Next {
        id: task.id_display(),
        due: due.to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    #[test]
    fn rules_read_and_write_back_plainly() {
        for (given, plain) in [
            ("Monthly", "monthly"),
            ("weekly", "weekly"),
            ("every 2 weeks", "fortnightly"),
            ("every 3 months", "quarterly"),
            ("every week", "weekly"),
            ("every 10 days", "every 10 days"),
            ("annually", "yearly"),
            ("every 2 years", "every 2 years"),
        ] {
            assert_eq!(Every::parse(given).unwrap().to_string(), plain, "{given}");
        }
        for bad in [
            "sometimes",
            "every 0 days",
            "every x weeks",
            "every 3 fortnights",
            "",
        ] {
            assert!(Every::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn months_keep_the_day_or_take_the_last() {
        assert_eq!(
            Every::Months(1).after(date!(2026 - 01 - 31)),
            date!(2026 - 02 - 28)
        );
        assert_eq!(
            Every::Months(1).after(date!(2026 - 12 - 15)),
            date!(2027 - 01 - 15)
        );
        assert_eq!(
            Every::Years(1).after(date!(2028 - 02 - 29)),
            date!(2029 - 02 - 28)
        );
        assert_eq!(
            Every::Weeks(2).after(date!(2026 - 10 - 17)),
            date!(2026 - 10 - 31)
        );
    }

    #[test]
    fn the_next_one_is_due_after_today_even_when_this_was_late() {
        let monthly = Every::Months(1);
        // On time: one month after its due date.
        assert_eq!(
            next_due(Some(date!(2026 - 10 - 01)), date!(2026 - 09 - 30), monthly),
            date!(2026 - 11 - 01)
        );
        // Three months late: the next due date still to come.
        assert_eq!(
            next_due(Some(date!(2026 - 07 - 01)), date!(2026 - 10 - 17), monthly),
            date!(2026 - 11 - 01)
        );
        // No due date: from the day it was done.
        assert_eq!(
            next_due(None, date!(2026 - 10 - 17), Every::Weeks(1)),
            date!(2026 - 10 - 24)
        );
    }

    #[test]
    fn done_makes_the_next_one_with_everything_but_progress() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("acme-tickets").join("tasks");
        for column in ["backlog", "todo", "doing", "done"] {
            std::fs::create_dir_all(root.join(column)).unwrap();
        }
        let board = Board::at(root.clone());
        std::fs::write(
            root.join("done/007-renew-the-certificate.md"),
            "---\nid: \"007\"\ntitle: \"Renew the certificate\"\nassignee: \"Sam Lee\"\ntags: [OPS]\npoints: \"2\"\npriority: \"high\"\nrecur: \"monthly\"\ndue: \"2026-10-01\"\nlocked-by: \"sam@acme.io\"\n---\n\n## Notes\n\n- [x] order it\n- [ ] install it\n",
        )
        .unwrap();
        let done = board.find("007").unwrap();
        let next = make_next(&board, &done, date!(2026 - 10 - 03))
            .unwrap()
            .unwrap();
        assert_eq!(next.due, "2026-11-01");
        let made = board.find(&next.id).unwrap();
        assert_eq!(made.column.as_str(), "backlog");
        assert_eq!(made.title(), "Renew the certificate");
        assert_eq!(made.assignee().as_deref(), Some("Sam Lee"));
        assert_eq!(made.points().as_deref(), Some("2"));
        assert_eq!(made.priority(), "high");
        assert_eq!(made.recur().as_deref(), Some("monthly"));
        assert_eq!(made.due().as_deref(), Some("2026-11-01"));
        assert_eq!(made.recurs_from().as_deref(), Some("007"));
        assert!(made.tags().iter().any(|t| t == "OPS"));
        let body = made.body().unwrap();
        assert!(body.contains("- [ ] order it"), "unticked: {body}");
        assert!(
            made.meta("locked-by").unwrap_or_default().is_empty(),
            "no lock"
        );
        // Not recurring: nothing.
        std::fs::write(
            root.join("done/020-once.md"),
            "---\nid: \"020\"\ntitle: \"Once\"\n---\n",
        )
        .unwrap();
        assert_eq!(
            make_next(
                &board,
                &board.find("020-once").unwrap(),
                date!(2026 - 10 - 03)
            )
            .unwrap(),
            None
        );
    }
}
