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

/// Creates a ticket in `repo`'s backlog (the board's own when `None`).
pub async fn create_ticket(title: &str, repo: Option<&str>) -> Result<Created, String> {
    call("create_ticket", &TitleArgs { title, repo }).await
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
