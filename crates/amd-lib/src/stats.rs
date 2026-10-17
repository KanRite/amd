//! Statistics (ticket 233): tickets raised, done and closed over a period,
//! and a running sprint's burn-up and burn-down, all read from the board's
//! git history when asked. Nothing is stored (RFC 0005: chart data is
//! always derived, never kept).
//!
//! - **Raised**: a ticket file added to the board.
//! - **Done**: a ticket moved into a done-kind column from another kind.
//! - **Closed**: a ticket removed from the board without being done
//!   (abandoned or deleted).
//!
//! A sprint's start is the commit that marked it started; a started sprint
//! takes nothing more and gives nothing back, so its scope is the points
//! of the tickets in it now, and its burn is when each of them reached a
//! done column.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use time::{Date, OffsetDateTime};

use crate::board::{Board, Kind};
use crate::group::{GROUP_FILE, State};

const DAY: i64 = 86_400;

/// The periods offered, in days.
pub const PERIODS: [u32; 3] = [7, 30, 90];
const PERIOD_KEY: &str = "ui.stats-days";

/// The period the landing page counts over (`ui.stats-days` in your own
/// config): 7, 30 or 90 days, 30 unless chosen.
pub fn period() -> u32 {
    crate::settings::Config::load()
        .get(PERIOD_KEY)
        .and_then(|v| v.trim().parse().ok())
        .filter(|d| PERIODS.contains(d))
        .unwrap_or(30)
}

/// Remembers the period, on this machine only.
pub fn set_period(days: u32) -> Result<()> {
    if !PERIODS.contains(&days) {
        bail!("the period is 7, 30 or 90 days");
    }
    let mut config = crate::settings::Config::load();
    config.set(PERIOD_KEY, &days.to_string());
    config.save()
}

/// What happened to a ticket in one commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum What {
    Raised,
    /// Moved between columns; whether it left and entered a done kind.
    Moved {
        from_done: bool,
        to_done: bool,
    },
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// Commit time, seconds since the epoch.
    pub time: i64,
    /// The ticket's file name without `.md`.
    pub stem: String,
    pub what: What,
}

/// Raised, done and closed in a period, in all and a week at a time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub raised: usize,
    pub done: usize,
    pub closed: usize,
    /// Oldest week first; the last one ends today.
    pub weeks: Vec<Week>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Week {
    pub raised: usize,
    pub done: usize,
    pub closed: usize,
}

/// A started sprint's burn, in points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Burn {
    /// The board's repository name.
    pub repo: String,
    pub sprint: String,
    /// `YYYY-MM-DD`, the day it started.
    pub start: String,
    /// Its length in days.
    pub days: u32,
    /// Points of the tickets in it.
    pub total: u32,
    /// Points done by the end of each day from the start, up to today (or
    /// its last day): `done_by_day[0]` is the first day.
    pub done_by_day: Vec<u32>,
}

/// The board's ticket events since `since` (seconds since the epoch),
/// oldest first.
pub fn events(board: &Board, since: i64) -> Result<Vec<Event>> {
    let repo = board.repo_root();
    let rel = board
        .root
        .strip_prefix(repo)
        .unwrap_or(Path::new("tasks"))
        .to_string_lossy()
        .replace('\\', "/");
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "log",
            "-z",
            "--reverse",
            "--name-status",
            "-M",
            "--format=%x01%ct",
        ])
        .arg(format!("--since=@{since}"))
        .arg("--")
        .arg(&rel)
        .output()
        .context("running git log")?;
    if !out.status.success() {
        // A repository with no commits yet has no history to read.
        return Ok(Vec::new());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(parse(board, &rel, &text))
}

fn parse(board: &Board, rel: &str, text: &str) -> Vec<Event> {
    let workflow = board.workflow();
    // A folder's kind; a column since removed still says done by its name.
    let done = |folder: &str| {
        workflow
            .by_folder(folder)
            .map_or(folder == "done", |c| c.is(Kind::Done))
    };
    // The column folder and ticket stem of a board path, if it's a ticket.
    let ticket = |path: &str| -> Option<(String, String)> {
        let inside = path.strip_prefix(rel)?.strip_prefix('/')?;
        let folder = inside.split('/').next()?.to_string();
        let name = inside.rsplit('/').next()?;
        let stem = name.strip_suffix(".md")?;
        (stem.chars().next()?.is_ascii_digit() && inside.contains('/'))
            .then(|| (folder, stem.to_string()))
    };
    let mut events = Vec::new();
    let mut time = 0;
    let mut tokens = text.split('\0').map(|t| t.trim_matches('\n'));
    while let Some(token) = tokens.next() {
        if let Some(header) = token.strip_prefix('\u{1}') {
            time = header.trim().parse().unwrap_or(0);
            continue;
        }
        let Some(status) = token.chars().next() else {
            continue;
        };
        match status {
            'A' | 'D' => {
                let Some((_, stem)) = tokens.next().and_then(ticket) else {
                    continue;
                };
                let what = if status == 'A' {
                    What::Raised
                } else {
                    What::Closed
                };
                events.push(Event { time, stem, what });
            }
            'R' | 'C' => {
                let (from, to) = (tokens.next(), tokens.next());
                if status == 'C' {
                    continue;
                }
                let (Some((from, _)), Some((to, stem))) =
                    (from.and_then(ticket), to.and_then(ticket))
                else {
                    continue;
                };
                if from != to {
                    events.push(Event {
                        time,
                        stem,
                        what: What::Moved {
                            from_done: done(&from),
                            to_done: done(&to),
                        },
                    });
                }
            }
            'M' | 'T' => {
                tokens.next();
            }
            _ => {}
        }
    }
    events
}

/// Raised, done and closed over the `days` days up to `now` (seconds since
/// the epoch), on every board given, with a count for each week.
pub fn counts(boards: &[&Board], days: u32, now: i64) -> Result<Counts> {
    if days == 0 {
        bail!("a period of at least one day");
    }
    let since = now - i64::from(days) * DAY;
    let weeks = days.div_ceil(7) as usize;
    let mut counts = Counts {
        weeks: vec![Week::default(); weeks],
        ..Counts::default()
    };
    for board in boards {
        for event in events(board, since)? {
            if event.time < since {
                continue;
            }
            // Weeks count back from now: the last one ends today.
            let back = ((now - event.time).max(0) / (7 * DAY)) as usize;
            let Some(week) = weeks.checked_sub(back + 1).map(|i| &mut counts.weeks[i]) else {
                continue;
            };
            match event.what {
                What::Raised => {
                    counts.raised += 1;
                    week.raised += 1;
                }
                What::Moved {
                    from_done: false,
                    to_done: true,
                } => {
                    counts.done += 1;
                    week.done += 1;
                }
                What::Closed => {
                    counts.closed += 1;
                    week.closed += 1;
                }
                What::Moved { .. } => {}
            }
        }
    }
    Ok(counts)
}

/// When the sprint `name` was started: the first commit whose
/// `_group.md` says so. `None` when that isn't in the history (an
/// uncommitted board).
pub fn sprint_started(board: &Board, name: &str) -> Option<i64> {
    let repo = board.repo_root();
    let file = board
        .dir(crate::board::Column::Backlog)
        .join(name)
        .join(GROUP_FILE);
    let rel = file.strip_prefix(repo).ok()?;
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "log",
            "--reverse",
            "--format=%ct",
            "-S",
            "started: \"started\"",
        ])
        .arg("--")
        .arg(rel)
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .and_then(|l| l.trim().parse().ok())
}

/// The burn of every started sprint on `board`, as of `now`.
pub fn burns(board: &Board, repo: &str, now: i64) -> Result<Vec<Burn>> {
    let mut out = Vec::new();
    for group in board.groups()? {
        if group.is_sprint() && group.state == State::Started {
            out.push(burn_of(board, repo, &group, now)?);
        }
    }
    Ok(out)
}

/// One sprint's burn as of `now` (a closing sprint's too, ticket 052).
pub fn burn_of(board: &Board, repo: &str, group: &crate::group::Group, now: i64) -> Result<Burn> {
    let start = sprint_started(board, &group.name).unwrap_or(now);
    let members = board.tasks_in_group(&group.name)?;
    let total = members.iter().map(points_of).sum();
    // When each ticket last reached done, for those done now.
    let mut done_at: HashMap<String, i64> = HashMap::new();
    for event in events(board, start - DAY)? {
        if let What::Moved {
            from_done: false,
            to_done: true,
        } = event.what
        {
            done_at.insert(event.stem, event.time);
        }
    }
    let start_day = day_start(start);
    let elapsed = ((now - start_day).max(0) / DAY) as u32;
    let last = elapsed.min(group.days.saturating_sub(1));
    let done_by_day = (0..=last)
        .map(|d| {
            let end = start_day + i64::from(d + 1) * DAY;
            members
                .iter()
                .filter(|t| t.column.is(Kind::Done))
                .filter(|t| done_at.get(&t.stem).copied().unwrap_or(start) < end)
                .map(points_of)
                .sum()
        })
        .collect();
    Ok(Burn {
        repo: repo.to_string(),
        sprint: group.name.clone(),
        start: date_of(start).to_string(),
        days: group.days,
        total,
        done_by_day,
    })
}

/// A ticket's points, 0 when unsized.
pub fn points_of(task: &crate::task::Task) -> u32 {
    task.points()
        .and_then(|p| p.trim().parse::<u32>().ok())
        .unwrap_or(0)
}

/// Midnight UTC of the day `time` falls on.
fn day_start(time: i64) -> i64 {
    time - time.rem_euclid(DAY)
}

pub(crate) fn date_of(time: i64) -> Date {
    OffsetDateTime::from_unix_timestamp(time)
        .map(|t| t.date())
        .unwrap_or(Date::MIN)
}

/// Now, in seconds since the epoch.
pub fn now() -> i64 {
    OffsetDateTime::now_utc().unix_timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git;
    use std::fs;

    fn board() -> (tempfile::TempDir, Board) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("acme-tickets");
        for column in ["backlog", "todo", "doing", "done"] {
            fs::create_dir_all(repo.join("tasks").join(column)).unwrap();
        }
        git::checked(&repo, &["init", "-q", "-b", "main"]).unwrap();
        for (k, v) in [
            ("user.email", "sam@acme.io"),
            ("user.name", "Sam Lee"),
            ("commit.gpgsign", "false"),
        ] {
            git::checked(&repo, &["config", k, v]).unwrap();
        }
        (dir, Board::at(repo.join("tasks")))
    }

    fn commit(board: &Board, message: &str, at: i64) {
        let repo = board.repo_root();
        git::checked(repo, &["add", "-A"]).unwrap();
        let date = format!("@{at} +0000");
        let out = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["commit", "-q", "-m", message])
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    }

    fn ticket(board: &Board, column: &str, stem: &str, extra: &str) {
        fs::write(
            board.root.join(column).join(format!("{stem}.md")),
            format!("---\nid: \"{}\"\n{extra}---\n\nBody {stem}\n", &stem[..3]),
        )
        .unwrap();
    }

    fn mv(board: &Board, from: &str, to: &str) {
        git::checked(
            board.repo_root(),
            &["mv", &format!("tasks/{from}"), &format!("tasks/{to}")],
        )
        .unwrap();
    }

    #[test]
    fn raised_done_and_closed_are_counted_by_week() {
        let (_dir, board) = board();
        let now = 1_800_000_000;
        // Raised 20 days ago (outside a week, inside 30 days).
        ticket(&board, "backlog", "001-a", "");
        ticket(&board, "backlog", "002-b", "");
        commit(&board, "new", now - 20 * DAY);
        // Through to done 2 days ago; 002 abandoned yesterday.
        mv(&board, "backlog/001-a.md", "todo/001-a.md");
        commit(&board, "todo", now - 3 * DAY);
        mv(&board, "todo/001-a.md", "doing/001-a.md");
        commit(&board, "doing", now - 3 * DAY + 60);
        mv(&board, "doing/001-a.md", "done/001-a.md");
        commit(&board, "done", now - 2 * DAY);
        git::checked(board.repo_root(), &["rm", "-q", "tasks/backlog/002-b.md"]).unwrap();
        commit(&board, "abandon", now - DAY);
        // An epic's file isn't a ticket.
        fs::create_dir_all(board.root.join("backlog/Big plan")).unwrap();
        fs::write(board.root.join("backlog/Big plan/_group.md"), "---\n---\n").unwrap();
        commit(&board, "group", now - DAY);

        let month = counts(&[&board], 30, now).unwrap();
        assert_eq!((month.raised, month.done, month.closed), (2, 1, 1));
        assert_eq!(month.weeks.len(), 5);
        assert_eq!(
            month.weeks[4],
            Week {
                raised: 0,
                done: 1,
                closed: 1
            }
        );
        assert_eq!(
            month.weeks[2].raised, 2,
            "20 days back is the third week of five"
        );
        let week = counts(&[&board], 7, now).unwrap();
        assert_eq!((week.raised, week.done, week.closed), (0, 1, 1));
    }

    #[test]
    fn a_started_sprint_burns_as_its_tickets_reach_done() {
        let (_dir, board) = board();
        let day0 = 1_800_000_000 - 1_800_000_000 % DAY;
        fs::create_dir_all(board.root.join("backlog/s1")).unwrap();
        fs::write(
            board.root.join("backlog/s1/_group.md"),
            "---\nkind: \"sprint\"\nname: \"s1\"\ndays: \"5\"\nstarted: \"pending\"\n---\n",
        )
        .unwrap();
        ticket(
            &board,
            "backlog/s1",
            "001-a",
            "points: \"3\"\nepic: \"s1\"\n",
        );
        ticket(
            &board,
            "backlog/s1",
            "002-b",
            "points: \"5\"\nepic: \"s1\"\n",
        );
        commit(&board, "sprint", day0 - DAY);
        let group = board.root.join("backlog/s1/_group.md");
        let text = fs::read_to_string(&group)
            .unwrap()
            .replace("pending", "started");
        fs::write(&group, text).unwrap();
        commit(&board, "group-start: s1", day0 + 3600);
        assert_eq!(sprint_started(&board, "s1"), Some(day0 + 3600));
        // 001 done on day 1 (the second day).
        mv(&board, "backlog/s1/001-a.md", "todo/001-a.md");
        commit(&board, "todo", day0 + DAY + 100);
        mv(&board, "todo/001-a.md", "doing/001-a.md");
        commit(&board, "doing", day0 + DAY + 200);
        mv(&board, "doing/001-a.md", "done/001-a.md");
        commit(&board, "done", day0 + DAY + 300);

        let burn = &burns(&board, "acme-tickets", day0 + 2 * DAY + 10).unwrap()[0];
        assert_eq!((burn.sprint.as_str(), burn.days, burn.total), ("s1", 5, 8));
        assert_eq!(burn.done_by_day, vec![0, 3, 3], "today is the third day");
        assert_eq!(burn.start, date_of(day0).to_string());
        // Never past its last day.
        let late = &burns(&board, "acme-tickets", day0 + 30 * DAY).unwrap()[0];
        assert_eq!(late.done_by_day.len(), 5);
    }
}
