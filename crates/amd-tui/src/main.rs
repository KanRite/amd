//! amd-tui — the agile-md terminal board.
//!
//! The same board `amd tui` opens, with a name of its own so it can be
//! launched by typing `amd-tui`. Everything it does goes through the library,
//! so the command line, the desktop board and this one stay one
//! implementation.

use std::process::ExitCode;

const USAGE: &str = "\
amd-tui — the agile-md terminal board

Usage: amd-tui [OPTIONS]

Options:
  -h, --help     Print this help
  -V, --version  Print the version

Keys (press ? inside for the full list): h/l pick a column, j/k a ticket,
H/L move the ticket a column, Enter opens it, f filters, r reloads, q quits.

The board resolves the current project from where it's launched (a plain
directory holding a project's sibling repos, or a repo inside one), plus
whatever else that project has registered — `amd register`/`amd deregister`
manage that. `amd` is the command line, `amdui` the desktop board.";

fn main() -> ExitCode {
    // Deliberately no clap, for the same reason `amdui` has none: a full-screen
    // board takes no arguments, and the two flags everyone tries anyway are
    // cheaper to answer here than to pull a parser in for.
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => {}
        Some("-h" | "--help") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some("-V" | "--version") => {
            println!("amd-tui {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some(other) => {
            eprintln!("amd-tui: unexpected argument '{other}'\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    }

    match amd_tui::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("amd-tui: {err:#}");
            ExitCode::FAILURE
        }
    }
}
