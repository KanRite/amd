//! amdui's backend: the Tauri commands the board's window calls. Each one
//! reads the project the way `amd` does and makes its change through
//! amd-lib — the same moves, leases, WIP limit, groups, templates and
//! auto-commit as the command line (ticket 084, docs/adr/0004). There is no
//! ticket, git or settings logic here of its own.

mod boards;
mod documents;
mod terminal;

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
    /// When each project's GitHub issues were last pulled in from the
    /// landing page (ticket 173), by its board repository, so going back to
    /// the landing page doesn't call GitHub every time.
    issues_pulled: Arc<Mutex<std::collections::HashMap<PathBuf, std::time::Instant>>>,
    /// The bottom panel's terminals (ticket 185), by id.
    terminals: Arc<Mutex<std::collections::HashMap<u32, terminal::Session>>>,
    next_terminal: Arc<std::sync::atomic::AtomicU32>,
}

/// The AI tab's command when Preferences doesn't name one (ticket 185).
const DEFAULT_AI_COMMAND: &str = "claude";

/// How long the landing page leaves a project's issues before asking
/// GitHub again (ticket 173).
const ISSUE_PULL_EVERY: std::time::Duration = std::time::Duration::from_secs(5 * 60);

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
        let project = Project::open()?;
        // An archived project only reads (ticket 157).
        agile_md::archive::ensure_writable(&project.repos[0].1).map_err(err)?;
        for (_, board) in &project.repos {
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
            authorship: agile_md::authorship::summary(board, &task).unwrap_or_default(),
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
    draft: Option<bool>,
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
            // A draft is a ticket carrying DRAFT (ticket 114).
            tags: if draft.unwrap_or(false) {
                vec![agile_md::task::DRAFT_TAG.to_string()]
            } else {
                Vec::new()
            },
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
                authorship: agile_md::authorship::summary(board, &task).unwrap_or_default(),
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
            &format!("prevent-arbitrary-tags {}", if on { "on" } else { "off" }),
            |s| {
                s.set_only_listed_tags(on);
                Ok(())
            },
        )
    })
    .await
}

/// Names the documents repository every project shares (ticket 129), or
/// clears it when `name` is empty.
#[tauri::command]
async fn set_shared_documents(state: Tauri<'_, AppState>, name: String) -> Result<SyncReport> {
    change(&state, move |p| {
        let board = &p.repos[0].1;
        let name = name.trim().to_string();
        let what = if name.is_empty() {
            "shared-documents cleared".to_string()
        } else {
            format!("shared-documents {name}")
        };
        save_setting(board, &what, |s| {
            s.set_shared_documents(Some(name.as_str()))
        })
    })
    .await
}

/// Pulls in open GitHub issues for every project on this machine whose
/// switch is on (ticket 173), when the landing page loads: at most once per
/// project every five minutes. Quiet about a project with nothing on
/// GitHub, no `gh`, or no network; one report per project that got
/// something.
///
/// GitHub is asked **without** the write lock (ticket 181): this runs on
/// every cold start and takes seconds per project, and holding the lock
/// made every button that opens a project wait for it. Only a project
/// with something new takes the lock, to catch up, plan again against
/// the counter as it is then, write and commit.
#[tauri::command]
async fn pull_issues(state: Tauri<'_, AppState>) -> Result<Vec<SyncReport>> {
    let writing = Arc::clone(&state.writing);
    let pulled = Arc::clone(&state.issues_pulled);
    blocking(move || {
        use agile_md::import_issues as issues;
        let mut reports = Vec::new();
        for project in agile_md::my_work::projects() {
            if project.archived {
                continue;
            }
            {
                let mut seen = pulled.lock().map_err(|_| "restart amdui".to_string())?;
                if seen
                    .get(&project.root)
                    .is_some_and(|at| at.elapsed() < ISSUE_PULL_EVERY)
                {
                    continue;
                }
                seen.insert(project.root.clone(), std::time::Instant::now());
            }
            let board = Board::in_repo(&project.root);
            if !board_settings::load(&board).import_issues() {
                continue;
            }
            let Ok(targets) = issues::targets(&board, None) else {
                continue;
            };
            // The network, unlocked: each repository's open issues, once.
            let fetched: Vec<(String, std::result::Result<Vec<issues::Issue>, String>)> = targets
                .iter()
                .map(|t| {
                    (
                        t.slug.clone(),
                        issues::fetch(&t.slug, None).map_err(|e| format!("{e:#}")),
                    )
                })
                .collect();
            let answer = |slug: &str, _: Option<&str>| -> anyhow::Result<Vec<issues::Issue>> {
                match fetched.iter().find(|(s, _)| s == slug) {
                    Some((_, Ok(found))) => Ok(found.clone()),
                    Some((_, Err(e))) => Err(anyhow::anyhow!("{e}")),
                    None => Ok(Vec::new()),
                }
            };
            // Anything new? Read-only, so still unlocked.
            match issues::plan_with(&board, &targets, None, answer) {
                Ok(plan) if !plan.new.is_empty() => {}
                _ => continue,
            }
            let _guard = writing
                .lock()
                .map_err(|_| "the board is in a bad state — restart amdui".to_string())?;
            if agile_md::sync::catch_up(&board, agile_md::sync::Purpose::Write).is_err() {
                continue;
            }
            let Ok(plan) = issues::plan_with(&board, &targets, None, answer) else {
                continue;
            };
            if plan.new.is_empty() {
                continue;
            }
            let detail = format!("{} from GitHub", plan.new.len());
            let Ok((done, warning)) =
                transition::with_auto_commit(&board, "import", "issues", &detail, || {
                    issues::apply(&board, &plan)
                })
            else {
                continue;
            };
            let n = done.len();
            reports.push(report(
                &board,
                format!(
                    "Imported {n} GitHub issue{} into {}",
                    if n == 1 { "" } else { "s" },
                    project.name
                ),
                warning,
            ));
        }
        Ok(reports)
    })
    .await
}

/// Whether the landing page pulls in this project's GitHub issues (ticket
/// 173).
#[tauri::command]
async fn set_import_issues(state: Tauri<'_, AppState>, on: bool) -> Result<SyncReport> {
    change(&state, move |p| {
        let board = &p.repos[0].1;
        save_setting(
            board,
            &format!("import-issues {}", if on { "on" } else { "off" }),
            |s| {
                s.set_import_issues(on);
                Ok(())
            },
        )
    })
    .await
}

/// The priority an imported GitHub issue starts at (ticket 158).
#[tauri::command]
async fn set_triage_priority(state: Tauri<'_, AppState>, priority: String) -> Result<SyncReport> {
    change(&state, move |p| {
        let board = &p.repos[0].1;
        let priority = priority.trim().to_lowercase();
        save_setting(board, &format!("triage-priority {priority}"), |s| {
            s.set_triage_priority(&priority)
        })
    })
    .await
}

/// Brings a repository's open GitHub issues into the backlog, tagged
/// TRIAGE (ticket 158), as `amd issues` does; `preview` only says
/// what it would do. `repo` empty means every declared repository on
/// GitHub; `label` empty means every issue.
#[tauri::command]
async fn import_issues(
    state: Tauri<'_, AppState>,
    repo: String,
    label: String,
    preview: bool,
) -> Result<amd_ui_model::IssueImport> {
    use agile_md::import_issues as issues;
    let some = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    let (repo, label) = (some(&repo), some(&label));
    let run = move |p: &Project| -> Result<amd_ui_model::IssueImport> {
        let board = &p.repos[0].1;
        let targets = issues::targets(board, repo.as_deref()).map_err(err)?;
        let plan = issues::plan(board, &targets, label.as_deref()).map_err(err)?;
        if preview || plan.new.is_empty() {
            return Ok(amd_ui_model::IssueImport {
                lines: plan.new.iter().map(issues::Planned::line).collect(),
                summary: plan.summary(preview),
                sync: None,
            });
        }
        // Only the repositories it could read (ticket 180).
        let slugs: Vec<&str> = targets
            .iter()
            .map(|t| t.slug.as_str())
            .filter(|s| !plan.skipped.iter().any(|(k, _)| k == s))
            .collect();
        let detail = format!("{} from {}", plan.new.len(), slugs.join(", "));
        let (done, warning) =
            transition::with_auto_commit(board, "import", "issues", &detail, || {
                issues::apply(board, &plan)
            })
            .map_err(err)?;
        Ok(amd_ui_model::IssueImport {
            lines: done.iter().map(issues::Planned::line).collect(),
            summary: plan.summary(false),
            sync: Some(report(
                board,
                transition::commit_message("import", "issues", &detail),
                warning,
            )),
        })
    };
    if preview {
        blocking(move || run(&Project::open()?)).await
    } else {
        change(&state, run).await
    }
}

/// The project's documents repositories and what's in them (ticket 129):
/// its own, then the shared one. One not cloned here is listed empty.
#[tauri::command]
async fn list_documents() -> Result<Vec<amd_ui_model::DocSource>> {
    blocking(|| {
        let project = Project::open()?;
        Ok(agile_md::documents::sources(&project.repos[0].1)
            .into_iter()
            .map(|source| {
                let documents = source
                    .root
                    .as_deref()
                    .and_then(|root| agile_md::documents::list(root).ok())
                    .unwrap_or_default();
                let folders = source
                    .root
                    .as_deref()
                    .map(agile_md::documents_edit::folders)
                    .unwrap_or_default();
                // A start page is the project's own (ticket 179).
                let own = source.kind == agile_md::documents::Kind::Own;
                let start = source
                    .root
                    .as_deref()
                    .filter(|_| own)
                    .and_then(agile_md::documents_edit::start_page);
                let pinned = own
                    && source
                        .root
                        .as_deref()
                        .and_then(agile_md::documents_edit::pinned)
                        .is_some();
                amd_ui_model::DocSource {
                    start,
                    pinned,
                    shared: source.kind == agile_md::documents::Kind::Shared,
                    cloned: source.root.is_some(),
                    name: source.name,
                    documents: documents
                        .into_iter()
                        .map(|d| amd_ui_model::DocEntry {
                            path: d.path,
                            title: d.title,
                            archived: d.archived,
                        })
                        .collect(),
                    folders,
                }
            })
            .collect())
    })
    .await
}

/// The document `path` in the documents repository `source`, rendered for
/// the reader (ticket 129). Only the project's own documents repositories
/// can be read, and nothing outside them.
#[tauri::command]
async fn read_document(source: String, path: String) -> Result<amd_ui_model::RenderedDoc> {
    blocking(move || {
        let root = documents_root(&source)?;
        let text = agile_md::documents::read(&root, &path).map_err(err)?;
        Ok(amd_ui_model::RenderedDoc {
            html: documents::render(&root, &path, &text),
            archived: agile_md::documents_edit::is_archived(&text),
            source,
            path,
            version: None,
        })
    })
    .await
}

/// The project's ideas, best first (ticket 131), with your own vote on
/// each.
#[tauri::command]
async fn list_ideas() -> Result<amd_ui_model::IdeaList> {
    blocking(|| {
        let project = Project::open()?;
        let source = agile_md::ideas::source(&project.repos[0].1);
        let me = source
            .root
            .as_deref()
            .and_then(|root| agile_md::ideas::Person::of(root).ok())
            .map(|p| p.email)
            .unwrap_or_default();
        Ok(amd_ui_model::IdeaList {
            cloned: source.root.is_some(),
            ideas: agile_md::ideas::list(&source)
                .iter()
                .map(|i| idea_card(i, &me))
                .collect(),
            source: source.name,
        })
    })
    .await
}

fn idea_card(idea: &agile_md::ideas::Idea, me: &str) -> amd_ui_model::IdeaCard {
    amd_ui_model::IdeaCard {
        path: idea.path.clone(),
        title: idea.title.clone(),
        author: idea
            .author
            .as_deref()
            .map(|a| a.split('<').next().unwrap_or(a).trim().to_string()),
        created: idea.created.clone(),
        score: idea.score(),
        voters: idea.votes.len(),
        mine: idea.vote_of(me),
        notes: idea.notes.len(),
        tickets: idea.tickets.clone(),
        archived: idea.archived,
    }
}

/// Where the project's ideas are, and who "you" are there.
fn ideas_root() -> Result<(Board, PathBuf, String, agile_md::ideas::Person)> {
    let project = Project::open()?;
    let board = Board::at(project.repos[0].1.root.clone());
    let source = agile_md::ideas::source(&board);
    let root = source
        .root
        .ok_or_else(|| format!("{} isn't cloned on this machine", source.name))?;
    let me = agile_md::ideas::Person::of(&root).map_err(err)?;
    Ok((board, root, source.name, me))
}

/// One idea opened: the write-up rendered, the votes and the notes.
#[tauri::command]
async fn read_idea(path: String) -> Result<amd_ui_model::IdeaDetail> {
    blocking(move || {
        let (_, root, source, me) = ideas_root()?;
        let idea = agile_md::ideas::read(&root, &source, &path).map_err(err)?;
        Ok(amd_ui_model::IdeaDetail {
            card: idea_card(&idea, &me.email),
            html: documents::render(&root, &path, &agile_md::ideas::write_up(&idea.text)),
            votes: idea
                .votes
                .iter()
                .map(|v| amd_ui_model::IdeaVote {
                    name: v.name.clone(),
                    value: v.value,
                })
                .collect(),
            notes: idea
                .notes
                .iter()
                .map(|n| amd_ui_model::IdeaNote {
                    name: n.name.clone(),
                    date: n.date.clone(),
                    text: n.text.clone(),
                })
                .collect(),
            source,
        })
    })
    .await
}

/// Runs a change to the project's ideas under the write lock; refused in an
/// archived project (ticket 157).
async fn idea_change(
    state: &Tauri<'_, AppState>,
    change: impl FnOnce(
        &Path,
        &agile_md::ideas::Person,
    ) -> anyhow::Result<(Option<String>, agile_md::documents::Restored)>
    + Send
    + 'static,
) -> Result<amd_ui_model::DocChange> {
    let writing = Arc::clone(&state.writing);
    blocking(move || {
        let _guard = writing
            .lock()
            .map_err(|_| "the board is in a bad state — restart amdui".to_string())?;
        let (board, root, _, me) = ideas_root()?;
        agile_md::archive::ensure_writable(&board).map_err(err)?;
        let (path, done) = change(&root, &me).map_err(err)?;
        Ok(amd_ui_model::DocChange {
            sync: SyncReport {
                committed: true,
                pushed: done.pushed,
                warning: done.warning,
                message: done.message,
            },
            path,
        })
    })
    .await
}

/// Puts an idea forward (ticket 131), with your name on it when `named`.
#[tauri::command]
async fn create_idea(
    state: Tauri<'_, AppState>,
    title: String,
    body: String,
    named: bool,
) -> Result<amd_ui_model::DocChange> {
    idea_change(&state, move |root, me| {
        let (path, done) = agile_md::ideas::create(root, &title, &body, named.then_some(me))?;
        Ok((Some(path), done))
    })
    .await
}

/// One click on an idea's + or −.
#[tauri::command]
async fn vote_idea(
    state: Tauri<'_, AppState>,
    path: String,
    delta: i32,
) -> Result<amd_ui_model::DocChange> {
    idea_change(&state, move |root, me| {
        let (_, done) = agile_md::ideas::vote(root, &path, delta.clamp(-1, 1), me)?;
        Ok((Some(path), done))
    })
    .await
}

/// Adds your note under an idea.
#[tauri::command]
async fn note_idea(
    state: Tauri<'_, AppState>,
    path: String,
    text: String,
) -> Result<amd_ui_model::DocChange> {
    idea_change(&state, move |root, me| {
        let done = agile_md::ideas::note(root, &path, &text, me)?;
        Ok((Some(path), done))
    })
    .await
}

/// Makes a backlog ticket from an idea, linked both ways.
#[tauri::command]
async fn idea_to_ticket(
    state: Tauri<'_, AppState>,
    path: String,
) -> Result<amd_ui_model::IdeaTicket> {
    change(&state, move |p| {
        let board = &p.repos[0].1;
        let source = agile_md::ideas::source(board);
        let root = source
            .root
            .ok_or_else(|| format!("{} isn't cloned on this machine", source.name))?;
        let (id, events, done) =
            agile_md::ideas::to_ticket(board, &root, &source.name, &path).map_err(err)?;
        let task = board.find(&id).map_err(err)?;
        let mut warnings: Vec<String> = events
            .into_iter()
            .filter_map(|e| match e {
                Event::Warn(w) => Some(w),
                Event::Info(_) => None,
            })
            .collect();
        warnings.extend(done.warning.clone());
        Ok(amd_ui_model::IdeaTicket {
            sync: SyncReport {
                committed: true,
                pushed: warnings.is_empty(),
                warning: (!warnings.is_empty()).then(|| warnings.join("; ")),
                message: format!("ticket {id} made from the idea"),
            },
            repo: board
                .repo_root()
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            file: task.file_name(),
            id,
        })
    })
    .await
}

/// Where the documents repository `source` (the project's own or the
/// shared one) is cloned; an error for any other name, or one not cloned.
fn documents_root(source: &str) -> Result<PathBuf> {
    let project = Project::open()?;
    agile_md::documents::sources(&project.repos[0].1)
        .into_iter()
        .find(|s| s.name == source)
        .ok_or_else(|| format!("{source} isn't one of this project's documents repositories"))?
        .root
        .ok_or_else(|| format!("{source} isn't cloned on this machine"))
}

fn doc_version(v: agile_md::documents::Version) -> amd_ui_model::DocVersion {
    amd_ui_model::DocVersion {
        commit: v.commit,
        short: v.short,
        author: v.author,
        date: v.date,
        subject: v.subject,
        path: v.path,
    }
}

/// The last 20 versions of a document, newest first (ticket 169).
#[tauri::command]
async fn document_history(source: String, path: String) -> Result<Vec<amd_ui_model::DocVersion>> {
    blocking(move || {
        let root = documents_root(&source)?;
        Ok(agile_md::documents::history(&root, &path, 20)
            .map_err(err)?
            .into_iter()
            .map(doc_version)
            .collect())
    })
    .await
}

/// An earlier version of a document, rendered for the reader against
/// today's tree (images, links) (ticket 169).
#[tauri::command]
async fn read_document_version(
    source: String,
    path: String,
    commit: String,
) -> Result<amd_ui_model::RenderedDoc> {
    blocking(move || {
        let root = documents_root(&source)?;
        let version = agile_md::documents::history(&root, &path, usize::MAX)
            .map_err(err)?
            .into_iter()
            .find(|v| v.commit == commit)
            .ok_or_else(|| format!("{commit} isn't a version of {path}"))?;
        let text = agile_md::documents::read_version(&root, &version.path, &version.commit)
            .map_err(err)?;
        Ok(amd_ui_model::RenderedDoc {
            html: documents::render(&root, &path, &text),
            archived: agile_md::documents_edit::is_archived(&text),
            source,
            path,
            version: Some(doc_version(version)),
        })
    })
    .await
}

/// Runs a documents-repository change under the write lock and reports it
/// (ticket 176).
async fn doc_change(
    state: &Tauri<'_, AppState>,
    source: String,
    change: impl FnOnce(&Path) -> anyhow::Result<(Option<String>, agile_md::documents::Restored)>
    + Send
    + 'static,
) -> Result<amd_ui_model::DocChange> {
    let writing = Arc::clone(&state.writing);
    blocking(move || {
        let _guard = writing
            .lock()
            .map_err(|_| "the board is in a bad state — restart amdui".to_string())?;
        let root = documents_root(&source)?;
        // Not the documents of an archived project (ticket 157); a shared
        // repository belongs to no one project.
        let project = Project::open()?;
        let board = &project.repos[0].1;
        if source
            == format!(
                "{}-documents",
                agile_md::current_project::project_name(board.repo_root())
            )
        {
            agile_md::archive::ensure_writable(board).map_err(err)?;
        }
        let (path, done) = change(&root).map_err(err)?;
        Ok(amd_ui_model::DocChange {
            sync: SyncReport {
                committed: true,
                pushed: done.pushed,
                warning: done.warning,
                message: done.message,
            },
            path,
        })
    })
    .await
}

/// Pins a document as the project's start page (ticket 179), or with no
/// path goes back to the README.
#[tauri::command]
async fn pin_start_page(
    state: Tauri<'_, AppState>,
    source: String,
    path: Option<String>,
) -> Result<amd_ui_model::DocChange> {
    doc_change(&state, source, move |root| {
        Ok((
            None,
            agile_md::documents_edit::pin_start_page(root, path.as_deref())?,
        ))
    })
    .await
}

/// Searches the project's documents and the shared ones (ticket 179):
/// titles and text, ignoring case, at most 3 lines a document and 200 in
/// all.
#[tauri::command]
async fn search_documents(query: String) -> Result<Vec<amd_ui_model::DocHit>> {
    blocking(move || {
        let project = Project::open()?;
        let mut out = Vec::new();
        for source in agile_md::documents::sources(&project.repos[0].1) {
            let Some(root) = source.root else { continue };
            for hit in agile_md::documents_edit::search(&root, &query, 3).map_err(err)? {
                out.push(amd_ui_model::DocHit {
                    source: source.name.clone(),
                    path: hit.path,
                    title: hit.title,
                    line: hit.line,
                    text: hit.text,
                });
            }
        }
        out.truncate(200);
        Ok(out)
    })
    .await
}

/// Marks a document archived, or not (ticket 176).
#[tauri::command]
async fn archive_document(
    state: Tauri<'_, AppState>,
    source: String,
    path: String,
    on: bool,
) -> Result<amd_ui_model::DocChange> {
    doc_change(&state, source, move |root| {
        Ok((
            None,
            agile_md::documents_edit::set_archived(root, &path, on)?,
        ))
    })
    .await
}

/// Makes a folder in a documents repository (ticket 176).
#[tauri::command]
async fn create_document_folder(
    state: Tauri<'_, AppState>,
    source: String,
    parent: String,
    name: String,
) -> Result<amd_ui_model::DocChange> {
    doc_change(&state, source, move |root| {
        Ok((
            None,
            agile_md::documents_edit::create_folder(root, &parent, &name)?,
        ))
    })
    .await
}

/// Removes an empty folder from a documents repository (ticket 176).
#[tauri::command]
async fn delete_document_folder(
    state: Tauri<'_, AppState>,
    source: String,
    folder: String,
) -> Result<amd_ui_model::DocChange> {
    doc_change(&state, source, move |root| {
        Ok((
            None,
            agile_md::documents_edit::delete_folder(root, &folder)?,
        ))
    })
    .await
}

/// Moves a document into a folder, its links kept working (ticket 176).
#[tauri::command]
async fn move_document(
    state: Tauri<'_, AppState>,
    source: String,
    path: String,
    folder: String,
) -> Result<amd_ui_model::DocChange> {
    doc_change(&state, source, move |root| {
        let (to, done) = agile_md::documents_edit::move_document(root, &path, &folder)?;
        Ok((Some(to), done))
    })
    .await
}

/// Makes a new document titled `title` in `folder` (ticket 176).
#[tauri::command]
async fn create_document(
    state: Tauri<'_, AppState>,
    source: String,
    folder: String,
    title: String,
) -> Result<amd_ui_model::DocChange> {
    doc_change(&state, source, move |root| {
        let (path, done) = agile_md::documents_edit::create_document(root, &folder, &title)?;
        Ok((Some(path), done))
    })
    .await
}

/// A document's text for editing: its frontmatter apart, the body, its
/// version, and what (if anything) keeps it out of rich text (ticket 176).
#[tauri::command]
async fn open_document_for_edit(source: String, path: String) -> Result<amd_ui_model::DocEdit> {
    blocking(move || {
        let root = documents_root(&source)?;
        let text = agile_md::documents::read(&root, &path).map_err(err)?;
        let (front, body) = agile_md::documents_edit::split_frontmatter(&text);
        Ok(amd_ui_model::DocEdit {
            version: agile_md::documents_edit::version(&text),
            frontmatter: front.unwrap_or_default().to_string(),
            unsupported: amd_ui_model::rich::unsupported(body)
                .into_iter()
                .map(str::to_string)
                .collect(),
            body: body.to_string(),
            source,
            path,
        })
    })
    .await
}

/// Saves an edited document, unless it changed since it was opened
/// (ticket 176).
#[tauri::command]
async fn save_document(
    state: Tauri<'_, AppState>,
    source: String,
    path: String,
    text: String,
    version: String,
) -> Result<amd_ui_model::DocChange> {
    doc_change(&state, source, move |root| {
        Ok((
            None,
            agile_md::documents_edit::save(root, &path, &text, &version)?,
        ))
    })
    .await
}

/// Puts a document back as it was at `commit`, as a new commit, pushed
/// (ticket 169; ADR 0007). Serialised with every other write.
#[tauri::command]
async fn restore_document(
    state: Tauri<'_, AppState>,
    source: String,
    path: String,
    commit: String,
) -> Result<SyncReport> {
    let writing = Arc::clone(&state.writing);
    blocking(move || {
        let _guard = writing
            .lock()
            .map_err(|_| "the board is in a bad state — restart amdui".to_string())?;
        let root = documents_root(&source)?;
        let done = agile_md::documents::restore(&root, &path, &commit).map_err(err)?;
        Ok(SyncReport {
            committed: true,
            pushed: done.pushed,
            warning: done.warning,
            message: done.message,
        })
    })
    .await
}

/// Opens a web or mail link from a document in the system's own browser or
/// mail program, never in amdui's window (ticket 129).
#[tauri::command]
async fn open_external(url: String) -> Result<()> {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:"))
    {
        return Err(format!("amd only opens web and mail links, not {url}"));
    }
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(&url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("couldn't open {url}: {e}"))
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

/// Applies the board's saved zoom (ticket 024) and returns it, in percent.
#[tauri::command]
fn zoom(window: tauri::WebviewWindow) -> Result<u32> {
    let percent = amd_ui_model::zoom::parse(Config::load().get("ui.zoom").unwrap_or_default());
    window
        .set_zoom(f64::from(percent) / 100.0)
        .map_err(|e| format!("zooming the board: {e}"))?;
    Ok(percent)
}

/// Zooms the board to `percent` (snapped to a step) and remembers it on this
/// machine, so it opens at that size next time.
#[tauri::command]
fn set_zoom(window: tauri::WebviewWindow, percent: u32) -> Result<u32> {
    let percent = amd_ui_model::zoom::normalise(percent);
    window
        .set_zoom(f64::from(percent) / 100.0)
        .map_err(|e| format!("zooming the board: {e}"))?;
    let mut config = Config::load();
    config.set("ui.zoom", &percent.to_string());
    config.save().map_err(err)?;
    Ok(percent)
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

/// The landing page (ticket 020): your todo and doing work across every
/// project on this machine, and the projects. Read fresh each time.
#[tauri::command]
async fn home() -> Result<amd_ui_model::Home> {
    blocking(|| {
        use agile_md::my_work;
        let here = std::env::current_dir().unwrap_or_default();
        let me = my_work::me(&here);
        let work = match &me {
            Some((email, name)) => {
                my_work::my_work(email, name.as_deref(), false, my_work::Sort::Priority)
            }
            None => Vec::new(),
        };
        // Nothing to do (ticket 160): work to pick from the backlogs, and
        // any hidden project it's in shown again before the list is read.
        let include_unassigned = my_work::include_unassigned();
        let to_pick = match &me {
            Some((email, name)) if work.is_empty() => {
                let pick = my_work::to_pick(email, name.as_deref(), include_unassigned);
                my_work::show_projects_of(&pick).map_err(err)?;
                pick
            }
            _ => Vec::new(),
        };
        let convert = |w: my_work::WorkItem| amd_ui_model::WorkItem {
            project: w.project,
            project_root: w.project_root.display().to_string(),
            repo: w.repo,
            column: agile_md::board::Column::parse(&w.column)
                .map(boards::from_column)
                .unwrap_or(amd_ui_model::Column::Todo),
            id: w.id,
            file: format!("{}.md", w.stem),
            title: w.title,
            priority: w.priority,
            points: w.points,
            created: w.created,
            folder: w.folder,
        };
        Ok(amd_ui_model::Home {
            work: work.into_iter().map(convert).collect(),
            to_pick: to_pick.into_iter().map(convert).collect(),
            include_unassigned,
            projects: my_work::projects()
                .into_iter()
                .map(|p| {
                    // Which documents buttons can work here (ticket 168).
                    use agile_md::documents::{Kind, sources};
                    let docs = sources(&agile_md::board::Board::in_repo(&p.root));
                    let own = docs.iter().find(|d| d.kind == Kind::Own);
                    let shared = docs.iter().find(|d| d.kind == Kind::Shared);
                    amd_ui_model::HomeProject {
                        name: p.name,
                        root: p.root.display().to_string(),
                        dir: p.dir.display().to_string(),
                        hidden: p.hidden,
                        documents: own.is_some_and(|d| d.root.is_some()),
                        shared: shared.map(|d| d.name.clone()),
                        shared_cloned: shared.is_some_and(|d| d.root.is_some()),
                        archived: p.archived,
                    }
                })
                .collect(),
            me: me.map(|(email, _)| email).unwrap_or_default(),
            // The project open in this process: what the launch directory
            // found, or the last one opened (ticket 168).
            current: Project::open()
                .ok()
                .map(|p| p.repos[0].1.repo_root().display().to_string())
                .unwrap_or_default(),
        })
    })
    .await
}

/// Archives a project, or brings it back (ticket 157), with `github` its
/// repositories on GitHub too. Works on any project on this machine, from
/// the landing page as well as from inside it, so it doesn't go through
/// [`change`], which refuses an archived project.
#[tauri::command]
async fn set_project_archived(
    state: Tauri<'_, AppState>,
    root: String,
    on: bool,
    github: bool,
) -> Result<SyncReport> {
    let writing = Arc::clone(&state.writing);
    blocking(move || {
        let _guard = writing
            .lock()
            .map_err(|_| "the board is in a bad state — restart amdui".to_string())?;
        // A repository's root (the landing page) or its board (Admin).
        let board = match Board::in_repo(Path::new(&root)) {
            board if board.root.is_dir() => board,
            _ => Board::at(PathBuf::from(&root)),
        };
        agile_md::sync::catch_up(&board, agile_md::sync::Purpose::Write).map_err(err)?;
        let events =
            agile_md::archive::set_archived(&board, on, github, &mut agile_md::archive::gh)
                .map_err(err)?;
        let mut info = Vec::new();
        let mut warnings = Vec::new();
        for event in events {
            match event {
                Event::Info(text) => info.push(text),
                Event::Warn(text) => warnings.push(text),
            }
        }
        Ok(report(
            &board,
            info.join("; "),
            (!warnings.is_empty()).then(|| warnings.join("; ")),
        ))
    })
    .await
}

/// Hides a project from My work, or shows it again (ticket 020): saved in
/// your own config on this machine, never committed.
#[tauri::command]
async fn set_project_hidden(root: String, hidden: bool) -> Result<()> {
    blocking(move || agile_md::my_work::set_hidden(Path::new(&root), hidden).map_err(err)).await
}

/// My work's "Include unassigned tickets" (ticket 160), remembered in your
/// own config on this machine.
#[tauri::command]
async fn set_include_unassigned(on: bool) -> Result<()> {
    blocking(move || agile_md::my_work::set_include_unassigned(on).map_err(err)).await
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
        // No shell outlives the window (ticket 185).
        .on_window_event(|window, event| {
            use tauri::Manager;
            if let tauri::WindowEvent::Destroyed = event
                && let Some(state) = window.try_state::<AppState>()
                && let Ok(mut terminals) = state.terminals.lock()
            {
                terminals.clear();
            }
        })
        .invoke_handler(tauri::generate_handler![
            load_board,
            home,
            set_project_hidden,
            set_project_archived,
            pin_start_page,
            search_documents,
            list_ideas,
            read_idea,
            create_idea,
            vote_idea,
            note_idea,
            idea_to_ticket,
            set_include_unassigned,
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
            set_shared_documents,
            import_issues,
            set_triage_priority,
            pull_issues,
            set_import_issues,
            list_documents,
            document_history,
            read_document_version,
            restore_document,
            archive_document,
            create_document_folder,
            delete_document_folder,
            move_document,
            create_document,
            open_document_for_edit,
            save_document,
            read_document,
            open_external,
            auto_commit,
            set_auto_commit,
            appearance,
            set_appearance,
            zoom,
            set_zoom,
            get_template,
            preview_template,
            save_template,
            reset_template,
            tasks_dir,
            set_tasks_dir,
            use_project,
            clone_repository,
            page_panic,
            terminal_open,
            terminal_input,
            terminal_resize,
            terminal_scroll,
            terminal_close,
            ai_command,
            set_ai_command,
            editor_command,
            set_editor_command,
            open_in_editor,
        ])
        .run(tauri::generate_context!())
        .expect("error while running amdui");
}

/// The page panicked: its message, on amdui's error output (the page has
/// stopped by then, and its console isn't visible).
#[tauri::command]
fn page_panic(message: String) {
    eprintln!("amdui: the page panicked: {message}");
}

/// Frames and endings from the panel's terminals, as Tauri events.
struct TauriSink(tauri::AppHandle);

impl terminal::Sink for TauriSink {
    fn frame(&self, frame: amd_ui_model::terminal::TermFrame) {
        use tauri::Emitter;
        let _ = self.0.emit("terminal-frame", frame);
    }
    fn exited(&self, id: u32) {
        use tauri::Emitter;
        let _ = self.0.emit("terminal-exit", id);
    }
}

/// Starts a terminal for the bottom panel (ticket 185): the user's shell,
/// or the AI command from Preferences, in the project folder (the folder
/// holding its repositories), on a `cols` × `rows` screen. Returns its id.
#[tauri::command]
async fn terminal_open(
    app: tauri::AppHandle,
    state: Tauri<'_, AppState>,
    kind: amd_ui_model::terminal::TermKind,
    cols: u16,
    rows: u16,
) -> Result<u32> {
    let terminals = Arc::clone(&state.terminals);
    let next = Arc::clone(&state.next_terminal);
    blocking(move || {
        use amd_ui_model::terminal::TermKind;
        let cwd = Project::open()
            .ok()
            .map(|p| agile_md::registry::project_dir_of(p.repos[0].1.repo_root()));
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "/bin/bash".to_string());
        let (program, args) = match kind {
            TermKind::Shell => (shell, vec!["-l".to_string()]),
            TermKind::Ai => (shell, vec!["-lic".to_string(), ai_command_now()]),
        };
        let id = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let session = terminal::Session::open(
            id,
            program.clone(),
            args,
            cwd,
            cols,
            rows,
            Arc::new(TauriSink(app)),
        )
        .map_err(|e| format!("starting {program}: {e}"))?;
        terminals
            .lock()
            .map_err(|_| "restart amdui".to_string())?
            .insert(id, session);
        Ok(id)
    })
    .await
}

/// Runs `f` on terminal `id`, if it's still open.
fn with_terminal(state: &AppState, id: u32, f: impl FnOnce(&terminal::Session)) -> Result<()> {
    let terminals = state
        .terminals
        .lock()
        .map_err(|_| "restart amdui".to_string())?;
    if let Some(session) = terminals.get(&id) {
        f(session);
    }
    Ok(())
}

/// Keystrokes or pasted text for terminal `id`.
#[tauri::command]
fn terminal_input(state: Tauri<'_, AppState>, id: u32, text: String) -> Result<()> {
    with_terminal(&state, id, |s| s.input(&text))
}

/// Terminal `id`'s screen is now `cols` × `rows`.
#[tauri::command]
fn terminal_resize(state: Tauri<'_, AppState>, id: u32, cols: u16, rows: u16) -> Result<()> {
    with_terminal(&state, id, |s| s.resize(cols, rows))
}

/// Scrolls terminal `id` back (positive) or forward through its history.
#[tauri::command]
fn terminal_scroll(state: Tauri<'_, AppState>, id: u32, lines: i32) -> Result<()> {
    with_terminal(&state, id, |s| s.scroll(lines))
}

/// Ends terminal `id` and its program.
#[tauri::command]
fn terminal_close(state: Tauri<'_, AppState>, id: u32) -> Result<()> {
    let session = state
        .terminals
        .lock()
        .map_err(|_| "restart amdui".to_string())?
        .remove(&id);
    // Dropped outside the lock: ending the program can take a moment.
    drop(session);
    Ok(())
}

fn ai_command_now() -> String {
    Config::load()
        .get("ui.ai-command")
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .unwrap_or(DEFAULT_AI_COMMAND)
        .to_string()
}

/// The command the AI tab runs (ticket 185), `claude` unless Preferences
/// says otherwise.
#[tauri::command]
fn ai_command() -> String {
    ai_command_now()
}

/// Sets the AI tab's command in your own config; empty goes back to
/// `claude`.
#[tauri::command]
fn set_ai_command(command: String) -> Result<String> {
    let mut config = Config::load();
    let command = command.trim();
    if command.is_empty() || command == DEFAULT_AI_COMMAND {
        config.remove("ui.ai-command");
    } else {
        config.set("ui.ai-command", command);
    }
    config.save().map_err(err)?;
    Ok(ai_command_now())
}

/// The editor command from Preferences (ticket 188), empty when none is set.
fn editor_command_now() -> String {
    Config::load()
        .get("ui.editor-command")
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

/// The command the board's Editor button runs; empty means no button.
#[tauri::command]
fn editor_command() -> String {
    editor_command_now()
}

/// Sets the editor command in your own config; empty removes it.
#[tauri::command]
fn set_editor_command(command: String) -> Result<String> {
    let mut config = Config::load();
    let command = command.trim();
    if command.is_empty() {
        config.remove("ui.editor-command");
    } else {
        config.set("ui.editor-command", command);
    }
    config.save().map_err(err)?;
    Ok(editor_command_now())
}

/// Opens the project's folder in the editor from Preferences (ticket 188).
/// The editor starts in a session of its own with no terminal, so it isn't
/// amdui's child: closing amdui leaves it running. It goes through the
/// user's shell, interactive like the AI tab's, so PATH is the one a
/// terminal has.
#[tauri::command]
async fn open_in_editor() -> Result<()> {
    blocking(move || {
        let command = editor_command_now();
        if command.is_empty() {
            return Err("set an editor in Preferences first".to_string());
        }
        let dir = Project::open()
            .map(|p| agile_md::registry::project_dir_of(p.repos[0].1.repo_root()))?;
        spawn_detached(&command, &dir).map_err(|e| format!("starting the editor ({command}): {e}"))
    })
    .await
}

/// Runs `command dir` through the user's login shell, detached from this
/// process: its own session, no stdin, stdout or stderr.
fn spawn_detached(command: &str, dir: &std::path::Path) -> std::io::Result<()> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/bin/bash".to_string());
    let mut cmd = std::process::Command::new(shell);
    // `$1` is the folder, so no quoting of its path is needed.
    cmd.args(["-lic", &format!("{command} \"$1\""), "amd"])
        .arg(dir)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: `setsid` is async-signal-safe and touches no memory.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    let mut child = cmd.spawn()?;
    // Reaped on a thread so it doesn't linger as a zombie while amdui runs.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
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

    /// Ticket 188: the editor command gets the project folder as its
    /// argument and runs, in a session of its own.
    #[cfg(unix)]
    #[test]
    fn the_editor_is_started_detached_with_the_folder() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("fake editor.sh");
        std::fs::write(&script, "#!/bin/sh\ncp /proc/self/stat \"$1/ran\"\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let dir = tmp.path().join("project dir");
        std::fs::create_dir(&dir).unwrap();
        spawn_detached(&format!("'{}'", script.display()), &dir).unwrap();
        let ran = dir.join("ran");
        for _ in 0..100 {
            if ran.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(ran.exists(), "the editor never ran");
    }

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
        let uses: Vec<(String, usize, bool, bool)> = summary
            .tags
            .iter()
            .map(|t| (t.name.clone(), t.count, t.listed, t.reserved))
            .collect();
        // DRAFT and TRIAGE are built in (tickets 114, 158): always there,
        // never listed.
        assert_eq!(
            uses,
            [
                ("DRAFT".to_string(), 0, false, true),
                ("FUTURE".to_string(), 1, true, false),
                ("MISC".to_string(), 1, false, false),
                ("OLD".to_string(), 0, true, false),
                ("TRIAGE".to_string(), 0, false, true),
            ]
        );
        assert!(summary.tags_only);
    }

    /// Ticket 120: the Repository filter and fields hold the project's
    /// declared repositories plus any a ticket names, so one that
    /// `amd deregister` takes off the project leaves them, unless a ticket
    /// still names it.
    #[test]
    fn a_repository_leaves_the_lists_when_undeclared_unless_a_ticket_names_it() {
        let (_dir, project) = project();
        let settings = project.repos[0].1.repo_root().join(".settings");
        let listed = |p: &Project| -> Vec<String> {
            p.summary()
                .unwrap()
                .ticket_repos
                .into_iter()
                .map(|r| r.name)
                .collect()
        };

        std::fs::write(&settings, "commit = off\nrepos = site\n").unwrap();
        assert_eq!(listed(&project), ["site"]);

        std::fs::write(&settings, "commit = off\n").unwrap();
        assert!(listed(&project).is_empty(), "undeclared and unnamed: gone");

        let file = new_ticket(&project, "About the site");
        let path = project.repos[0].1.root.join("backlog").join(&file);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("repository: \"\""), "{text}");
        std::fs::write(
            &path,
            text.replace("repository: \"\"", "repository: \"site\""),
        )
        .unwrap();
        assert_eq!(listed(&project), ["site"], "a ticket still names it");
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

        // Into todo takes no lease (ticket 156): todo only means it's next.
        move_in(&project, &a, &backlog, &todo, false).unwrap();
        let summary = project.summary().unwrap();
        assert!(summary.columns[1].tickets[0].lock.is_none());

        // Into doing takes it for the mover; back to todo gives it back.
        let doing = column(UiColumn::Doing);
        move_in(&project, &a, &todo, &doing, false).unwrap();
        let lock = project.summary().unwrap().columns[2].tickets[0]
            .lock
            .clone()
            .unwrap();
        assert!(lock.mine && lock.by == "t@t.co", "{lock:?}");
        move_in(&project, &a, &doing, &todo, false).unwrap();
        assert!(
            project.summary().unwrap().columns[1].tickets[0]
                .lock
                .is_none()
        );

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
