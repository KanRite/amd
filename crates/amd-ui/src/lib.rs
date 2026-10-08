//! amdui's backend: the Tauri commands the board's window calls. Each one
//! reads the project the way `amd` does and makes its change through
//! amd-lib — the same moves, leases, WIP limit, groups, templates and
//! auto-commit as the command line (ticket 084, docs/adr/0004). There is no
//! ticket, git or settings logic here of its own.

mod boards;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agile_md::board::{Board, Column};
use agile_md::group::{DEFAULT_DAYS, Group, Kind, State};
use agile_md::settings::Config;
use agile_md::task::{ContentHash, Task};
use agile_md::templates::{DEFAULT_TEMPLATE, TEMPLATE_SUFFIX, Templates, example_context};
use agile_md::transition::{self, Event, Outcome, TicketFields};
use agile_md::{board_settings, current_project, settings};
use amd_ui_model::appearance::Appearance;
use amd_ui_model::epic::Place;
use amd_ui_model::wip::WipLimits;
use amd_ui_model::{Created, SyncReport, TicketDocument, TicketTemplate};
use tauri::State as Tauri;

use boards::{Project, Result, err, task_at, to_column};

/// The global settings key remembering the board shown last, for a launch
/// from somewhere that isn't inside a project.
const LAST_BOARD: &str = "ui.board";

/// Board changes one at a time: two drags in quick succession must not
/// interleave their moves and commits.
#[derive(Default)]
struct AppState {
    writing: Arc<Mutex<()>>,
}

/// Runs a change against the project off the async runtime, alone.
async fn change<T: Send + 'static>(
    state: &Tauri<'_, AppState>,
    f: impl FnOnce(&Project) -> Result<T> + Send + 'static,
) -> Result<T> {
    let writing = Arc::clone(&state.writing);
    blocking(move || {
        let _guard = writing
            .lock()
            .map_err(|_| "the board is in a bad state — restart amdui".to_string())?;
        // In step with the remote before any change (ticket 102): never
        // write on a stale clone. A clone that can't be brought up to date
        // refuses the change; no network only warns, so is ignored here.
        for (_, board) in &Project::open()?.repos {
            agile_md::sync::catch_up(board, agile_md::sync::Purpose::Write).map_err(err)?;
            // Settings still in a documents repository come across, once
            // (ticket 111).
            transition::with_auto_commit(board, "settings", "migrate", "from documents", || {
                board_settings::migrate_from_documents(board).map(|_| ())
            })
            .map_err(err)?;
        }
        f(&Project::open()?)
    })
    .await
}

/// Runs blocking work (files, git) off the async runtime, so a slow `git
/// push` never freezes the window.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("background task failed: {e}"))?
}

/// What git did, as the header reports it. `message` is the commit message
/// amd-lib writes (`doing: 026 "title"`); a warning is a commit or push that
/// failed without undoing the change.
fn report(board: &Board, message: String, warning: Option<String>) -> SyncReport {
    let auto = board_settings::load(board).auto_commit();
    let done = auto && warning.is_none();
    SyncReport {
        message,
        committed: done,
        pushed: done,
        warning,
    }
}

/// [`report`] for an amd-lib transition, whose warnings come as events.
fn report_outcome(board: &Board, message: String, outcome: &Outcome) -> SyncReport {
    let warnings: Vec<&str> = outcome
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Warn(w) => Some(w.as_str()),
            Event::Info(_) => None,
        })
        .collect();
    let warning = (!warnings.is_empty()).then(|| warnings.join("; "));
    report(board, message, warning)
}

fn message(verb: &str, task: &Task) -> String {
    format!("{verb}: {} \"{}\"", task.id_display(), task.title())
}

/// Runs a change through amd-lib's auto-commit and reports it.
fn committed(
    board: &Board,
    verb: &str,
    subject: &str,
    detail: &str,
    f: impl FnOnce() -> anyhow::Result<()>,
) -> Result<SyncReport> {
    let ((), warning) =
        transition::with_auto_commit(board, verb, subject, detail, f).map_err(err)?;
    // The status line says what the commit says, empty detail and all.
    Ok(report(
        board,
        transition::commit_message(verb, subject, detail),
        warning,
    ))
}

#[tauri::command]
async fn load_board(window: tauri::WebviewWindow) -> Result<amd_ui_model::Board> {
    titled(&window, blocking(|| Project::open()?.summary()).await)
}

/// A window's title for the project it shows: `<project> — amdui`, so two
/// boards' windows can be told apart in the window switcher (ticket 116).
fn window_title(project: &str) -> String {
    match project.trim() {
        "" => "amdui".to_string(),
        project => format!("{project} — amdui"),
    }
}

/// Titles `window` after the board it is about to show. Every command that
/// hands the page a whole board goes through this, so the title follows a
/// change of project too. Only cosmetic, so a failure to set it is ignored.
fn titled(
    window: &tauri::WebviewWindow,
    board: Result<amd_ui_model::Board>,
) -> Result<amd_ui_model::Board> {
    if let Ok(board) = &board {
        let _ = window.set_title(&window_title(&board.project));
    }
    board
}

/// What's on disk for the project on screen, summarised (ticket 091): the
/// page asks every second and reloads only when this changes, so the board
/// follows changes made anywhere — a terminal, an editor, a pull.
#[tauri::command]
async fn board_fingerprint() -> Result<String> {
    blocking(|| {
        let project = Project::open()?;
        let mut hasher = std::hash::DefaultHasher::new();
        for (name, board) in &project.repos {
            std::hash::Hash::hash(&(name, agile_md::model::fingerprint(board)), &mut hasher);
        }
        Ok(format!("{:016x}", std::hash::Hasher::finish(&hasher)))
    })
    .await
}

/// Brings every repository on screen up to date with its remote, the way a
/// listing in the CLI does (ticket 102). The page calls it every 30 seconds;
/// whatever it pulls in changes the fingerprint, and the board refreshes.
/// Warnings (offline, a diverged clone) are returned, not raised.
#[tauri::command]
async fn sync_board(state: Tauri<'_, AppState>) -> Result<Vec<String>> {
    let writing = Arc::clone(&state.writing);
    blocking(move || {
        let _guard = writing
            .lock()
            .map_err(|_| "the board is in a bad state — restart amdui".to_string())?;
        let mut warnings = Vec::new();
        for (_, board) in &Project::open()?.repos {
            match agile_md::sync::catch_up(board, agile_md::sync::Purpose::Read) {
                Ok(events) => warnings.extend(events.into_iter().filter_map(|event| match event {
                    Event::Warn(text) => Some(text),
                    Event::Info(_) => None,
                })),
                Err(e) => warnings.push(format!("{e:#}")),
            }
        }
        Ok(warnings)
    })
    .await
}

/// The ticket's current version on disk, as `open_ticket` reports it, or
/// `None` once it's no longer at `place` (moved or removed) — how an open
/// editor learns its ticket changed underneath it.
#[tauri::command]
async fn ticket_version(place: Place, file: String) -> Result<Option<String>> {
    blocking(move || {
        let project = Project::open()?;
        let board = project.board(&place.repo)?;
        Ok(task_at(board, &place, &file)
            .ok()
            .and_then(|task| task.hash().ok())
            .map(|hash| hash.to_string()))
    })
    .await
}

#[tauri::command]
async fn open_ticket(place: Place, file: String) -> Result<TicketDocument> {
    blocking(move || {
        let project = Project::open()?;
        let board = project.board(&place.repo)?;
        let task = task_at(board, &place, &file)?;
        let content = std::fs::read_to_string(&task.path).map_err(|e| e.to_string())?;
        Ok(TicketDocument {
            repo: place.repo.clone(),
            file,
            column: place.column,
            epic: place.epic.clone(),
            lock: project.lock_of(&task),
            lapsed: project.lapsed_of(&task),
            version: task.hash().map_err(err)?.to_string(),
            content,
        })
    })
    .await
}

/// Saves the editor's text through amd-lib (`Task::set_text`): refused if
/// the ticket changed since it was opened, or is locked by someone else.
#[tauri::command]
async fn save_ticket(
    state: Tauri<'_, AppState>,
    place: Place,
    file: String,
    content: String,
    version: String,
) -> Result<SyncReport> {
    change(&state, move |project| {
        let board = project.board(&place.repo)?;
        let task = task_at(board, &place, &file)?;
        let expected: ContentHash = version
            .parse()
            .map_err(|_| "the editor lost track of this ticket — reopen it".to_string())?;
        // The project's tag list decides (ticket 122): an unlisted tag is
        // refused when only listed tags are allowed. Saved in uppercase.
        let tags = amd_ui_model::parse_frontmatter(&content).tags;
        board_settings::load(&project.repos[0].1)
            .check_tags(&tags)
            .map_err(err)?;
        let outcome = transition::save_text(board, &task, expected, &content).map_err(err)?;
        Ok(report_outcome(board, message("edit", &task), &outcome))
    })
    .await
}

#[tauri::command]
async fn move_ticket(
    state: Tauri<'_, AppState>,
    file: String,
    from: Place,
    to: Place,
    force: Option<bool>,
) -> Result<SyncReport> {
    change(&state, move |project| {
        move_in(project, &file, &from, &to, force.unwrap_or(false))
    })
    .await
}

/// A drag on the board, as amd-lib moves: into an epic or sprint is
/// `set_epic` (from todo, back to the backlog first); out of one to the
/// loose backlog is `set_epic(None)`; anything else is a column move, which
/// takes the lease on entering todo or doing. The window's own `Place`
/// (a column, or an epic in the backlog) maps onto these directly.
fn move_in(
    project: &Project,
    file: &str,
    from: &Place,
    to: &Place,
    force: bool,
) -> Result<SyncReport> {
    if from.repo != to.repo {
        return Err("A ticket can't move between repositories.".into());
    }
    let board = project.board(&from.repo)?;
    let task = task_at(board, from, file)?;
    let (id, title) = (task.id_display(), task.title());
    match (&from.epic, &to.epic, to.column) {
        (_, Some(epic), _) => committed(board, "epic", &id, &title, || {
            let task = if task.column == Column::Todo {
                let dest = board.move_task(&task, Column::Backlog, force)?;
                let mut back = Task::from_path(&dest, Column::Backlog)
                    .ok_or_else(|| anyhow::anyhow!("{} is not a ticket", dest.display()))?;
                back.epic = dest
                    .parent()
                    .filter(|dir| *dir != board.dir(Column::Backlog))
                    .and_then(|dir| dir.file_name())
                    .map(|name| name.to_string_lossy().into_owned());
                back
            } else {
                task
            };
            board.set_epic(&task, Some(epic)).map(|_| ())
        }),
        (Some(_), None, amd_ui_model::Column::Backlog) => {
            committed(board, "backlog", &id, &title, || {
                board.set_epic(&task, None).map(|_| ())
            })
        }
        _ => {
            let outcome =
                transition::move_to(board, &task, to_column(to.column), force).map_err(err)?;
            Ok(report_outcome(
                board,
                format!("{}: {id} \"{title}\"", to.column.dir_name()),
                &outcome,
            ))
        }
    }
}

/// Creates a ticket in `repo`'s backlog, the way `amd new` does.
#[tauri::command]
async fn create_ticket(
    state: Tauri<'_, AppState>,
    title: String,
    repo: Option<String>,
    repository: Option<String>,
) -> Result<Created> {
    change(&state, move |project| {
        let repo = repo.unwrap_or_default();
        let board = project.board(&repo)?;
        let fields = TicketFields {
            title: title.clone(),
            assignee: String::new(),
            // What it's about: the repository being filtered to, if any.
            repository,
            points: String::new(),
            epic: None,
            tags: Vec::new(),
            branch_type: String::new(),
            parent: String::new(),
            related: Vec::new(),
            body: String::new(),
        };
        let outcome = transition::create_ticket(board, &fields).map_err(err)?;
        let dest = outcome
            .dest
            .clone()
            .ok_or("the ticket was created but not found")?;
        let task = Task::from_path(&dest, Column::Backlog).ok_or("the new ticket can't be read")?;
        let name = if repo.is_empty() {
            project.repos[0].0.clone()
        } else {
            repo
        };
        let sync = report_outcome(board, message("new", &task), &outcome);
        Ok(Created {
            doc: TicketDocument {
                repo: name,
                file: task.file_name(),
                column: amd_ui_model::Column::Backlog,
                epic: None,
                lock: None,
                lapsed: None,
                version: task.hash().map_err(err)?.to_string(),
                content: std::fs::read_to_string(&task.path).map_err(|e| e.to_string())?,
            },
            sync,
        })
    })
    .await
}

fn new_group(board: &Board, name: &str, kind: Kind, days: u32) -> Result<SyncReport> {
    let name = name.trim().to_string();
    let verb = match kind {
        Kind::Epic => "group-epic",
        Kind::Sprint => "group-sprint",
    };
    committed(board, verb, &name, "", || {
        board.create_group(&Group {
            dir: board.dir(Column::Backlog).join(&name),
            name: name.clone(),
            kind,
            description: String::new(),
            days,
            state: State::Pending,
        })
    })
}

/// Creates an epic in `repo`'s backlog (`amd group epic`).
#[tauri::command]
async fn create_epic(
    state: Tauri<'_, AppState>,
    name: String,
    repo: Option<String>,
) -> Result<SyncReport> {
    change(&state, move |p| {
        new_group(
            p.board(&repo.unwrap_or_default())?,
            &name,
            Kind::Epic,
            DEFAULT_DAYS,
        )
    })
    .await
}

/// Creates a pending sprint in `repo`'s backlog (`amd group sprint`).
#[tauri::command]
async fn create_sprint(
    state: Tauri<'_, AppState>,
    name: String,
    days: u32,
    repo: Option<String>,
) -> Result<SyncReport> {
    change(&state, move |p| {
        new_group(
            p.board(&repo.unwrap_or_default())?,
            &name,
            Kind::Sprint,
            days,
        )
    })
    .await
}

/// Starts a sprint (`amd group start`): no more tickets in or back out.
#[tauri::command]
async fn start_sprint(
    state: Tauri<'_, AppState>,
    repo: Option<String>,
    name: String,
) -> Result<SyncReport> {
    change(&state, move |p| {
        let board = p.board(&repo.unwrap_or_default())?;
        committed(board, "group-start", &name, "", || {
            board.group(&name)?.start()
        })
    })
    .await
}

/// Renames an epic or sprint everywhere it is recorded (`amd group rename`).
#[tauri::command]
async fn rename_epic(
    state: Tauri<'_, AppState>,
    repo: Option<String>,
    from: String,
    to: String,
) -> Result<SyncReport> {
    change(&state, move |p| {
        let board = p.board(&repo.unwrap_or_default())?;
        committed(board, "group-rename", &from, to.trim(), || {
            board.rename_group(&from, &to).map(|_| ())
        })
    })
    .await
}

/// Takes a ticket off the board into the archive (`amd abandon`).
#[tauri::command]
async fn archive_ticket(
    state: Tauri<'_, AppState>,
    place: Place,
    file: String,
) -> Result<SyncReport> {
    change(&state, move |p| {
        let board = p.board(&place.repo)?;
        let task = task_at(board, &place, &file)?;
        let (id, title) = (task.id_display(), task.title());
        committed(board, "abandon", &id, &title, || {
            board.archive(&task).map(|_| ())
        })
    })
    .await
}

/// Locks a ticket for the user (`amd lock`), or renews their own lock.
#[tauri::command]
async fn lock_ticket(state: Tauri<'_, AppState>, place: Place, file: String) -> Result<SyncReport> {
    change(&state, move |p| {
        let board = p.board(&place.repo)?;
        let task = task_at(board, &place, &file)?;
        let hours = board_settings::load(board).lease_hours();
        let (id, title) = (task.id_display(), task.title());
        committed(board, "lock", &id, &title, || {
            task.take_lease(task.hash()?, hours)
        })
    })
    .await
}

/// Releases the user's own lock, or with `force` breaks someone else's
/// (`amd unlock [--force]`).
#[tauri::command]
async fn unlock_ticket(
    state: Tauri<'_, AppState>,
    place: Place,
    file: String,
    force: Option<bool>,
) -> Result<SyncReport> {
    change(&state, move |p| {
        let board = p.board(&place.repo)?;
        let task = task_at(board, &place, &file)?;
        let (id, title) = (task.id_display(), task.title());
        committed(board, "unlock", &id, &title, || {
            if force.unwrap_or(false) {
                task.break_lease(task.hash()?)
            } else {
                task.release_lease(task.hash()?)
            }
        })
    })
    .await
}

/// Saves a board setting through amd-lib, committed and pushed like any
/// change, since every setting lives in the tickets repository (ticket 111).
fn save_setting(
    board: &Board,
    what: &str,
    change: impl FnOnce(&mut board_settings::BoardSettings) -> anyhow::Result<()>,
) -> Result<SyncReport> {
    let mut settings = board_settings::load(board);
    change(&mut settings).map_err(err)?;
    committed(board, "settings", what, "", || settings.save(board))
}

/// Adds a tag to the project's list, in UPPERCASE (ticket 122).
#[tauri::command]
async fn add_tag(state: Tauri<'_, AppState>, tag: String) -> Result<SyncReport> {
    change(&state, move |p| {
        let board = &p.repos[0].1;
        let name = agile_md::task::normalize_tag(&tag);
        let mut settings = board_settings::load(board);
        settings.add_tag(&tag).map_err(err)?;
        committed(board, "tags", "add", &name, || settings.save(board))
    })
    .await
}

/// Takes an unused tag off the project's list: refused while any ticket on
/// the project's boards carries it (ticket 122).
#[tauri::command]
async fn remove_tag(state: Tauri<'_, AppState>, tag: String) -> Result<SyncReport> {
    change(&state, move |p| {
        let board = &p.repos[0].1;
        let boards = p.boards();
        let name = agile_md::task::normalize_tag(&tag);
        committed(board, "tags", "remove", &name, || {
            agile_md::model::remove_unused_tag(board, &boards, &tag).map(|_| ())
        })
    })
    .await
}

/// Whether only listed tags may be put on a ticket (ticket 122).
#[tauri::command]
async fn set_tags_only(state: Tauri<'_, AppState>, on: bool) -> Result<SyncReport> {
    change(&state, move |p| {
        let board = &p.repos[0].1;
        save_setting(
            board,
            &format!("tags-only {}", if on { "on" } else { "off" }),
            |s| {
                s.set_only_listed_tags(on);
                Ok(())
            },
        )
    })
    .await
}

/// Sets `repo`'s WIP limit on doing (0 turns it off).
#[tauri::command]
async fn set_wip_limits(
    state: Tauri<'_, AppState>,
    limits: WipLimits,
    repo: Option<String>,
) -> Result<SyncReport> {
    change(&state, move |p| {
        let board = p.board(&repo.unwrap_or_default())?;
        save_setting(board, &format!("wip-limit {}", limits.doing), |s| {
            s.set_wip_limit(Column::Doing, Some(limits.doing))
        })
    })
    .await
}

/// Sets how long a lock lasts on `repo`'s board.
#[tauri::command]
async fn set_lease_hours(
    state: Tauri<'_, AppState>,
    hours: u32,
    repo: Option<String>,
) -> Result<SyncReport> {
    change(&state, move |p| {
        let board = p.board(&repo.unwrap_or_default())?;
        save_setting(board, &format!("lease-hours {hours}"), |s| {
            s.set_lease_hours(f64::from(hours))
        })
    })
    .await
}

/// Whether the project's board commits and pushes each change (`commit`).
#[tauri::command]
async fn auto_commit() -> Result<bool> {
    blocking(|| {
        let project = Project::open()?;
        Ok(board_settings::load(project.board("")?).auto_commit())
    })
    .await
}

/// Turns auto-commit on or off for the project's board.
#[tauri::command]
async fn set_auto_commit(state: Tauri<'_, AppState>, on: bool) -> Result<bool> {
    change(&state, move |p| {
        let board = p.board("")?;
        let what = if on { "commit on" } else { "commit off" };
        save_setting(board, what, |s| {
            s.set_auto_commit(on);
            Ok(())
        })?;
        Ok(board_settings::load(board).auto_commit())
    })
    .await
}

/// The user's colour scheme, from agile-md's own settings on this machine.
#[tauri::command]
fn appearance() -> Appearance {
    let config = Config::load();
    Appearance::from_keys(
        config.get("ui.theme").unwrap_or_default(),
        config.get("ui.accent").unwrap_or_default(),
    )
}

/// Saves the user's colour scheme, so it is used again next time.
#[tauri::command]
fn set_appearance(appearance: Appearance) -> Result<Appearance> {
    let mut config = Config::load();
    config.set("ui.theme", appearance.theme.key());
    config.set("ui.accent", appearance.accent.key());
    config.save().map_err(err)?;
    Ok(appearance)
}

fn template_file(board: &Board) -> PathBuf {
    board
        .templates_dir()
        .join(format!("{DEFAULT_TEMPLATE}{TEMPLATE_SUFFIX}"))
}

fn template_of(board: &Board) -> Result<TicketTemplate> {
    let path = template_file(board);
    Ok(TicketTemplate {
        source: Templates::load(board)
            .and_then(|t| t.source_text(DEFAULT_TEMPLATE))
            .map_err(err)?,
        customised: path.is_file(),
        path: path.display().to_string(),
    })
}

/// `source` rendered for a made-up ticket, or why it can't be.
fn preview(board: &Board, source: &str) -> Result<String> {
    Templates::load(board)
        .and_then(|t| t.with_source(DEFAULT_TEMPLATE, source))
        .and_then(|t| t.render(DEFAULT_TEMPLATE, example_context(board)))
        .map_err(err)
}

#[tauri::command]
async fn get_template() -> Result<TicketTemplate> {
    blocking(|| template_of(Project::open()?.board("")?)).await
}

#[tauri::command]
async fn preview_template(source: String) -> Result<String> {
    blocking(move || preview(Project::open()?.board("")?, &source)).await
}

/// Saves the board's own ticket template (what `amd templates eject` makes),
/// once it renders.
#[tauri::command]
async fn save_template(state: Tauri<'_, AppState>, source: String) -> Result<TicketTemplate> {
    change(&state, move |p| {
        let board = p.board("")?;
        preview(board, &source)?;
        let path = template_file(board);
        committed(board, "template", DEFAULT_TEMPLATE, "", || {
            std::fs::create_dir_all(board.templates_dir())?;
            std::fs::write(&path, &source)?;
            Ok(())
        })?;
        template_of(board)
    })
    .await
}

/// Goes back to the built-in template by removing the board's own.
#[tauri::command]
async fn reset_template(state: Tauri<'_, AppState>) -> Result<TicketTemplate> {
    change(&state, move |p| {
        let board = p.board("")?;
        let path = template_file(board);
        if path.is_file() {
            committed(board, "template", DEFAULT_TEMPLATE, "built-in", || {
                std::fs::remove_file(&path)?;
                Ok(())
            })?;
        }
        template_of(board)
    })
    .await
}

/// Where the board on screen lives.
#[tauri::command]
async fn tasks_dir() -> Result<Option<String>> {
    blocking(|| {
        Ok(Project::open()
            .ok()
            .map(|p| p.repos[0].1.root.display().to_string()))
    })
    .await
}

/// Shows the project around `path` instead (a tickets repository, its tasks
/// directory, or the folder holding a project's repositories), and
/// remembers it for a launch from outside any project.
#[tauri::command]
async fn set_tasks_dir(
    window: tauri::WebviewWindow,
    state: Tauri<'_, AppState>,
    path: String,
) -> Result<amd_ui_model::Board> {
    let writing = Arc::clone(&state.writing);
    let board = blocking(move || {
        let _guard = writing
            .lock()
            .map_err(|_| "the board is in a bad state — restart amdui".to_string())?;
        show_from(Path::new(path.trim()))
    })
    .await;
    titled(&window, board)
}

/// Switches to another project in the same folder (ticket 093): records
/// it in that folder's `.current-project` — the same file `amd project use`
/// writes — and shows it. The window then runs from the folder itself, so
/// the file is what decides, not whichever tickets repository it was
/// launched inside.
#[tauri::command]
async fn use_project(
    window: tauri::WebviewWindow,
    state: Tauri<'_, AppState>,
    name: String,
) -> Result<amd_ui_model::Board> {
    let writing = Arc::clone(&state.writing);
    let board = blocking(move || {
        let _guard = writing
            .lock()
            .map_err(|_| "the board is in a bad state — restart amdui".to_string())?;
        let open = Project::open()?;
        let dir = open.repos[0]
            .1
            .repo_root()
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| "this board isn't inside a folder of projects".to_string())?;
        if !current_project::projects(&dir)
            .iter()
            .any(|p| p.name == name)
        {
            return Err(format!("no project '{name}' in {}", dir.display()));
        }
        let before = current_project::read(&dir);
        current_project::write(&dir, &name).map_err(err)?;
        show_from(&dir).inspect_err(|_| {
            let _ = match &before {
                Some(before) => current_project::write(&dir, before),
                None => std::fs::remove_file(dir.join(current_project::FILE)).map_err(Into::into),
            };
        })
    })
    .await;
    titled(&window, board)
}

/// Moves the process to `path`, opens the project found from there, and
/// remembers it for a launch from outside any project. Changes nothing if
/// no project opens.
fn show_from(path: &Path) -> Result<amd_ui_model::Board> {
    let previous = std::env::current_dir().ok();
    std::env::set_current_dir(path).map_err(|e| format!("{}: {e}", path.display()))?;
    match Project::open().and_then(|p| p.summary()) {
        Ok(board) => {
            let mut config = Config::load();
            if let Ok(here) = std::env::current_dir() {
                config.set(LAST_BOARD, &here.display().to_string());
                config.save().map_err(err)?;
            }
            Ok(board)
        }
        Err(e) => {
            if let Some(previous) = previous {
                let _ = std::env::set_current_dir(previous);
            }
            Err(e)
        }
    }
}

/// Starts the board. Launched from outside any project (a desktop launcher
/// starts in the home directory), it opens the board it showed last.
pub fn run() {
    boards::init_local_offset();
    if Board::discover().is_err()
        && let Some(last) = settings::Config::load().get(LAST_BOARD).map(PathBuf::from)
    {
        let _ = std::env::set_current_dir(last);
    }
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            load_board,
            board_fingerprint,
            sync_board,
            ticket_version,
            open_ticket,
            save_ticket,
            move_ticket,
            create_ticket,
            create_epic,
            create_sprint,
            start_sprint,
            rename_epic,
            archive_ticket,
            lock_ticket,
            unlock_ticket,
            set_wip_limits,
            set_lease_hours,
            add_tag,
            remove_tag,
            set_tags_only,
            auto_commit,
            set_auto_commit,
            appearance,
            set_appearance,
            get_template,
            preview_template,
            save_template,
            reset_template,
            tasks_dir,
            set_tasks_dir,
            use_project,
            clone_repository,
        ])
        .run(tauri::generate_context!())
        .expect("error while running amdui");
}

/// Clones a repository the project declares but this machine hasn't got
/// (ticket 108), beside the tickets repository, and registers it — what
/// `amd clone <name>` does. Returns a warning for each other clone of the
/// same remote (ticket 095).
#[tauri::command]
async fn clone_repository(state: Tauri<'_, AppState>, name: String) -> Result<Vec<String>> {
    let writing = Arc::clone(&state.writing);
    blocking(move || {
        let _guard = writing
            .lock()
            .map_err(|_| "the board is in a bad state — restart amdui".to_string())?;
        clone_into(&Project::open()?, &name)
    })
    .await
}

fn clone_into(project: &Project, name: &str) -> Result<Vec<String>> {
    let cloned =
        agile_md::clone::clone_repo(&project.repos[0].1, name).map_err(|e| format!("{e:#}"))?;
    Ok(cloned
        .also_at
        .iter()
        .map(|other| {
            format!(
                "{name} is also cloned at {} — two copies of one repository drift apart; prefer one",
                other.display()
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use amd_ui_model::Column as UiColumn;

    /// Ticket 117: the status line after a change matches its commit, with
    /// no empty `""` when there is no detail.
    #[test]
    fn the_status_line_says_what_the_commit_says() {
        let (_dir, project) = project();
        let board = &project.repos[0].1;
        let started = committed(board, "group-start", "Sprint 1", "", || Ok(())).unwrap();
        assert_eq!(started.message, "group-start: Sprint 1");
        let renamed = committed(board, "group-rename", "Old", "New", || Ok(())).unwrap();
        assert_eq!(renamed.message, "group-rename: Old \"New\"");
    }

    /// Ticket 122: the board carries the project's tags, listed and in
    /// use, with counts and the "only listed" switch, for Admin and the
    /// editor.
    #[test]
    fn the_board_carries_the_project_tags_with_counts() {
        let (_dir, project) = project();
        let board = &project.repos[0].1;
        let mut settings = board_settings::load(board);
        settings.add_tag("future").unwrap();
        settings.add_tag("old").unwrap();
        settings.set_only_listed_tags(true);
        settings.save(board).unwrap();
        let file = new_ticket(&project, "Tagged");
        let path = board.root.join("backlog").join(&file);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace("tags: []", "tags: [future,misc]")).unwrap();

        let summary = project.summary().unwrap();
        let uses: Vec<(String, usize, bool)> = summary
            .tags
            .iter()
            .map(|t| (t.name.clone(), t.count, t.listed))
            .collect();
        assert_eq!(
            uses,
            [
                ("FUTURE".to_string(), 1, true),
                ("MISC".to_string(), 1, false),
                ("OLD".to_string(), 0, true),
            ]
        );
        assert!(summary.tags_only);
    }

    #[test]
    fn the_window_is_titled_after_its_project() {
        assert_eq!(window_title("fantastic-tools"), "fantastic-tools — amdui");
        assert_eq!(window_title(""), "amdui");
        assert_eq!(window_title("  "), "amdui");
    }

    /// The page's "clone…" goes through amd-lib's own checks (ticket 108).
    /// A successful clone writes the machine's registry, so it's covered in
    /// tests/test.sh, which points that at a temp directory.
    #[test]
    fn cloning_refuses_a_repository_the_project_doesnt_declare() {
        let (_dir, project) = project();
        let err = clone_into(&project, "nosuch").unwrap_err();
        assert!(err.contains("isn't declared for this project"), "{err}");
    }

    /// A project of one git repository with a board, auto-commit off.
    fn project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("demo-tickets");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t.co"]);
        git(&["config", "user.name", "t"]);
        let board = Board::in_repo(&repo);
        board.create().unwrap();
        std::fs::write(repo.join(".settings"), "commit = off\n").unwrap();
        let project = Project {
            repos: vec![("demo-tickets".into(), board)],
            unregistered: vec!["demo-site".into()],
            identity: Some("t@t.co".into()),
        };
        (dir, project)
    }

    fn new_ticket(project: &Project, title: &str) -> String {
        let fields = TicketFields {
            title: title.into(),
            assignee: String::new(),
            repository: None,
            points: String::new(),
            epic: None,
            tags: Vec::new(),
            branch_type: String::new(),
            parent: String::new(),
            related: Vec::new(),
            body: String::new(),
        };
        let dest = transition::create_ticket(project.board("").unwrap(), &fields)
            .unwrap()
            .dest
            .unwrap();
        dest.file_name().unwrap().to_string_lossy().into_owned()
    }

    fn column(c: UiColumn) -> Place {
        Place::column(c)
    }

    #[test]
    fn the_board_shows_what_amd_lib_reads() {
        let (_dir, project) = project();
        let file = new_ticket(&project, "First");
        let board = project.summary().unwrap();
        let card = &board.columns[0].tickets[0];
        assert_eq!(
            (card.file.as_str(), card.title.as_str()),
            (file.as_str(), "First")
        );
        assert_eq!(card.priority, "low");
        assert_eq!(board.repos[0].name, "demo-tickets");
        assert_eq!(board.repos[0].wip.doing, 2);
        assert_eq!(board.repos[0].lease_hours, 8);
        assert!(!board.repos[1].available, "a declared repo not cloned here");
        // No declared project: the repository's name.
        assert_eq!(board.project, "demo-tickets");
    }

    #[test]
    fn the_header_shows_the_projects_own_name() {
        let (_dir, project) = project();
        let settings = agile_md::board_settings::path(project.board("").unwrap());
        std::fs::write(&settings, "commit = off\nproject = grimartz\n").unwrap();
        assert_eq!(project.summary().unwrap().project, "grimartz");
    }

    #[test]
    fn the_board_carries_the_project_users_for_the_assignee_picker() {
        let (_dir, project) = project();
        let settings = agile_md::board_settings::path(project.board("").unwrap());
        std::fs::write(
            &settings,
            "commit = off\nusers = oldippy <o@t.co>, Ann <a@t.co>\n",
        )
        .unwrap();
        let board = project.summary().unwrap();
        assert_eq!(board.users, ["Ann", "oldippy"]);
    }

    #[test]
    fn a_lapsed_lease_is_shown_as_lapsed_not_as_a_lock() {
        let (_dir, project) = project();
        let file = new_ticket(&project, "Lapsed");
        let path = project.board("").unwrap().root.join("backlog").join(&file);
        let text = std::fs::read_to_string(&path).unwrap();
        let text = text
            .replace("locked-by: \"\"", "locked-by: \"t@t.co\"")
            .replace(
                "locked-until: \"\"",
                "locked-until: \"2000-01-01T00:00:00Z\"",
            );
        std::fs::write(&path, text).unwrap();

        let ticket = project.summary().unwrap().columns[0].tickets[0].clone();
        assert!(ticket.lock.is_none(), "a lapsed lease holds nothing");
        let lapsed = ticket.lapsed.unwrap();
        assert!(lapsed.mine && lapsed.by == "t@t.co", "{lapsed:?}");
    }

    #[test]
    fn drags_move_tickets_the_way_amd_does() {
        let (_dir, project) = project();
        let a = new_ticket(&project, "A");
        let backlog = column(UiColumn::Backlog);
        let todo = column(UiColumn::Todo);
        let board = project.board("").unwrap();
        new_group(board, "ui", Kind::Epic, DEFAULT_DAYS).unwrap();

        // A column can't be skipped.
        let err = move_in(&project, &a, &backlog, &column(UiColumn::Done), false).unwrap_err();
        assert!(err.contains("one column at a time"), "{err}");

        // Into todo takes the lease for the mover.
        move_in(&project, &a, &backlog, &todo, false).unwrap();
        let summary = project.summary().unwrap();
        let lock = summary.columns[1].tickets[0].lock.clone().unwrap();
        assert!(lock.mine && lock.by == "t@t.co", "{lock:?}");

        // From todo onto an epic: back to the backlog, filed in it.
        move_in(&project, &a, &todo, &Place::epic("ui"), false).unwrap();
        let summary = project.summary().unwrap();
        assert_eq!(summary.epics[0].tickets[0].file, a);

        // Out of the epic straight to todo, keeping its label; and back.
        move_in(&project, &a, &Place::epic("ui"), &todo, false).unwrap();
        let card = project.summary().unwrap().columns[1].tickets[0].clone();
        assert_eq!(card.epic_label, "ui");
        move_in(&project, &a, &todo, &backlog, false).unwrap();
        assert_eq!(project.summary().unwrap().epics[0].tickets.len(), 1);

        // Out of the epic into the loose backlog.
        move_in(&project, &a, &Place::epic("ui"), &backlog, false).unwrap();
        let summary = project.summary().unwrap();
        assert!(summary.epics[0].tickets.is_empty());
        assert_eq!(summary.columns[0].tickets[0].epic_label, "");
    }

    #[test]
    fn doing_takes_two_unless_forced() {
        let (_dir, project) = project();
        let files: Vec<String> = ["A", "B", "C"]
            .iter()
            .map(|t| new_ticket(&project, t))
            .collect();
        let (backlog, todo, doing) = (
            column(UiColumn::Backlog),
            column(UiColumn::Todo),
            column(UiColumn::Doing),
        );
        for f in &files {
            move_in(&project, f, &backlog, &todo, false).unwrap();
        }
        move_in(&project, &files[0], &todo, &doing, false).unwrap();
        move_in(&project, &files[1], &todo, &doing, false).unwrap();
        let err = move_in(&project, &files[2], &todo, &doing, false).unwrap_err();
        assert!(err.contains("2 of 2"), "{err}");
        move_in(&project, &files[2], &todo, &doing, true).unwrap();
        assert_eq!(project.summary().unwrap().columns[2].tickets.len(), 3);
    }

    #[test]
    fn the_editor_saves_through_amd_lib_and_keeps_locks() {
        let (_dir, project) = project();
        let file = new_ticket(&project, "Edit me");
        let board = project.board("").unwrap();
        let place = column(UiColumn::Backlog);
        let task = task_at(board, &place, &file).unwrap();
        let opened = task.hash().unwrap();
        let text = std::fs::read_to_string(&task.path).unwrap();

        // Locked after the editor opened: the save keeps the lock, but is
        // refused as a conflict because the file changed underneath it.
        task.take_lease(opened, 8.0).unwrap();
        let edited = text.replace("## Notes", "## Notes\n\nMore");
        let err = transition::save_text(board, &task, opened, &edited).unwrap_err();
        assert!(format!("{err:#}").contains("changed"), "{err:#}");

        // Reopened, the save lands and the lock stays.
        let task = task_at(board, &place, &file).unwrap();
        transition::save_text(board, &task, task.hash().unwrap(), &edited).unwrap();
        let saved = std::fs::read_to_string(&task.path).unwrap();
        assert!(
            saved.contains("More") && saved.contains("locked-by: \"t@t.co\""),
            "{saved}"
        );

        // An invalid priority or size is refused.
        let task = task_at(board, &place, &file).unwrap();
        let bad = saved.replace("priority: \"low\"", "priority: \"urgent\"");
        assert!(transition::save_text(board, &task, task.hash().unwrap(), &bad).is_err());
    }

    #[test]
    fn templates_preview_and_save_through_amd_lib() {
        let (_dir, project) = project();
        let board = project.board("").unwrap();
        let built_in = template_of(board).unwrap();
        assert!(!built_in.customised);
        let rendered = preview(board, &built_in.source).unwrap();
        assert!(
            rendered.contains("title: \"An example ticket\""),
            "{rendered}"
        );
        assert!(preview(board, "{{ nope").is_err());
    }

    #[test]
    fn the_editor_derives_branch_names_as_amd_does() {
        for title in [
            "Crash on save",
            "Ünïcödé — títlé!",
            "a very long title that runs well past the fifty character limit amd sets",
        ] {
            assert_eq!(
                amd_ui_model::slugify(title),
                agile_md::task::slugify(title),
                "{title}"
            );
        }
    }
}
