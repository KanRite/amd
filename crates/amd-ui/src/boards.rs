//! The project's boards as amd-lib reads them, turned into what the window
//! shows ([`amd_ui_model`]'s types), and back again: a [`Place`] and a file
//! name from the window become the [`Task`] amd-lib acts on.
//!
//! Nothing here writes. Every change is an amd-lib call in `lib.rs`.

use std::path::Path;
use std::sync::OnceLock;

use agile_md::board::{Board, Column};
use agile_md::board_settings;
use agile_md::group::{GROUP_FILE, State};
use agile_md::model::load_repos;
use agile_md::task::Task;
use amd_ui_model::epic::{GroupKind, Place};
use amd_ui_model::wip::WipLimits;
use amd_ui_model::{self as ui, BoardColumn, Epic, Lock, RepoInfo, TicketSummary};
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};

pub type Result<T> = std::result::Result<T, String>;

/// An amd-lib error as the window shows it: the whole chain, on one line.
pub fn err(error: anyhow::Error) -> String {
    format!("{error:#}")
}

/// This machine's offset from UTC, read once at start-up while the process
/// is still single-threaded — the only time `time` will tell us on Linux.
static LOCAL_OFFSET: OnceLock<UtcOffset> = OnceLock::new();

pub fn init_local_offset() {
    LOCAL_OFFSET.get_or_init(|| UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC));
}

/// The project the board shows: its own repository first, then every other
/// repository it declares that is cloned here (`amd register`).
pub struct Project {
    pub repos: Vec<(String, Board)>,
    /// Declared repositories with no board on this machine.
    pub unregistered: Vec<String>,
    /// The git identity locks are compared against.
    pub identity: Option<String>,
}

impl Project {
    /// The project around the current directory, as `amd` finds it.
    pub fn open() -> Result<Project> {
        let found = load_repos().map_err(err)?;
        Ok(Project {
            repos: found.repos.into_iter().map(|r| (r.name, r.board)).collect(),
            unregistered: found.unregistered,
            identity: found.identity,
        })
    }

    /// The board of the repository called `repo`; empty means the project's
    /// own.
    pub fn board(&self, repo: &str) -> Result<&Board> {
        if repo.is_empty() {
            return Ok(&self.repos[0].1);
        }
        self.repos
            .iter()
            .find(|(name, _)| name == repo)
            .map(|(_, board)| board)
            .ok_or_else(|| format!("The {repo} repository has no board on this machine."))
    }

    /// Everything the window draws: every repository's tickets and epics,
    /// merged, each labelled with its repository.
    pub fn summary(&self) -> Result<ui::Board> {
        let mut columns: Vec<BoardColumn> = ui::Column::ALL
            .into_iter()
            .map(|column| BoardColumn {
                column,
                tickets: Vec::new(),
            })
            .collect();
        let mut epics = Vec::new();
        let mut repos = Vec::new();
        for (name, board) in &self.repos {
            let settings = board_settings::load(board);
            repos.push(RepoInfo {
                name: name.clone(),
                available: true,
                wip: WipLimits {
                    doing: settings.wip_limit(Column::Doing).unwrap_or(0),
                },
                lease_hours: settings.lease_hours().round().max(1.0) as u32,
            });
            for column in Column::ALL {
                for task in board.tasks_in(column).map_err(err)? {
                    let summary = self.summary_of(&task, name);
                    if task.epic.is_none() {
                        columns[column.index()].tickets.push(summary);
                    }
                }
            }
            for group in board.groups().map_err(err)? {
                let tickets = board
                    .tasks_in(Column::Backlog)
                    .map_err(err)?
                    .into_iter()
                    .filter(|t| t.epic.as_deref() == Some(group.name.as_str()))
                    .map(|t| self.summary_of(&t, name))
                    .collect();
                epics.push(Epic {
                    repo: name.clone(),
                    kind: if group.is_sprint() {
                        GroupKind::Sprint
                    } else {
                        GroupKind::Epic
                    },
                    days: if group.is_sprint() { group.days } else { 0 },
                    started: group.is_sprint() && group.state == State::Started,
                    name: group.name,
                    tickets,
                });
            }
        }
        repos.extend(self.unregistered.iter().map(|name| RepoInfo {
            name: name.clone(),
            available: false,
            wip: WipLimits::default(),
            lease_hours: ui::DEFAULT_LEASE_HOURS,
        }));
        for column in &mut columns {
            column.tickets.sort_by(ui::ticket::default_order);
        }
        for epic in &mut epics {
            epic.tickets.sort_by(ui::ticket::default_order);
        }
        let (own_name, own_board) = &self.repos[0];
        let project = board_settings::load(own_board)
            .project()
            .map(str::to_string)
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| own_name.clone());
        Ok(ui::Board {
            project,
            tasks_dir: self.repos[0].1.root.display().to_string(),
            repos,
            columns,
            epics,
        })
    }

    /// A ticket as a card on the board.
    pub fn summary_of(&self, task: &Task, repo: &str) -> TicketSummary {
        TicketSummary {
            repo: repo.to_string(),
            file: task.file_name(),
            column: from_column(task.column),
            id: task.id_display(),
            title: task.title(),
            assignee: task.assignee().unwrap_or_default(),
            tags: task.tags(),
            priority: task.priority(),
            created: task.meta("created").unwrap_or_default(),
            epic: task.epic.clone(),
            text: task.body().unwrap_or_default().trim().to_string(),
            epic_label: task.epic_meta().unwrap_or_default(),
            points: task.points().unwrap_or_default(),
            lock: self.lock_of(task),
        }
    }

    /// The lock on a ticket, while one is in force.
    pub fn lock_of(&self, task: &Task) -> Option<Lock> {
        if !task.lease_active() {
            return None;
        }
        let by = task.locked_by()?;
        let until = task.locked_until()?;
        Some(Lock {
            mine: self.identity.as_deref() == Some(by.as_str()),
            until: until.format(&Rfc3339).unwrap_or_default(),
            until_local: local_time(until),
            by,
        })
    }
}

/// When a lock lapses, as the board shows it: "Sun 4 Oct 18:30".
fn local_time(at: OffsetDateTime) -> String {
    let offset = LOCAL_OFFSET.get().copied().unwrap_or(UtcOffset::UTC);
    let format = time::macros::format_description!(
        "[weekday repr:short] [day padding:none] [month repr:short] [hour]:[minute]"
    );
    at.to_offset(offset).format(format).unwrap_or_default()
}

pub fn to_column(column: ui::Column) -> Column {
    match column {
        ui::Column::Backlog => Column::Backlog,
        ui::Column::Todo => Column::Todo,
        ui::Column::Doing => Column::Doing,
        ui::Column::Done => Column::Done,
    }
}

pub fn from_column(column: Column) -> ui::Column {
    match column {
        Column::Backlog => ui::Column::Backlog,
        Column::Todo => ui::Column::Todo,
        Column::Doing => ui::Column::Doing,
        Column::Done => ui::Column::Done,
    }
}

/// The ticket at `place` called `file`. Only a plain `*.md` name directly
/// in the place's directory is accepted, and the epic must exist, so the
/// window can never name an arbitrary file.
pub fn task_at(board: &Board, place: &Place, file: &str) -> Result<Task> {
    let plain = file.ends_with(".md")
        && file != GROUP_FILE
        && !file.starts_with('.')
        && Path::new(file).file_name().and_then(|n| n.to_str()) == Some(file);
    if !plain {
        return Err(format!("not a ticket file name: {file:?}"));
    }
    let column = to_column(place.column);
    let mut dir = board.dir(column);
    if let Some(epic) = &place.epic {
        if column != Column::Backlog || board.group(epic).is_err() {
            return Err(format!("there is no epic called {epic:?}"));
        }
        dir = dir.join(epic);
    }
    let path = dir.join(file);
    let mut task = Task::from_path(&path, column)
        .filter(|_| path.is_file())
        .ok_or_else(|| format!("no ticket {file} there any more — reload the board"))?;
    task.epic = place.epic.clone();
    Ok(task)
}
