//! Spike (ticket 078): does `amd-lib` drop into a Tauri backend the same
//! way it already drops into `amd-cli`/`amd-ui` (docs/adr/0004's "one
//! library, two interfaces" — this is the third)? Read-only board view
//! only, on purpose — no drag-and-drop, no ticket editor, none of
//! `amdui`'s recent rich-text/field work re-implemented. If this goes
//! further, those are separate, deliberate follow-on decisions.

use agile_md::board::{Board, Column};
use serde::Serialize;

#[derive(Serialize)]
struct CardView {
    id: String,
    title: String,
    points: Option<String>,
    assignee: Option<String>,
    tags: Vec<String>,
}

#[derive(Serialize)]
struct ColumnView {
    name: String,
    cards: Vec<CardView>,
}

#[derive(Serialize)]
struct BoardView {
    project: String,
    columns: Vec<ColumnView>,
}

/// Everything `Board::discover` can go wrong for a `.to_string()`'d
/// `anyhow::Error`: `Result<T, String>` is what Tauri turns into a
/// rejected `invoke()` promise on the JS side (per the Tauri skill).
#[tauri::command]
fn board() -> Result<BoardView, String> {
    let board = Board::discover().map_err(|err| err.to_string())?;
    let mut columns = Vec::new();
    for column in Column::ALL {
        let tasks = board.tasks_in(column).map_err(|err| err.to_string())?;
        let cards = tasks
            .into_iter()
            .map(|task| CardView {
                id: task.id_display(),
                title: task.title(),
                points: task.points(),
                assignee: task.assignee(),
                tags: task.tags(),
            })
            .collect();
        columns.push(ColumnView {
            name: column.as_str().to_string(),
            cards,
        });
    }
    Ok(BoardView {
        project: board.name(),
        columns,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![board])
        .run(tauri::generate_context!())
        .expect("error while building tauri application");
}
