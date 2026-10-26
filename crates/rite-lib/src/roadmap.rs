//! Roadmaps (ticket 206): a plan for one board of the project, made of
//! milestones.
//!
//! A roadmap is one markdown file, `roadmaps/<slug>.md`, in the project's
//! tickets repository beside `tasks/` (not inside it, so the ticket scan never
//! sees it). The slug is the target board's name, so a board has at most one
//! roadmap. A milestone is a `## name` section holding its kind, dates and
//! description. Adding one also creates the epic — or, when the roadmap uses
//! sprints, the sprint — of the same name on the target board, in the same
//! change so one auto-commit records both.
//!
//! ```text
//! ---
//! name: "acme-api"
//! board: "acme-api"
//! goal: "Ship the beta."
//! sprints: true
//! ---
//!
//! ## Beta
//! - kind: epic
//! - start: 2026-10-01
//! - due: 2026-11-01
//! - days-per-week: 4
//! - duration: 12
//!
//! What the beta has to do.
//! ```
//!
//! A roadmap can be renamed (`rename`); its file moves with its name.
//!
//! A roadmap from before it had a board (no `board:` line) still reads; it is
//! offered for adoption by a board (`adopt`).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use time::macros::format_description;
use time::{Date, Duration};

use crate::board::Board;
use crate::group::{DEFAULT_DAYS, Group, Kind, State};

/// The folder inside the tickets repository that holds the roadmaps.
pub const FOLDER: &str = "roadmaps";

/// The most characters a milestone's short name may have.
pub const MILESTONE_NAME_MAX: usize = 20;

/// Working days a week when a milestone doesn't say.
pub const DEFAULT_DAYS_PER_WEEK: u32 = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Milestone {
    pub name: String,
    pub description: String,
    /// An epic, or a sprint whose length is `duration`, else `start` to `due`.
    pub kind: Kind,
    /// `YYYY-MM-DD`; empty when none was given.
    pub start: String,
    /// The due-by date, `YYYY-MM-DD`; empty when none was given.
    pub due: String,
    /// Working days a week (1 to 7); 0 means `DEFAULT_DAYS_PER_WEEK`.
    pub days_per_week: u32,
    /// Length in working days; 0 means none was given.
    pub duration: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roadmap {
    /// The file name without `.md`.
    pub slug: String,
    pub name: String,
    /// The board (repository) whose epics or sprints the milestones make;
    /// empty on a roadmap from before roadmaps had boards.
    pub board: String,
    /// What the roadmap is for.
    pub goal: String,
    /// Milestones are sprints rather than epics.
    pub sprints: bool,
    pub milestones: Vec<Milestone>,
}

/// How a roadmap or milestone stands today.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Total working days of the milestones that have a duration; `None` if
    /// none has.
    pub duration: Option<u32>,
    /// Working days left to the last due date; negative when it has passed.
    pub remaining: Option<i64>,
    /// Taking longer than calculated.
    pub late: bool,
}

fn folder(board: &Board) -> PathBuf {
    board.repo_root().join(FOLDER)
}

/// The file name a roadmap's name gives: lowercase letters and digits,
/// everything between them one `-`.
pub fn slug(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() {
            slug.extend(c.to_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_string()
}

fn parse_date(value: &str) -> Result<Date> {
    Date::parse(value, format_description!("[year]-[month]-[day]"))
        .map_err(|_| anyhow::anyhow!("'{value}' is not a date: write it as YYYY-MM-DD"))
}

/// A date field that may be left empty.
fn check_date(value: &str) -> Result<()> {
    if value.is_empty() {
        return Ok(());
    }
    parse_date(value).map(|_| ())
}

fn check_line(what: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("a {what} needs a name");
    }
    if value.contains(['\n', '\r']) {
        bail!("a {what} name is one line");
    }
    Ok(())
}

/// A sprint's length in days from its milestone's dates: start to due, at
/// least one day. Without both dates it falls back to the default length.
pub fn sprint_days(start: &str, due: &str) -> Result<u32> {
    if start.is_empty() || due.is_empty() {
        return Ok(DEFAULT_DAYS);
    }
    let days = (parse_date(due)? - parse_date(start)?).whole_days();
    if days < 0 {
        bail!("the milestone ends ({due}) before it starts ({start})");
    }
    Ok(u32::try_from(days).unwrap_or(u32::MAX).max(1))
}

/// Working days from `from` to `to`, both counted: a week's first
/// `per_week` days (Monday first) are working days. 0 when `to` is earlier.
pub fn working_days(from: Date, to: Date, per_week: u32) -> i64 {
    let per_week = if per_week == 0 {
        DEFAULT_DAYS_PER_WEEK
    } else {
        per_week.min(7)
    };
    let mut count = 0;
    let mut day = from;
    while day <= to {
        if u32::from(day.weekday().number_days_from_monday()) < per_week {
            count += 1;
        }
        let Some(next) = day.next_day() else { break };
        day = next;
    }
    count
}

impl Milestone {
    fn per_week(&self) -> u32 {
        if self.days_per_week == 0 {
            DEFAULT_DAYS_PER_WEEK
        } else {
            self.days_per_week
        }
    }

    /// Working days it takes: the duration given, else start to due.
    pub fn working_duration(&self) -> Option<u32> {
        if self.duration > 0 {
            return Some(self.duration);
        }
        let (start, due) = (parse_date(&self.start).ok()?, parse_date(&self.due).ok()?);
        Some(u32::try_from(working_days(start, due, self.per_week())).unwrap_or(u32::MAX))
    }

    /// Working days from `today` to the due date, counting both; negative
    /// (the days since) once it has passed.
    pub fn remaining(&self, today: Date) -> Option<i64> {
        let due = parse_date(&self.due).ok()?;
        if today <= due {
            return Some(working_days(today, due, self.per_week()));
        }
        let after = due.checked_add(Duration::days(1))?;
        Some(-working_days(after, today, self.per_week()))
    }

    /// Past due and not finished, or with less time left than its unfinished
    /// share of the duration. `done` and `total` count the epic's or
    /// sprint's tickets.
    pub fn late(&self, today: Date, done: usize, total: usize) -> bool {
        if total > 0 && done == total {
            return false;
        }
        let Some(remaining) = self.remaining(today) else {
            return false;
        };
        if remaining < 0 {
            return true;
        }
        let Some(duration) = self.working_duration() else {
            return false;
        };
        let unfinished = if total == 0 {
            1.0
        } else {
            1.0 - done as f64 / total as f64
        };
        (remaining as f64) < f64::from(duration) * unfinished
    }
}

impl Roadmap {
    /// How the roadmap stands on `today`; `progress` gives a milestone's
    /// finished and total ticket counts.
    pub fn summary(&self, today: Date, progress: &dyn Fn(&Milestone) -> (usize, usize)) -> Summary {
        let durations: Vec<u32> = self
            .milestones
            .iter()
            .filter_map(Milestone::working_duration)
            .collect();
        // ISO dates sort as text.
        let last = self
            .milestones
            .iter()
            .filter(|m| m.remaining(today).is_some())
            .max_by(|a, b| a.due.cmp(&b.due));
        Summary {
            duration: (!durations.is_empty()).then(|| durations.iter().sum()),
            remaining: last.and_then(|m| m.remaining(today)),
            late: self.milestones.iter().any(|m| {
                let (done, total) = progress(m);
                m.late(today, done, total)
            }),
        }
    }

    /// `summary`, reading each milestone's tickets from `target`.
    pub fn summary_on(&self, today: Date, target: &Board) -> Summary {
        self.summary(today, &|m| tickets_done(target, m))
    }

    fn render(&self) -> String {
        let quoted = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string());
        let mut out = format!("---\nname: {}\n", quoted(&self.name));
        if !self.board.is_empty() {
            out.push_str(&format!("board: {}\n", quoted(&self.board)));
        }
        if !self.goal.is_empty() {
            out.push_str(&format!("goal: {}\n", quoted(&self.goal)));
        }
        if self.sprints {
            out.push_str("sprints: true\n");
        }
        out.push_str("---\n");
        for m in &self.milestones {
            out.push_str(&format!("\n## {}\n- kind: {}\n", m.name, m.kind.as_str()));
            if !m.start.is_empty() {
                out.push_str(&format!("- start: {}\n", m.start));
            }
            if !m.due.is_empty() {
                out.push_str(&format!("- due: {}\n", m.due));
            }
            if m.days_per_week > 0 {
                out.push_str(&format!("- days-per-week: {}\n", m.days_per_week));
            }
            if m.duration > 0 {
                out.push_str(&format!("- duration: {}\n", m.duration));
            }
            if !m.description.is_empty() {
                out.push_str(&format!("\n{}\n", m.description.trim_end()));
            }
        }
        out
    }

    fn parse(slug: &str, text: &str) -> Roadmap {
        let mut roadmap = Roadmap {
            slug: slug.to_string(),
            name: slug.to_string(),
            board: String::new(),
            goal: String::new(),
            sprints: false,
            milestones: Vec::new(),
        };
        let mut lines = text.lines();
        if lines.next().map(str::trim) == Some("---") {
            for line in lines.by_ref() {
                if line.trim() == "---" {
                    break;
                }
                if let Some((key, value)) = line.split_once(':') {
                    let value = value.trim();
                    let text = serde_json::from_str::<String>(value)
                        .unwrap_or_else(|_| value.trim_matches('"').to_string());
                    match key.trim() {
                        "name" => roadmap.name = text,
                        "board" => roadmap.board = text,
                        "goal" => roadmap.goal = text,
                        "sprints" => roadmap.sprints = value == "true",
                        _ => {}
                    }
                }
            }
        }
        let mut description = Vec::new();
        let finish = |roadmap: &mut Roadmap, lines: &mut Vec<&str>| {
            if let Some(m) = roadmap.milestones.last_mut() {
                m.description = lines.join("\n").trim().to_string();
            }
            lines.clear();
        };
        for line in lines {
            if let Some(name) = line.strip_prefix("## ") {
                finish(&mut roadmap, &mut description);
                roadmap.milestones.push(Milestone {
                    name: name.trim().to_string(),
                    description: String::new(),
                    kind: Kind::Epic,
                    start: String::new(),
                    due: String::new(),
                    days_per_week: 0,
                    duration: 0,
                });
            } else if let Some(m) = roadmap.milestones.last_mut() {
                let field = line.strip_prefix("- ").and_then(|l| l.split_once(':'));
                match field {
                    Some(("kind", v)) if description.is_empty() => {
                        m.kind = if v.trim() == "sprint" {
                            Kind::Sprint
                        } else {
                            Kind::Epic
                        };
                    }
                    Some(("start", v)) if description.is_empty() => m.start = v.trim().to_string(),
                    Some(("due", v)) if description.is_empty() => m.due = v.trim().to_string(),
                    Some(("days-per-week", v)) if description.is_empty() => {
                        m.days_per_week = v.trim().parse().unwrap_or(0);
                    }
                    Some(("duration", v)) if description.is_empty() => {
                        m.duration = v.trim().parse().unwrap_or(0);
                    }
                    _ => description.push(line),
                }
            }
        }
        finish(&mut roadmap, &mut description);
        roadmap
    }

    fn path(&self, store: &Board) -> PathBuf {
        folder(store).join(format!("{}.md", self.slug))
    }
}

/// How many tickets of the epic or sprint a milestone made are done, and how
/// many it has.
fn tickets_done(target: &Board, milestone: &Milestone) -> (usize, usize) {
    let tasks = target.tasks_in_group(&milestone.name).unwrap_or_default();
    let done = tasks
        .iter()
        .filter(|t| t.column.is(crate::board::Kind::Done))
        .count();
    (done, tasks.len())
}

/// Every roadmap of the repository, by board name (those without a board
/// last).
pub fn list(store: &Board) -> Result<Vec<Roadmap>> {
    let dir = folder(store);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err).with_context(|| format!("reading {}", dir.display())),
    };
    let mut roadmaps = Vec::new();
    for entry in entries {
        let path = entry
            .with_context(|| format!("reading {}", dir.display()))?
            .path();
        if path.extension().and_then(|e| e.to_str()) == Some("md")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            roadmaps.push(Roadmap::parse(stem, &read(&path)?));
        }
    }
    roadmaps.sort_by_key(|r| {
        (
            r.board.is_empty(),
            r.board.to_lowercase(),
            r.name.to_lowercase(),
        )
    });
    Ok(roadmaps)
}

fn read(path: &Path) -> Result<String> {
    crate::eol::read(path).with_context(|| format!("reading {}", path.display()))
}

fn find(store: &Board, slug: &str) -> Result<Roadmap> {
    list(store)?
        .into_iter()
        .find(|r| r.slug == slug)
        .ok_or_else(|| anyhow::anyhow!("no roadmap called {slug}"))
}

/// The roadmap name a board gets unless it is given an alias: the board's
/// name without its `-tickets` suffix.
pub fn default_name(board_name: &str) -> String {
    let board_name = board_name.trim();
    board_name
        .strip_suffix("-tickets")
        .filter(|n| !n.is_empty())
        .unwrap_or(board_name)
        .to_string()
}

/// Create the (empty) roadmap for `board_name`, stored in `store`. It is
/// called `alias` if that isn't empty, else the board's `default_name`.
pub fn create(store: &Board, board_name: &str, alias: &str) -> Result<Roadmap> {
    let board_name = board_name.trim();
    if board_name.is_empty() {
        bail!("choose the board the roadmap is for");
    }
    let name = match alias.trim() {
        "" => default_name(board_name),
        alias => alias.to_string(),
    };
    check_line("roadmap", &name)?;
    let roadmap = Roadmap {
        slug: slug(&name),
        name,
        board: board_name.to_string(),
        goal: String::new(),
        sprints: false,
        milestones: Vec::new(),
    };
    if roadmap.slug.is_empty() {
        bail!(
            "'{}' has no letters or digits to name a file with",
            roadmap.name
        );
    }
    if list(store)?.iter().any(|r| r.board == board_name) || roadmap.path(store).exists() {
        bail!("{board_name} already has a roadmap");
    }
    write(store, &roadmap)?;
    Ok(roadmap)
}

/// Give a roadmap from before roadmaps had boards its board.
pub fn adopt(store: &Board, slug: &str, board_name: &str) -> Result<Roadmap> {
    let board_name = board_name.trim();
    let mut roadmap = find(store, slug)?;
    if !roadmap.board.is_empty() {
        bail!(
            "{} is already the roadmap of {}",
            roadmap.name,
            roadmap.board
        );
    }
    if board_name.is_empty() {
        bail!("choose the board the roadmap is for");
    }
    if list(store)?.iter().any(|r| r.board == board_name) {
        bail!("{board_name} already has a roadmap");
    }
    roadmap.board = board_name.to_string();
    write(store, &roadmap)?;
    Ok(roadmap)
}

/// Set what the roadmap is for, and whether its milestones are sprints.
pub fn set_details(store: &Board, slug: &str, goal: &str, sprints: bool) -> Result<Roadmap> {
    let mut roadmap = find(store, slug)?;
    roadmap.goal = goal.trim().to_string();
    roadmap.sprints = sprints;
    write(store, &roadmap)?;
    Ok(roadmap)
}

/// Give the roadmap `slug` a new name. The file is named after the name, so
/// it moves too (`git mv` when tracked, so history follows). Its board,
/// goal and milestones are untouched. Refused, changing nothing, if the name
/// is empty or its file name is another roadmap's.
pub fn rename(store: &Board, slug: &str, new_name: &str) -> Result<Roadmap> {
    let mut roadmap = find(store, slug)?;
    let name = new_name.trim().to_string();
    check_line("roadmap", &name)?;
    let new_slug = self::slug(&name);
    if new_slug.is_empty() {
        bail!("'{name}' has no letters or digits to name a file with");
    }
    let old = roadmap.path(store);
    roadmap.name = name;
    roadmap.slug = new_slug;
    let new = roadmap.path(store);
    if new != old {
        if new.exists() {
            bail!("there is already a roadmap called {}", roadmap.slug);
        }
        if crate::git::is_tracked(store.repo_root(), &old) {
            crate::git::mv(store.repo_root(), &old, &new)?;
        } else {
            fs::rename(&old, &new)
                .with_context(|| format!("moving {} to {}", old.display(), new.display()))?;
        }
    }
    write(store, &roadmap)?;
    Ok(roadmap)
}

fn write(store: &Board, roadmap: &Roadmap) -> Result<()> {
    let path = roadmap.path(store);
    fs::create_dir_all(folder(store))
        .with_context(|| format!("creating {}", folder(store).display()))?;
    fs::write(&path, roadmap.render()).with_context(|| format!("writing {}", path.display()))
}

/// Add a milestone to the roadmap `slug` (kept in `store`), and create its
/// epic — or sprint, if the roadmap uses sprints — on `target`, the roadmap's
/// board. All the checks run before anything is written, so a refused
/// milestone leaves no epic behind and no half-written roadmap.
///
/// An epic or sprint already called the milestone's name is used instead
/// (ticket 306), whatever kind it is: a sprint that hasn't started takes the
/// milestone's length and description, as a new one would; a started or
/// closed sprint is left as it is, with a warning, since it can't be
/// updated. The warnings come back beside the roadmap.
pub fn add_milestone(
    store: &Board,
    target: &Board,
    slug: &str,
    milestone: Milestone,
) -> Result<(Roadmap, Vec<String>)> {
    let mut roadmap = find(store, slug)?;
    if roadmap.board.is_empty() {
        bail!("give {} a board before adding milestones", roadmap.name);
    }
    let kind = if roadmap.sprints {
        Kind::Sprint
    } else {
        Kind::Epic
    };
    let milestone = Milestone {
        name: milestone.name.trim().to_string(),
        description: milestone.description.trim().to_string(),
        kind,
        ..milestone
    };
    check_line("milestone", &milestone.name)?;
    if milestone.name.chars().count() > MILESTONE_NAME_MAX {
        bail!("a milestone's short name is at most {MILESTONE_NAME_MAX} characters");
    }
    check_date(&milestone.start)?;
    check_date(&milestone.due)?;
    if !(0..=7).contains(&milestone.days_per_week) {
        bail!("days per week is 1 to 7");
    }
    if milestone.description.lines().any(|l| l.starts_with("## ")) {
        bail!("a description can't have a line starting with '## '");
    }
    let dates = sprint_days(&milestone.start, &milestone.due)?;
    if roadmap.milestones.iter().any(|m| m.name == milestone.name) {
        bail!(
            "{} already has a milestone {}",
            roadmap.name,
            milestone.name
        );
    }
    let sprint = kind == Kind::Sprint;
    // A sprint is as long as the duration given, else as start to due.
    let days = if milestone.duration > 0 {
        milestone.duration
    } else {
        dates
    };
    // `_group.md` keeps a description on one line.
    let one_line = milestone
        .description
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut warnings = Vec::new();
    if let Ok(mut existing) = target.group(&milestone.name) {
        // Use what's there, as what it is.
        let milestone = Milestone {
            kind: existing.kind,
            ..milestone
        };
        if existing.is_sprint() {
            if existing.accepts_changes() {
                existing.days = days;
                existing.description = one_line;
                existing.save()?;
            } else {
                warnings.push(format!(
                    "{} is a {} sprint, which can't be updated: the milestone uses it as it is",
                    existing.name,
                    if existing.state == State::Closed {
                        "closed"
                    } else {
                        "started"
                    },
                ));
            }
        }
        roadmap.milestones.push(milestone);
        write(store, &roadmap)?;
        return Ok((roadmap, warnings));
    }
    // The epic's name is the milestone's name; its description and dates stay
    // here. A sprint takes the description and the length.
    target.create_group(&Group {
        dir: target
            .dir(crate::board::Column::Backlog)
            .join(&milestone.name),
        name: milestone.name.clone(),
        kind,
        description: if sprint { one_line } else { String::new() },
        days,
        state: State::Pending,
    })?;
    roadmap.milestones.push(milestone);
    write(store, &roadmap)?;
    Ok((roadmap, warnings))
}

/// Take the milestone `name` off the roadmap `slug` (ticket 306). Its epic or
/// sprint on `target` goes too when it's empty (no tickets at all) and not
/// started; otherwise it stays, and the note says why.
pub fn delete_milestone(
    store: &Board,
    target: &Board,
    slug: &str,
    name: &str,
) -> Result<Vec<String>> {
    let mut roadmap = find(store, slug)?;
    let before = roadmap.milestones.len();
    roadmap.milestones.retain(|m| m.name != name);
    if roadmap.milestones.len() == before {
        bail!("{} has no milestone {name}", roadmap.name);
    }
    write(store, &roadmap)?;
    Ok(remove_container(target, name)?.into_iter().collect())
}

/// Delete the roadmap `slug` (ticket 306): each milestone's epic or sprint
/// on `target` goes when it's empty and not started (as
/// [`delete_milestone`]), then the roadmap's file.
pub fn delete(store: &Board, target: Option<&Board>, slug: &str) -> Result<Vec<String>> {
    let roadmap = find(store, slug)?;
    let mut notes = Vec::new();
    if let Some(target) = target {
        for m in &roadmap.milestones {
            notes.extend(remove_container(target, &m.name)?);
        }
    }
    let path = roadmap.path(store);
    fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    Ok(notes)
}

/// Removes the epic or sprint `name` when it's empty and not started; else a
/// note saying why it was kept. Nothing when there's none.
fn remove_container(target: &Board, name: &str) -> Result<Option<String>> {
    let Ok(group) = target.group(name) else {
        return Ok(None);
    };
    let what = if group.is_sprint() { "sprint" } else { "epic" };
    if !group.accepts_changes() {
        return Ok(Some(format!(
            "kept the {what} {name}: it {}",
            group.locked_word()
        )));
    }
    let tickets = target.tasks_in_group(name)?.len();
    if tickets > 0 {
        return Ok(Some(format!(
            "kept the {what} {name}: it has {tickets} ticket{}",
            if tickets == 1 { "" } else { "s" }
        )));
    }
    target.delete_group(name)?;
    Ok(None)
}

/// Change the start and due dates of the milestone `name` on the roadmap
/// `slug` (ticket 235). Either may be empty. A sprint milestone with no
/// duration of its own is as long as start to due, so its sprint on `target`
/// takes the new length while it hasn't started; a started sprint is a
/// commitment and keeps its length. Checked before anything is written.
pub fn set_milestone_dates(
    store: &Board,
    target: &Board,
    slug: &str,
    name: &str,
    start: &str,
    due: &str,
) -> Result<Roadmap> {
    let mut roadmap = find(store, slug)?;
    let (start, due) = (start.trim(), due.trim());
    check_date(start)?;
    check_date(due)?;
    let days = sprint_days(start, due)?;
    let Some(m) = roadmap.milestones.iter_mut().find(|m| m.name == name) else {
        bail!("{} has no milestone {name}", roadmap.name);
    };
    m.start = start.to_string();
    m.due = due.to_string();
    let resize = m.kind == Kind::Sprint && m.duration == 0;
    if resize
        && let Ok(mut group) = target.group(name)
        && group.is_sprint()
        && group.accepts_changes()
        && group.days != days
    {
        group.days = days;
        group.save()?;
    }
    write(store, &roadmap)?;
    Ok(roadmap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Column;
    use time::macros::date;

    fn board() -> (tempfile::TempDir, Board) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("tasks");
        fs::create_dir_all(root.join("backlog")).unwrap();
        (dir, Board::at(root))
    }

    fn milestone(name: &str, start: &str, due: &str) -> Milestone {
        Milestone {
            name: name.to_string(),
            description: "What it has to do.\n\nAnd more.".to_string(),
            kind: Kind::Epic,
            start: start.to_string(),
            due: due.to_string(),
            days_per_week: 0,
            duration: 0,
        }
    }

    #[test]
    fn roadmaps_drop_tickets_from_the_name_unless_given_an_alias() {
        let (_dir, board) = board();
        assert_eq!(default_name("acme-tickets"), "acme");
        assert_eq!(default_name("-tickets"), "-tickets");
        let r = create(&board, "acme-tickets", "").unwrap();
        assert_eq!(
            (r.name.as_str(), r.board.as_str()),
            ("acme", "acme-tickets")
        );
        let r = create(&board, "other-tickets", " Big Plan ").unwrap();
        assert_eq!(
            (r.name.as_str(), r.slug.as_str(), r.board.as_str()),
            ("Big Plan", "big-plan", "other-tickets")
        );
    }

    #[test]
    fn a_milestone_short_name_is_at_most_20_characters() {
        let (_dir, board) = board();
        create(&board, "acme-api", "").unwrap();
        let ok = milestone(&"a".repeat(20), "", "");
        add_milestone(&board, &board, "acme-api", ok).unwrap();
        let long = milestone(&"b".repeat(21), "", "");
        assert!(add_milestone(&board, &board, "acme-api", long).is_err());
    }

    #[test]
    fn renaming_a_roadmap_moves_its_file_and_keeps_the_rest() {
        let (_dir, board) = board();
        create(&board, "acme-api", "").unwrap();
        set_details(&board, "acme-api", "Ship it.", true).unwrap();
        add_milestone(&board, &board, "acme-api", milestone("Beta", "", "")).unwrap();
        let r = rename(&board, "acme-api", " Big Plan ").unwrap();
        assert_eq!((r.name.as_str(), r.slug.as_str()), ("Big Plan", "big-plan"));
        let all = list(&board).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0], r);
        assert_eq!(
            (all[0].board.as_str(), all[0].goal.as_str(), all[0].sprints),
            ("acme-api", "Ship it.", true)
        );
        assert_eq!(all[0].milestones.len(), 1);
        assert!(!folder(&board).join("acme-api.md").exists());
        // Only the capitals change: same file.
        let r = rename(&board, "big-plan", "BIG PLAN").unwrap();
        assert_eq!((r.name.as_str(), r.slug.as_str()), ("BIG PLAN", "big-plan"));
    }

    #[test]
    fn a_roadmap_cannot_be_renamed_to_nothing_or_to_another() {
        let (_dir, board) = board();
        create(&board, "one", "").unwrap();
        create(&board, "two", "").unwrap();
        assert!(rename(&board, "one", "  ").is_err());
        assert!(rename(&board, "one", "!!!").is_err());
        assert!(rename(&board, "one", "Two").is_err());
        assert!(rename(&board, "missing", "x").is_err());
        let names: Vec<_> = list(&board).unwrap().into_iter().map(|r| r.slug).collect();
        assert_eq!(names, ["one", "two"]);
    }

    #[test]
    fn slugs_are_file_names() {
        assert_eq!(slug("Q4 Launch!"), "q4-launch");
        assert_eq!(slug("  --  "), "");
    }

    #[test]
    fn a_roadmap_and_its_milestones_round_trip() {
        let (_dir, board) = board();
        create(&board, "acme-api", "").unwrap();
        set_details(&board, "acme-api", "Ship the beta.\nThen the rest.", false).unwrap();
        let mut beta = milestone("Beta", "2026-10-01", "2026-11-01");
        beta.days_per_week = 4;
        beta.duration = 12;
        add_milestone(&board, &board, "acme-api", beta).unwrap();
        let roadmaps = list(&board).unwrap();
        assert_eq!(roadmaps.len(), 1);
        let r = &roadmaps[0];
        assert_eq!((r.board.as_str(), r.sprints), ("acme-api", false));
        assert_eq!(r.goal, "Ship the beta.\nThen the rest.");
        let m = &r.milestones[0];
        assert_eq!((m.name.as_str(), m.due.as_str()), ("Beta", "2026-11-01"));
        assert_eq!((m.days_per_week, m.duration), (4, 12));
        assert_eq!(m.description, "What it has to do.\n\nAnd more.");
    }

    #[test]
    fn a_board_has_one_roadmap() {
        let (_dir, board) = board();
        create(&board, "acme-api", "").unwrap();
        assert!(create(&board, "acme-api", "").is_err());
        assert!(create(&board, "", "").is_err());
    }

    #[test]
    fn a_milestone_is_an_epic_unless_the_roadmap_uses_sprints() {
        let (_dir, board) = board();
        create(&board, "acme-api", "").unwrap();
        add_milestone(&board, &board, "acme-api", milestone("Beta", "", "")).unwrap();
        let group = board.group("Beta").unwrap();
        assert_eq!(group.kind, Kind::Epic);
        assert_eq!(group.description, "");

        set_details(&board, "acme-api", "", true).unwrap();
        let mut s1 = milestone("S1", "2026-10-01", "2026-10-11");
        s1.duration = 7;
        add_milestone(&board, &board, "acme-api", s1).unwrap();
        let group = board.group("S1").unwrap();
        assert!(group.is_sprint());
        assert_eq!(group.days, 7);
        assert_eq!(group.description, "What it has to do. And more.");
        add_milestone(
            &board,
            &board,
            "acme-api",
            milestone("S2", "2026-10-01", "2026-10-11"),
        )
        .unwrap();
        assert_eq!(board.group("S2").unwrap().days, 10);
    }

    fn group(board: &Board, name: &str, kind: Kind, days: u32) -> Group {
        let group = Group {
            dir: board.dir(Column::Backlog).join(name),
            name: name.to_string(),
            kind,
            description: "Its own words".to_string(),
            days,
            state: State::Pending,
        };
        board.create_group(&group).unwrap();
        group
    }

    fn ticket_in(board: &Board, group: &str) {
        let dir = board.dir(Column::Backlog).join(group);
        fs::write(
            dir.join("001-a-ticket.md"),
            "---\nid: \"001\"\ntitle: \"A ticket\"\n---\n",
        )
        .unwrap();
    }

    #[test]
    fn a_milestone_uses_an_epic_already_of_its_name_as_an_epic() {
        let (_dir, board) = board();
        create(&board, "acme-api", "").unwrap();
        set_details(&board, "acme-api", "", true).unwrap();
        group(&board, "Launch", Kind::Epic, 14);
        let (roadmap, warnings) =
            add_milestone(&board, &board, "acme-api", milestone("Launch", "", "")).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(
            roadmap.milestones[0].kind,
            Kind::Epic,
            "the epic's kind, not the roadmap's"
        );
        let kept = board.group("Launch").unwrap();
        assert_eq!(
            (kept.kind, kept.description.as_str()),
            (Kind::Epic, "Its own words")
        );
    }

    #[test]
    fn a_milestone_updates_a_sprint_of_its_name_that_hasnt_started() {
        let (_dir, board) = board();
        create(&board, "acme-api", "").unwrap();
        group(&board, "S3", Kind::Sprint, 14);
        let mut s3 = milestone("S3", "", "");
        s3.duration = 5;
        // An epics roadmap still takes the sprint as a sprint.
        let (roadmap, warnings) = add_milestone(&board, &board, "acme-api", s3).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(roadmap.milestones[0].kind, Kind::Sprint);
        let updated = board.group("S3").unwrap();
        assert_eq!(updated.days, 5);
        assert_eq!(updated.description, "What it has to do. And more.");
    }

    #[test]
    fn a_started_or_closed_sprint_is_used_as_it_is_with_a_warning() {
        let (_dir, board) = board();
        create(&board, "acme-api", "").unwrap();
        group(&board, "S4", Kind::Sprint, 14).start().unwrap();
        let mut closed = group(&board, "S5", Kind::Sprint, 14);
        closed.start().unwrap();
        closed.close().unwrap();
        for (name, word) in [("S4", "started"), ("S5", "closed")] {
            let mut m = milestone(name, "", "");
            m.duration = 3;
            let (_, warnings) = add_milestone(&board, &board, "acme-api", m).unwrap();
            assert_eq!(
                warnings,
                [format!(
                    "{name} is a {word} sprint, which can't be updated: the milestone uses it as it is"
                )]
            );
            let kept = board.group(name).unwrap();
            assert_eq!(
                (kept.days, kept.description.as_str()),
                (14, "Its own words")
            );
        }
    }

    #[test]
    fn deleting_a_milestone_removes_its_epic_only_when_empty_and_not_started() {
        let (_dir, board) = board();
        create(&board, "acme-api", "").unwrap();
        for name in ["Empty", "Busy"] {
            add_milestone(&board, &board, "acme-api", milestone(name, "", "")).unwrap();
        }
        ticket_in(&board, "Busy");
        group(&board, "Run", Kind::Sprint, 14).start().unwrap();
        add_milestone(&board, &board, "acme-api", milestone("Run", "", "")).unwrap();

        assert!(
            delete_milestone(&board, &board, "acme-api", "Empty")
                .unwrap()
                .is_empty()
        );
        assert!(board.group("Empty").is_err(), "an empty epic goes with it");
        assert_eq!(
            delete_milestone(&board, &board, "acme-api", "Busy").unwrap(),
            ["kept the epic Busy: it has 1 ticket"]
        );
        assert!(board.group("Busy").is_ok());
        assert_eq!(
            delete_milestone(&board, &board, "acme-api", "Run").unwrap(),
            ["kept the sprint Run: it has started"]
        );
        assert!(list(&board).unwrap()[0].milestones.is_empty());
        assert!(delete_milestone(&board, &board, "acme-api", "Nope").is_err());
    }

    #[test]
    fn deleting_a_roadmap_removes_its_file_and_its_empty_epics() {
        let (_dir, board) = board();
        create(&board, "acme-api", "").unwrap();
        for name in ["One", "Two"] {
            add_milestone(&board, &board, "acme-api", milestone(name, "", "")).unwrap();
        }
        ticket_in(&board, "Two");
        let notes = delete(&board, Some(&board), "acme-api").unwrap();
        assert_eq!(notes, ["kept the epic Two: it has 1 ticket"]);
        assert!(list(&board).unwrap().is_empty());
        assert!(board.group("One").is_err());
        assert!(board.group("Two").is_ok());
        assert!(
            delete(&board, Some(&board), "acme-api").is_err(),
            "already gone"
        );
    }

    #[test]
    fn the_epic_goes_on_the_target_board() {
        let (_a, store) = board();
        let (_b, target) = board();
        create(&store, "other", "").unwrap();
        add_milestone(&store, &target, "other", milestone("Beta", "", "")).unwrap();
        assert!(target.group("Beta").is_ok());
        assert!(store.groups().unwrap().is_empty());
        assert_eq!(list(&store).unwrap()[0].milestones.len(), 1);
    }

    #[test]
    fn a_refused_milestone_leaves_nothing_behind() {
        let (_dir, board) = board();
        create(&board, "acme-api", "").unwrap();
        let mut sevens = milestone("D", "", "");
        sevens.days_per_week = 9;
        let bad = [
            milestone("todo", "", ""),
            milestone("a/b", "", ""),
            milestone("", "", ""),
            milestone("S", "2026-10-10", "2026-10-01"),
            milestone("D", "", "next week"),
            sevens,
        ];
        for m in bad {
            assert!(add_milestone(&board, &board, "acme-api", m).is_err());
        }
        assert!(board.groups().unwrap().is_empty());
        assert!(list(&board).unwrap()[0].milestones.is_empty());
    }

    #[test]
    fn a_duplicate_milestone_is_refused() {
        let (_dir, board) = board();
        create(&board, "acme-api", "").unwrap();
        add_milestone(&board, &board, "acme-api", milestone("Beta", "", "")).unwrap();
        assert!(add_milestone(&board, &board, "acme-api", milestone("Beta", "", "")).is_err());
    }

    #[test]
    fn an_old_roadmap_can_be_adopted_by_a_board() {
        let (_dir, board) = board();
        fs::create_dir_all(folder(&board)).unwrap();
        fs::write(
            folder(&board).join("launch.md"),
            "---\nname: \"Launch\"\ndue: \"2026-12-01\"\n---\n\n## Beta\n- kind: epic\n\nText.\n",
        )
        .unwrap();
        let old = &list(&board).unwrap()[0];
        assert_eq!((old.name.as_str(), old.board.as_str()), ("Launch", ""));
        assert!(add_milestone(&board, &board, "launch", milestone("X", "", "")).is_err());
        adopt(&board, "launch", "acme-api").unwrap();
        assert_eq!(list(&board).unwrap()[0].board, "acme-api");
        assert!(adopt(&board, "launch", "acme-ui").is_err());
        assert!(create(&board, "acme-api", "").is_err());
    }

    #[test]
    fn roadmaps_are_not_tickets() {
        let (_dir, board) = board();
        create(&board, "acme-api", "").unwrap();
        assert!(board.tasks_in(Column::Backlog).unwrap().is_empty());
        assert!(board.groups().unwrap().is_empty());
    }

    #[test]
    fn working_days_count_the_first_days_of_each_week() {
        // 2026-10-12 is a Monday.
        assert_eq!(
            working_days(date!(2026 - 10 - 12), date!(2026 - 10 - 18), 5),
            5
        );
        assert_eq!(
            working_days(date!(2026 - 10 - 12), date!(2026 - 10 - 18), 4),
            4
        );
        assert_eq!(
            working_days(date!(2026 - 10 - 12), date!(2026 - 10 - 12), 5),
            1
        );
        assert_eq!(
            working_days(date!(2026 - 10 - 17), date!(2026 - 10 - 18), 5),
            0
        );
        assert_eq!(
            working_days(date!(2026 - 10 - 13), date!(2026 - 10 - 12), 5),
            0
        );
        assert_eq!(
            working_days(date!(2026 - 10 - 12), date!(2026 - 10 - 25), 0),
            10
        );
    }

    #[test]
    fn duration_is_given_or_calculated() {
        let mut m = milestone("A", "2026-10-12", "2026-10-23");
        assert_eq!(m.working_duration(), Some(10));
        m.duration = 3;
        assert_eq!(m.working_duration(), Some(3));
        assert_eq!(milestone("B", "", "2026-10-23").working_duration(), None);
    }

    #[test]
    fn remaining_counts_down_then_goes_negative() {
        let m = milestone("A", "", "2026-10-23");
        assert_eq!(m.remaining(date!(2026 - 10 - 19)), Some(5));
        assert_eq!(m.remaining(date!(2026 - 10 - 23)), Some(1));
        assert_eq!(m.remaining(date!(2026 - 10 - 26)), Some(-1));
        assert_eq!(
            milestone("B", "", "").remaining(date!(2026 - 10 - 19)),
            None
        );
    }

    #[test]
    fn late_is_overdue_or_short_of_time_for_the_unfinished_work() {
        let m = milestone("A", "2026-10-12", "2026-10-23"); // 10 working days
        // Plenty of time, nothing done.
        assert!(!m.late(date!(2026 - 10 - 12), 0, 4));
        // Five days left, half the work left.
        assert!(!m.late(date!(2026 - 10 - 19), 2, 4));
        // Five days left, nothing done: only five days for ten days of work.
        assert!(m.late(date!(2026 - 10 - 19), 0, 4));
        // Overdue.
        assert!(m.late(date!(2026 - 10 - 26), 3, 4));
        // Finished, even if overdue.
        assert!(!m.late(date!(2026 - 10 - 26), 4, 4));
        // No due date: nothing to be late against.
        assert!(!milestone("B", "", "").late(date!(2026 - 10 - 26), 0, 0));
    }

    #[test]
    fn a_roadmap_adds_up_its_milestones() {
        let mut a = milestone("A", "2026-10-12", "2026-10-23");
        a.duration = 0;
        let mut b = milestone("B", "", "2026-11-06");
        b.duration = 5;
        let roadmap = Roadmap {
            slug: "x".into(),
            name: "x".into(),
            board: "x".into(),
            goal: String::new(),
            sprints: false,
            milestones: vec![a, b],
        };
        let summary = roadmap.summary(date!(2026 - 10 - 12), &|_| (0, 0));
        assert_eq!(summary.duration, Some(15));
        // To the last due date, 2026-11-06 (a Friday): 4 full weeks.
        assert_eq!(summary.remaining, Some(20));
        assert!(!summary.late);
        let late = roadmap.summary(date!(2026 - 10 - 26), &|_| (0, 3));
        assert!(late.late);
    }
    #[test]
    fn a_milestones_dates_can_be_changed() {
        let (_dir, board) = board();
        create(&board, "acme-api", "").unwrap();
        add_milestone(
            &board,
            &board,
            "acme-api",
            milestone("Beta", "2026-10-01", "2026-11-01"),
        )
        .unwrap();
        let r = set_milestone_dates(
            &board,
            &board,
            "acme-api",
            "Beta",
            "2026-10-05",
            " 2026-11-20 ",
        )
        .unwrap();
        let m = &r.milestones[0];
        assert_eq!(
            (m.start.as_str(), m.due.as_str()),
            ("2026-10-05", "2026-11-20")
        );
        assert_eq!(list(&board).unwrap()[0], r, "written");
        let r = set_milestone_dates(&board, &board, "acme-api", "Beta", "", "").unwrap();
        assert_eq!(
            (r.milestones[0].start.as_str(), r.milestones[0].due.as_str()),
            ("", "")
        );
        assert!(
            set_milestone_dates(
                &board,
                &board,
                "acme-api",
                "Beta",
                "2026-11-02",
                "2026-11-01"
            )
            .is_err()
        );
        assert!(set_milestone_dates(&board, &board, "acme-api", "Beta", "soon", "").is_err());
        assert!(set_milestone_dates(&board, &board, "acme-api", "Gamma", "", "").is_err());
    }

    #[test]
    fn a_pending_sprint_follows_its_milestones_dates_and_a_started_one_does_not() {
        let (_dir, board) = board();
        create(&board, "acme-api", "").unwrap();
        set_details(&board, "acme-api", "", true).unwrap();
        add_milestone(
            &board,
            &board,
            "acme-api",
            milestone("S1", "2026-10-01", "2026-10-11"),
        )
        .unwrap();
        assert_eq!(board.group("S1").unwrap().days, 10);
        set_milestone_dates(&board, &board, "acme-api", "S1", "2026-10-01", "2026-10-15").unwrap();
        assert_eq!(board.group("S1").unwrap().days, 14);
        board.group("S1").unwrap().start().unwrap();
        set_milestone_dates(&board, &board, "acme-api", "S1", "2026-10-01", "2026-10-30").unwrap();
        assert_eq!(
            board.group("S1").unwrap().days,
            14,
            "a started sprint keeps its length"
        );
        // One with a duration of its own keeps that.
        let mut s2 = milestone("S2", "2026-11-01", "2026-11-11");
        s2.duration = 5;
        add_milestone(&board, &board, "acme-api", s2).unwrap();
        set_milestone_dates(&board, &board, "acme-api", "S2", "2026-11-01", "2026-11-30").unwrap();
        assert_eq!(board.group("S2").unwrap().days, 5);
    }
}
