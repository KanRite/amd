//! Shell completion scripts, generated from the clap command itself.
//!
//! `amd completions [SHELL]` prints the script to stdout; with no argument it
//! works the shell out from `$SHELL`. Generating rather than shipping checked-in
//! scripts means completions can never drift from the CLI — a new subcommand or
//! flag is completable the moment it exists.
//!
//! The value-level completions come from the parsers in `value.rs`: `--type`
//! offers the change types this board accepts, `--template` the built-in ticket
//! types, `ls` its columns.

use std::env;
use std::io;
use std::path::Path;

use anyhow::{Result, bail};
use clap::Command;
use clap_complete::{Shell, generate};

/// Print the completion script for `shell`, or for `$SHELL` when it's `None`.
pub fn print(shell: Option<Shell>, command: &mut Command) -> Result<()> {
    let shell = match shell {
        Some(shell) => shell,
        None => from_env()?,
    };
    let name = command.get_name().to_string();
    generate(shell, command, name, &mut io::stdout());
    // The script goes to stdout so it can be redirected; the hint goes to
    // stderr so it survives the redirect without corrupting the script.
    eprintln!("# install it with:\n# {}", install_hint(shell));
    Ok(())
}

/// Work the shell out from `$SHELL` (`/bin/zsh` -> zsh).
fn from_env() -> Result<Shell> {
    let shell = env::var("SHELL").unwrap_or_default();
    let name = Path::new(&shell)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    match name {
        "bash" => Ok(Shell::Bash),
        "zsh" => Ok(Shell::Zsh),
        "fish" => Ok(Shell::Fish),
        "elvish" => Ok(Shell::Elvish),
        "pwsh" | "powershell" => Ok(Shell::PowerShell),
        "" => bail!("$SHELL isn't set — name the shell: amd completions bash|zsh|fish"),
        other => {
            bail!("don't know how to complete for '{other}' — try: amd completions bash|zsh|fish")
        }
    }
}

/// Where the script wants to live, printed as a hint after generating it.
pub fn install_hint(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => {
            "amd completions bash > /usr/local/etc/bash_completion.d/amd\n\
                        # or: echo 'source <(amd completions bash)' >> ~/.bashrc"
        }
        Shell::Zsh => {
            "amd completions zsh > \"${fpath[1]}/_amd\"\n\
                       # then: rm -f ~/.zcompdump; compinit"
        }
        Shell::Fish => "amd completions fish > ~/.config/fish/completions/amd.fish",
        _ => "redirect the output into your shell's completion directory",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn script(shell: Shell) -> String {
        let mut command = crate::Cli::command();
        let mut out = Vec::new();
        let name = command.get_name().to_string();
        generate(shell, &mut command, name, &mut out);
        String::from_utf8(out).expect("completions are utf-8")
    }

    #[test]
    fn every_shell_gets_a_script_that_knows_the_commands() {
        for (shell, marker) in [
            (Shell::Bash, "complete -F"),
            (Shell::Zsh, "#compdef amd"),
            (Shell::Fish, "complete -c amd"),
        ] {
            let script = script(shell);
            assert!(script.contains(marker), "{shell}: missing {marker}");
            // "start" is deliberately not checked here: `amd group start`
            // would make the substring match trivially, masking the fact
            // that `amd start` itself no longer exists as a top-level command.
            for command in [
                "new",
                "todo",
                "doing",
                "backlog",
                "abandon",
                "link",
                "templates",
                "completions",
            ] {
                assert!(script.contains(command), "{shell}: missing {command}");
            }
        }
    }

    #[test]
    fn scripts_complete_the_values_too() {
        // The parsers advertise their possible values, so `--branch-type <TAB>`
        // offers branch types rather than falling back to filenames.
        //
        // These used to assert on "feat" and "development" — words that only
        // ever appeared in help text describing ticket types that no longer
        // exist. The assertions passed because the documentation was wrong, so
        // pin them to values a parser actually advertises.
        let fish = script(Shell::Fish);
        assert!(fish.contains("bugfix"), "{fish}");
        assert!(fish.contains("ticket"), "{fish}");
        let bash = script(Shell::Bash);
        assert!(bash.contains("todo doing done all"), "{bash}");
        assert!(bash.contains("points epic order title"), "{bash}");
    }
}
