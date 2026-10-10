//! Calls into the Tauri backend.
//!
//! `withGlobalTauri` exposes `window.__TAURI__.core.invoke`; this binds it
//! with `wasm-bindgen` and wraps each command in a typed function, so the
//! rest of the UI never touches `JsValue`.

use amd_ui_model::appearance::Appearance;
use amd_ui_model::epic::Place;
use amd_ui_model::wip::WipLimits;
use amd_ui_model::{Board, Created, SyncReport, TicketDocument, TicketTemplate};
use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

async fn call<A: Serialize, R: DeserializeOwned>(cmd: &str, args: &A) -> Result<R, String> {
    let args = serde_wasm_bindgen::to_value(args).map_err(|e| e.to_string())?;
    let out = invoke(cmd, args)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
    serde_wasm_bindgen::from_value(out).map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct NoArgs {}

#[derive(Serialize)]
struct SourceArgs<'a> {
    source: &'a str,
}

#[derive(Serialize)]
struct PathArgs<'a> {
    path: &'a str,
}

pub async fn load_board() -> Result<Board, String> {
    call("load_board", &NoArgs {}).await
}

#[derive(Serialize)]
struct OnArgs {
    on: bool,
}

pub async fn auto_commit() -> Result<bool, String> {
    call("auto_commit", &NoArgs {}).await
}

pub async fn set_auto_commit(on: bool) -> Result<bool, String> {
    call("set_auto_commit", &OnArgs { on }).await
}

pub async fn get_template() -> Result<TicketTemplate, String> {
    call("get_template", &NoArgs {}).await
}

pub async fn preview_template(source: &str) -> Result<String, String> {
    call("preview_template", &SourceArgs { source }).await
}

pub async fn save_template(source: &str) -> Result<TicketTemplate, String> {
    call("save_template", &SourceArgs { source }).await
}

pub async fn reset_template() -> Result<TicketTemplate, String> {
    call("reset_template", &NoArgs {}).await
}

pub async fn tasks_dir() -> Result<Option<String>, String> {
    call("tasks_dir", &NoArgs {}).await
}

pub async fn set_tasks_dir(path: &str) -> Result<Board, String> {
    call("set_tasks_dir", &PathArgs { path }).await
}

#[derive(Serialize)]
struct ProjectArgs<'a> {
    name: &'a str,
}

/// Switches to another project in the same folder (ticket 093).
pub async fn use_project(name: &str) -> Result<Board, String> {
    call("use_project", &ProjectArgs { name }).await
}

/// Clones a declared repository that isn't on this machine (ticket 108);
/// returns any duplicate-clone warnings.
pub async fn clone_repository(name: &str) -> Result<Vec<String>, String> {
    call("clone_repository", &ProjectArgs { name }).await
}

#[derive(Serialize)]
struct MoveArgs<'a> {
    file: &'a str,
    from: &'a Place,
    to: &'a Place,
    force: bool,
}

/// Moves a ticket; `force` overrides a full column's WIP limit.
pub async fn move_ticket(
    file: &str,
    from: &Place,
    to: &Place,
    force: bool,
) -> Result<SyncReport, String> {
    call(
        "move_ticket",
        &MoveArgs {
            file,
            from,
            to,
            force,
        },
    )
    .await
}

#[derive(Serialize)]
struct TicketRef<'a> {
    place: &'a Place,
    file: &'a str,
}

#[derive(Serialize)]
struct SaveArgs<'a> {
    place: &'a Place,
    file: &'a str,
    content: &'a str,
    version: &'a str,
}

#[derive(Serialize)]
struct TitleArgs<'a> {
    title: &'a str,
    repo: Option<&'a str>,
    repository: Option<&'a str>,
    draft: bool,
}

#[derive(Serialize)]
struct NameArgs<'a> {
    name: &'a str,
    repo: Option<&'a str>,
}

#[derive(Serialize)]
struct RenameArgs<'a> {
    repo: Option<&'a str>,
    from: &'a str,
    to: &'a str,
}

pub async fn open_ticket(place: &Place, file: &str) -> Result<TicketDocument, String> {
    call("open_ticket", &TicketRef { place, file }).await
}

/// A summary of what's on disk; it changes whenever the board's files do
/// (ticket 091).
pub async fn board_fingerprint() -> Result<String, String> {
    call("board_fingerprint", &NoArgs {}).await
}

/// Brings the board up to date with its remote (ticket 102); returns any
/// warnings.
pub async fn sync_board() -> Result<Vec<String>, String> {
    call("sync_board", &NoArgs {}).await
}

/// The ticket's current version on disk, or `None` once it has moved or
/// gone.
pub async fn ticket_version(place: &Place, file: &str) -> Result<Option<String>, String> {
    call("ticket_version", &TicketRef { place, file }).await
}

/// Saves a ticket's text; `version` is the fingerprint it was opened with,
/// so a ticket changed since is refused rather than overwritten.
pub async fn save_ticket(
    place: &Place,
    file: &str,
    content: &str,
    version: &str,
) -> Result<SyncReport, String> {
    call(
        "save_ticket",
        &SaveArgs {
            place,
            file,
            content,
            version,
        },
    )
    .await
}

pub async fn archive_ticket(place: &Place, file: &str) -> Result<SyncReport, String> {
    call("archive_ticket", &TicketRef { place, file }).await
}

/// Creates a ticket in `repo`'s backlog (the board's own when `None`),
/// as a draft when `draft` (ticket 114).
pub async fn create_ticket(
    title: &str,
    repo: Option<&str>,
    repository: Option<&str>,
    draft: bool,
) -> Result<Created, String> {
    call(
        "create_ticket",
        &TitleArgs {
            title,
            repo,
            repository,
            draft,
        },
    )
    .await
}

pub async fn create_epic(name: &str, repo: Option<&str>) -> Result<SyncReport, String> {
    call("create_epic", &NameArgs { name, repo }).await
}

pub async fn rename_epic(repo: Option<&str>, from: &str, to: &str) -> Result<SyncReport, String> {
    call("rename_epic", &RenameArgs { repo, from, to }).await
}

#[derive(Serialize)]
struct SprintArgs<'a> {
    name: &'a str,
    days: u32,
    repo: Option<&'a str>,
}

#[derive(Serialize)]
struct StartArgs<'a> {
    repo: Option<&'a str>,
    name: &'a str,
}

pub async fn create_sprint(
    name: &str,
    days: u32,
    repo: Option<&str>,
) -> Result<SyncReport, String> {
    call("create_sprint", &SprintArgs { name, days, repo }).await
}

pub async fn start_sprint(repo: Option<&str>, name: &str) -> Result<SyncReport, String> {
    call("start_sprint", &StartArgs { repo, name }).await
}

#[derive(Serialize)]
struct AppearanceArgs {
    appearance: Appearance,
}

pub async fn appearance() -> Result<Appearance, String> {
    call("appearance", &NoArgs {}).await
}

/// Applies the board's saved zoom and returns it, in percent (ticket 024).
pub async fn zoom() -> Result<u32, String> {
    call("zoom", &NoArgs {}).await
}

#[derive(Serialize)]
struct ZoomArgs {
    percent: u32,
}

/// Zooms the board and remembers the size on this machine.
pub async fn set_zoom(percent: u32) -> Result<u32, String> {
    call("set_zoom", &ZoomArgs { percent }).await
}

pub async fn set_appearance(appearance: Appearance) -> Result<Appearance, String> {
    call("set_appearance", &AppearanceArgs { appearance }).await
}

#[derive(Serialize)]
struct WipArgs<'a> {
    limits: WipLimits,
    repo: Option<&'a str>,
}

pub async fn set_wip_limits(limits: WipLimits, repo: Option<&str>) -> Result<SyncReport, String> {
    call("set_wip_limits", &WipArgs { limits, repo }).await
}

#[derive(Serialize)]
struct TagArgs<'a> {
    tag: &'a str,
}

/// Adds a tag to the project's list (ticket 122); saved in UPPERCASE.
pub async fn add_tag(tag: &str) -> Result<SyncReport, String> {
    call("add_tag", &TagArgs { tag }).await
}

/// Takes an unused tag off the project's list (ticket 122).
pub async fn remove_tag(tag: &str) -> Result<SyncReport, String> {
    call("remove_tag", &TagArgs { tag }).await
}

/// Whether only listed tags may be put on a ticket (ticket 122).
pub async fn set_tags_only(on: bool) -> Result<SyncReport, String> {
    call("set_tags_only", &OnArgs { on }).await
}

#[derive(Serialize)]
struct UnlockArgs<'a> {
    place: &'a Place,
    file: &'a str,
    force: bool,
}

/// Locks a ticket for the user, or renews their own lock.
pub async fn lock_ticket(place: &Place, file: &str) -> Result<SyncReport, String> {
    call("lock_ticket", &TicketRef { place, file }).await
}

/// Releases the user's own lock; `force` breaks someone else's.
pub async fn unlock_ticket(place: &Place, file: &str, force: bool) -> Result<SyncReport, String> {
    call("unlock_ticket", &UnlockArgs { place, file, force }).await
}

#[derive(Serialize)]
struct HoursArgs<'a> {
    hours: u32,
    repo: Option<&'a str>,
}

pub async fn set_lease_hours(hours: u32, repo: Option<&str>) -> Result<SyncReport, String> {
    call("set_lease_hours", &HoursArgs { hours, repo }).await
}

/// The landing page: your work across every project, and the projects
/// (ticket 020).
pub async fn home() -> Result<amd_ui_model::Home, String> {
    call("home", &NoArgs {}).await
}

#[derive(Serialize)]
struct HiddenArgs<'a> {
    root: &'a str,
    hidden: bool,
}

/// Hides a project from My work, or shows it again.
pub async fn set_project_hidden(root: &str, hidden: bool) -> Result<(), String> {
    call("set_project_hidden", &HiddenArgs { root, hidden }).await
}

/// The project's documents repositories and their documents (ticket 129).
pub async fn list_documents() -> Result<Vec<amd_ui_model::DocSource>, String> {
    call("list_documents", &NoArgs {}).await
}

#[derive(Serialize)]
struct DocArgs<'a> {
    source: &'a str,
    path: &'a str,
}

/// A document, rendered for reading.
pub async fn read_document(source: &str, path: &str) -> Result<amd_ui_model::RenderedDoc, String> {
    call("read_document", &DocArgs { source, path }).await
}

#[derive(Serialize)]
struct UrlArgs<'a> {
    url: &'a str,
}

/// Opens a web or mail link outside amdui.
pub async fn open_external(url: &str) -> Result<(), String> {
    call("open_external", &UrlArgs { url }).await
}

#[derive(Serialize)]
struct SharedArgs<'a> {
    name: &'a str,
}

/// Names the documents repository every project shares; empty clears it.
pub async fn set_shared_documents(name: &str) -> Result<SyncReport, String> {
    call("set_shared_documents", &SharedArgs { name }).await
}

#[derive(Serialize)]
struct IssuesArgs<'a> {
    repo: &'a str,
    label: &'a str,
    preview: bool,
}

/// Imports open GitHub issues into the backlog, tagged TRIAGE (ticket 158),
/// or with `preview` only says what it would import.
pub async fn import_issues(
    repo: &str,
    label: &str,
    preview: bool,
) -> Result<amd_ui_model::IssueImport, String> {
    call(
        "import_issues",
        &IssuesArgs {
            repo,
            label,
            preview,
        },
    )
    .await
}
