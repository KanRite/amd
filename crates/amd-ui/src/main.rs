//! amdui — the agile-md desktop board.
//!
//! The same window `amd gui` opens, with a name of its own so it can be
//! launched the way an app is launched: from a dock, a runner, a shortcut,
//! or just by typing `amdui`. Everything it does goes through the library,
//! so the command line and the board remain one implementation.

use std::process::ExitCode;

const USAGE: &str = "\
amdui — the agile-md desktop board

Usage: amdui [OPTIONS]

Options:
  -h, --help     Print this help
  -V, --version  Print the version

The board resolves the current project from where it's launched (a plain
directory holding a project's sibling repos, or a repo inside one), plus
whatever else that project has registered — `amd register`/`amd deregister`
manage that. Launched from anywhere else, it opens the board it showed last.
`amd` is the command line, `amd gui` opens this same window.";

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
    amd_ui::run();
    ExitCode::SUCCESS
}
