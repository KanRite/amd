//! `amd project` — the projects in this folder and which one `amd` uses
//! (ticket 093). Runs before any board is opened: it has to work exactly
//! when discovery can't decide on its own.

use std::path::Path;

use anyhow::{Result, bail};

use agile_md::board::Board;
use agile_md::current_project::{self, ProjectEntry};
use agile_md::transition::Event;

use crate::form;

/// `amd project`: list them, marking the current one.
pub fn list() -> Result<()> {
    let dir = current_project::project_dir()?;
    let projects = current_project::projects(&dir);
    if projects.is_empty() {
        println!("no projects in {}", dir.display());
        return Ok(());
    }
    let current = current_project::current(&dir);
    let width = projects.iter().map(|p| p.name.len()).max().unwrap_or(0);
    println!("projects in {}:", dir.display());
    for project in &projects {
        let marker = if current.as_deref() == Some(project.name.as_str()) {
            '*'
        } else {
            ' '
        };
        let archived = if agile_md::archive::is_archived(&Board::in_repo(&project.tickets)) {
            "  (archived)"
        } else {
            ""
        };
        println!(
            "{marker} {:<width$}  {}{archived}",
            project.name,
            repositories(&dir, project)
        );
    }
    if current.is_none() {
        println!("no current project — choose one with `amd project use <name>`");
    }
    Ok(())
}

/// `amd project use [name]`: make one current; with no name, pick from a
/// list in a terminal.
pub fn use_project(name: Option<String>) -> Result<()> {
    let dir = current_project::project_dir()?;
    let projects = current_project::projects(&dir);
    let names: Vec<String> = projects.iter().map(|p| p.name.clone()).collect();
    if names.is_empty() {
        bail!("no projects in {}", dir.display());
    }
    let name = match name {
        Some(name) => name,
        None if form::available() => {
            let default = current_project::current(&dir).unwrap_or_else(|| names[0].clone());
            form::select("Project", names.clone(), &default)?
        }
        None => bail!(
            "usage: amd project use <name> (one of: {})",
            names.join(", ")
        ),
    };
    if !names.contains(&name) {
        bail!(
            "no project '{name}' in {} (one of: {})",
            dir.display(),
            names.join(", ")
        );
    }
    current_project::write(&dir, &name)?;
    println!("current project: {name}");
    Ok(())
}

/// `amd project archive|unarchive [name]` (tickets 157, 263): the named
/// project in this folder, else the one amd uses here. Always on GitHub
/// too; archiving also takes it off amd on this machine.
pub fn archive(name: Option<String>, on: bool) -> Result<()> {
    let board = match name {
        Some(name) => {
            let dir = current_project::project_dir()?;
            let projects = current_project::projects(&dir);
            match projects.iter().find(|p| p.name == name) {
                Some(project) => Board::in_repo(&project.tickets),
                None => bail!(
                    "no project '{name}' in {} (one of: {})",
                    dir.display(),
                    projects
                        .iter()
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        }
        None => Board::discover()?,
    };
    if !board.root.is_dir() {
        bail!("no board at {}", board.root.display());
    }
    agile_md::project_trust::check(&board)?;
    let report = |events: Vec<Event>| {
        for event in events {
            match event {
                Event::Info(text) => println!("{text}"),
                Event::Warn(text) => eprintln!("amd: {text}"),
            }
        }
    };
    report(agile_md::sync::catch_up(
        &board,
        agile_md::sync::Purpose::Write,
    )?);
    report(agile_md::archive::set_archived(
        &board,
        on,
        true,
        &mut agile_md::archive::gh,
        &mut agile_md::archive::forget,
    )?);
    // Back on amd here (ticket 263): archiving took it off this machine.
    if !on && !agile_md::archive::is_archived(&board) {
        let root = board.repo_root().to_path_buf();
        crate::init::register(&root)?;
        let project = agile_md::current_project::project_name(&root);
        if let Some(dir) = root.parent() {
            let documents = dir.join(format!("{project}-documents"));
            if documents.join(".git").exists() {
                crate::init::register(&documents)?;
            }
        }
    }
    Ok(())
}

/// Says so when `init`/`join` adding `added` to `dir` made it a folder of
/// several projects, and the one already here was kept current.
pub fn keep_existing(dir: &Path, added: &str) -> Result<()> {
    if let Some(kept) = current_project::keep_existing(dir, added)? {
        println!(
            "two projects here now — current is still {kept}; `amd project use {added}` to switch"
        );
    }
    Ok(())
}

/// `agile-md-tickets, agile-md-documents` — relative to the folder, since
/// that's where the list was asked for.
fn repositories(dir: &Path, project: &ProjectEntry) -> String {
    [Some(&project.tickets), project.documents.as_ref()]
        .into_iter()
        .flatten()
        .map(|path| path.strip_prefix(dir).unwrap_or(path).display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}
