//! The terminal board.
//!
//! Four columns side by side, one ticket selected at a time, moved with the
//! keyboard through the same `agile_md::transition` the command line uses —
//! so a move from here takes the lease, switches the branch and auto-commits
//! exactly as `amd todo`/`doing`/`done` would. Filters live on a screen of
//! their own rather than a toolbar.
//!
//! `app` is the state and the key handling, with no drawing and no terminal
//! in it, so it is tested without either; `ui` only draws what `app` holds.

pub mod app;
#[cfg(test)]
mod fixture;
pub mod form;
pub mod ui;

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use app::App;

/// Open the board and run until the user quits.
pub fn run() -> Result<()> {
    // Load before taking over the terminal, so a "no board here" error prints
    // as an ordinary error rather than flashing on the alternate screen.
    let mut app = App::load()?;
    // git's progress and error text would land in the middle of the screen;
    // failures still reach the status line through the error they return.
    agile_md::git::set_quiet(true);
    // `ratatui::init` enters raw mode and the alternate screen and installs a
    // panic hook that restores both. That hook is not optional here: release
    // builds use `panic = "abort"`, so nothing unwinds to restore the terminal.
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.quit {
        terminal.draw(|frame| ui::draw(frame, app))?;
        if let Event::Key(key) = event::read()?
            // Windows also reports key releases; acting on both would move
            // every ticket twice.
            && key.kind == KeyEventKind::Press
        {
            app.handle_key(key);
        }
    }
    Ok(())
}
