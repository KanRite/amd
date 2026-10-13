//! Roadmaps (ticket 206): a named plan for a board, with a due date, made of
//! milestones.
//!
//! A roadmap is one markdown file, `roadmaps/<slug>.md`, in the tickets
//! repository beside `tasks/` (not inside it, so the ticket scan never sees
//! it). A milestone is a `## name` section holding its kind, dates and
//! description. Adding one also creates the epic — or, if asked, the sprint —
//! of the same name on the board, in the same change so one auto-commit
//! records both.
//!
//! ```text
//! ---
//! name: "Launch"
//! due: "2026-12-01"
//! ---
//!
//! ## Beta
//! - kind: epic
//! - start: 2026-10-01
//! - due: 2026-11-01
//!
//! What the beta has to do.
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use time::Date;
use time::macros::format_description;

use crate::board::Board;
use crate::group::{DEFAULT_DAYS, Group, Kind, State};

/// The folder inside the tickets repository that holds the roadmaps.
pub const FOLDER: &str = "roadmaps";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Milestone {
    pub name: String,
    pub description: String,
    /// An epic, or a sprint whose length is `start` to `due`.
    pub kind: Kind,
    /// `YYYY-MM-DD`; empty when none was given.
    pub start: String,
    /// The due-by date, `YYYY-MM-DD`; empty when none was given.
    pub due: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roadmap {
    /// The file name without `.md`.
    pub slug: String,
    pub name: String,
    /// The estimated due date, `YYYY-MM-DD`.
    pub due: String,
    pub milestones: Vec<Milestone>,
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

impl Roadmap {
    fn render(&self) -> String {
        let mut out = format!("---\nname: {:?}\ndue: {:?}\n---\n", self.name, self.due);
        for m in &self.milestones {
            out.push_str(&format!("\n## {}\n- kind: {}\n", m.name, m.kind.as_str()));
            if !m.start.is_empty() {
                out.push_str(&format!("- start: {}\n", m.start));
            }
            if !m.due.is_empty() {
                out.push_str(&format!("- due: {}\n", m.due));
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
            due: String::new(),
            milestones: Vec::new(),
        };
        let mut lines = text.lines();
        if lines.next().map(str::trim) == Some("---") {
            for line in lines.by_ref() {
                if line.trim() == "---" {
                    break;
                }
                if let Some((key, value)) = line.split_once(':') {
                    let value = value.trim().trim_matches('"').to_string();
                    match key.trim() {
                        "name" => roadmap.name = value,
                        "due" => roadmap.due = value,
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
                    _ => description.push(line),
                }
            }
        }
        finish(&mut roadmap, &mut description);
        roadmap
    }

    fn path(&self, board: &Board) -> PathBuf {
        folder(board).join(format!("{}.md", self.slug))
    }
}

/// Every roadmap of the board's repository, by name.
pub fn list(board: &Board) -> Result<Vec<Roadmap>> {
    let dir = folder(board);
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
    roadmaps.sort_by_key(|r| r.name.to_lowercase());
    Ok(roadmaps)
}

fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

/// Create a roadmap for `board`, whose epics its milestones will be.
pub fn create(board: &Board, name: &str, due: &str) -> Result<Roadmap> {
    let name = name.trim();
    check_line("roadmap", name)?;
    check_date(due)?;
    let roadmap = Roadmap {
        slug: slug(name),
        name: name.to_string(),
        due: due.to_string(),
        milestones: Vec::new(),
    };
    if roadmap.slug.is_empty() {
        bail!("'{name}' has no letters or digits to name a file with");
    }
    let path = roadmap.path(board);
    if path.exists() {
        bail!("a roadmap called {name} already exists");
    }
    write(board, &roadmap)?;
    Ok(roadmap)
}

fn write(board: &Board, roadmap: &Roadmap) -> Result<()> {
    let path = roadmap.path(board);
    fs::create_dir_all(folder(board))
        .with_context(|| format!("creating {}", folder(board).display()))?;
    fs::write(&path, roadmap.render()).with_context(|| format!("writing {}", path.display()))
}

/// Add a milestone to the roadmap `slug`, and create its epic (or sprint) on
/// `board`. All the
/// checks run before anything is written, so a refused milestone leaves no
/// epic behind and no half-written roadmap.
pub fn add_milestone(board: &Board, slug: &str, milestone: Milestone) -> Result<Roadmap> {
    let mut roadmap = list(board)?
        .into_iter()
        .find(|r| r.slug == slug)
        .ok_or_else(|| anyhow::anyhow!("no roadmap called {slug}"))?;
    let milestone = Milestone {
        name: milestone.name.trim().to_string(),
        description: milestone.description.trim().to_string(),
        ..milestone
    };
    check_line("milestone", &milestone.name)?;
    check_date(&milestone.start)?;
    check_date(&milestone.due)?;
    if milestone.description.lines().any(|l| l.starts_with("## ")) {
        bail!("a description can't have a line starting with '## '");
    }
    let days = sprint_days(&milestone.start, &milestone.due)?;
    if roadmap.milestones.iter().any(|m| m.name == milestone.name) {
        bail!(
            "{} already has a milestone {}",
            roadmap.name,
            milestone.name
        );
    }
    // The epic's name is the milestone's name; its description and dates stay
    // here. A sprint takes the description and the length.
    let sprint = milestone.kind == Kind::Sprint;
    board.create_group(&Group {
        dir: board
            .dir(crate::board::Column::Backlog)
            .join(&milestone.name),
        name: milestone.name.clone(),
        kind: milestone.kind,
        // `_group.md` keeps a description on one line.
        description: if sprint {
            milestone
                .description
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            String::new()
        },
        days,
        state: State::Pending,
    })?;
    roadmap.milestones.push(milestone);
    write(board, &roadmap)?;
    Ok(roadmap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Column;

    fn board() -> (tempfile::TempDir, Board) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("tasks");
        fs::create_dir_all(root.join("backlog")).unwrap();
        (dir, Board::at(root))
    }

    fn milestone(name: &str, kind: Kind, start: &str, due: &str) -> Milestone {
        Milestone {
            name: name.to_string(),
            description: "What it has to do.\n\nAnd more.".to_string(),
            kind,
            start: start.to_string(),
            due: due.to_string(),
        }
    }

    #[test]
    fn slugs_are_file_names() {
        assert_eq!(slug("Q4 Launch!"), "q4-launch");
        assert_eq!(slug("  --  "), "");
    }

    #[test]
    fn a_roadmap_and_its_milestones_round_trip() {
        let (_dir, board) = board();
        create(&board, "Launch", "2026-12-01").unwrap();
        add_milestone(
            &board,
            "launch",
            milestone("Beta", Kind::Epic, "", "2026-11-01"),
        )
        .unwrap();
        add_milestone(
            &board,
            "launch",
            milestone("Sprint 1", Kind::Sprint, "2026-10-01", "2026-10-15"),
        )
        .unwrap();
        let roadmaps = list(&board).unwrap();
        assert_eq!(roadmaps.len(), 1);
        let r = &roadmaps[0];
        assert_eq!((r.name.as_str(), r.due.as_str()), ("Launch", "2026-12-01"));
        assert_eq!(r.milestones.len(), 2);
        assert_eq!(r.milestones[0].name, "Beta");
        assert_eq!(r.milestones[0].due, "2026-11-01");
        assert_eq!(
            r.milestones[0].description,
            "What it has to do.\n\nAnd more."
        );
        assert_eq!(r.milestones[1].kind, Kind::Sprint);
        assert_eq!(r.milestones[1].start, "2026-10-01");
    }

    #[test]
    fn a_milestone_creates_an_epic_named_for_it_with_no_description() {
        let (_dir, board) = board();
        create(&board, "Launch", "").unwrap();
        add_milestone(&board, "launch", milestone("Beta", Kind::Epic, "", "")).unwrap();
        let group = board.group("Beta").unwrap();
        assert_eq!(group.kind, Kind::Epic);
        assert_eq!(group.description, "");
    }

    #[test]
    fn a_sprint_milestone_takes_the_description_and_the_length() {
        let (_dir, board) = board();
        create(&board, "Launch", "").unwrap();
        add_milestone(
            &board,
            "launch",
            milestone("S1", Kind::Sprint, "2026-10-01", "2026-10-11"),
        )
        .unwrap();
        let group = board.group("S1").unwrap();
        assert!(group.is_sprint());
        assert_eq!(group.days, 10);
        assert_eq!(group.description, "What it has to do. And more.");
    }

    #[test]
    fn a_refused_milestone_leaves_nothing_behind() {
        let (_dir, board) = board();
        create(&board, "Launch", "").unwrap();
        let bad = [
            milestone("todo", Kind::Epic, "", ""),
            milestone("a/b", Kind::Epic, "", ""),
            milestone("S", Kind::Sprint, "2026-10-10", "2026-10-01"),
            milestone("D", Kind::Epic, "", "next week"),
        ];
        for m in bad {
            assert!(add_milestone(&board, "launch", m).is_err());
        }
        assert!(board.groups().unwrap().is_empty());
        assert!(list(&board).unwrap()[0].milestones.is_empty());
    }

    #[test]
    fn a_duplicate_milestone_or_roadmap_is_refused() {
        let (_dir, board) = board();
        create(&board, "Launch", "").unwrap();
        assert!(create(&board, "launch", "").is_err());
        add_milestone(&board, "launch", milestone("Beta", Kind::Epic, "", "")).unwrap();
        assert!(add_milestone(&board, "launch", milestone("Beta", Kind::Epic, "", "")).is_err());
    }

    #[test]
    fn roadmaps_are_not_tickets() {
        let (_dir, board) = board();
        create(&board, "Launch", "").unwrap();
        assert!(board.tasks_in(Column::Backlog).unwrap().is_empty());
        assert!(board.groups().unwrap().is_empty());
    }

    #[test]
    fn a_bad_roadmap_is_refused() {
        let (_dir, board) = board();
        assert!(create(&board, "", "").is_err());
        assert!(create(&board, "!!!", "").is_err());
        assert!(create(&board, "Launch", "soon").is_err());
    }
}
