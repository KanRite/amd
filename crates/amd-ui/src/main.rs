//! amdui — the agile-md desktop board.
//!
//! The same window `amd gui` opens, with a name of its own so it can be
//! launched the way an app is launched: from a dock, a runner, a shortcut, or
//! just by typing `amdui`. Everything it does goes through the library, so the
//! command line and the board remain one implementation.
//!
//! This binary exists only when the `gui` feature is on (see `required-features`
//! in Cargo.toml), which is why nothing here is behind a `cfg`.

use std::process::ExitCode;

const USAGE: &str = "\
amdui — the agile-md desktop board

Usage: amdui [OPTIONS]

Options:
  -h, --help     Print this help
  -V, --version  Print the version

The board draws every repository on the registry, so it runs from anywhere —
you don't have to be standing in one. `amd repos` lists them, `amd` is the
command line, and `amd gui` opens this same window.";

fn main() -> ExitCode {
    // Deliberately no clap: a window takes no arguments, and the two flags
    // everyone tries anyway are cheaper to answer here than to pull a parser
    // in for.
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => {}
        Some("-h" | "--help") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some("-V" | "--version") => {
            println!("amdui {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some(other) => {
            eprintln!("amdui: unexpected argument '{other}'\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    }

    match agile_md::gui::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // Matches `amd`'s `amd: {err:#}`, so a failure reads the same
            // whichever front end hit it.
            eprintln!("amdui: {err:#}");
            ExitCode::FAILURE
        }
    }
}
