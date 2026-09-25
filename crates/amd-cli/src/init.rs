//! `amd init <project>` — create or connect a project's dedicated tickets
//! repository, or (`--local-only`) just scaffold the board here.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use agile_md::board::Board;
use agile_md::{board_settings, git, registry::Registry};

pub fn run(project: String, local_only: bool, org: Option<String>) -> Result<()> {
    board_settings::validate_project_name(&project)?;

    if local_only {
        return init_here(&project);
    }

    // Guard, not a placement trick: an earlier version tried to infer a
    // sensible location (a sibling of whatever repo you happened to be
    // standing in) when run from inside one. That inference was a real
    // source of confusion — better to refuse outright and say so, than to
    // guess where you meant. `amd init <project>` must be run from a plain
    // directory; `--local-only` is the command for "inside a repo".
    if let Some(repo_root) = git::repo_root() {
        bail!(
            "amd init <project> must not be run inside a git repository (you're inside {}) — \
             cd to a plain directory first, or use --local-only to scaffold the board here instead",
            repo_root.display()
        );
    }
    let base = std::env::current_dir().context("reading the current directory")?;
    let target = base.join(format!("{project}-tickets"));

    let created_new = if target.exists() {
        if !target.join(".git").exists() {
            bail!(
                "{} already exists and isn't a git repository — remove it or choose a \
                 different project name",
                target.display()
            );
        }
        if git::remote_url(&target).is_none() {
            eprintln!(
                "warning: {} has no 'origin' remote — it isn't visible to a team yet",
                target.display()
            );
        }
        false
    } else {
        create_and_clone(&base, &project, org.as_deref(), &target)?;
        // A freshly cloned, still-empty repository has no commits yet, so
        // its branch name comes from the local git's own defaultBranch
        // config rather than anything GitHub set — force it to `main`
        // before the first commit, regardless of that config.
        git::set_head_branch(&target, "main")?;
        true
    };

    let board = Board::in_repo(&target);
    board.create()?;
    set_project(&board, &project)?;

    if created_new {
        git::commit_all(&target, &format!("Initialise {project} tickets board"))?;
        git::push(&target)?;
    }

    let mut registry = Registry::load();
    registry.add(&target)?;
    registry.save()?;

    println!("Initialised project '{project}' at {}", target.display());
    if let Some(url) = git::remote_url(&target) {
        println!("  remote: {url}");
    }
    Ok(())
}

/// `--local-only`: today's plain `amd init` behaviour, plus recording the
/// declared project name.
fn init_here(project: &str) -> Result<()> {
    let board = Board::locate()?;
    board.create()?;
    set_project(&board, project)?;
    agile_md::registry::remember(&board);
    println!("Initialised agile-md board at {}", board.root.display());
    Ok(())
}

fn set_project(board: &Board, project: &str) -> Result<()> {
    let mut settings = board_settings::load(board);
    settings.set_project(project)?;
    settings.save(board)
}

/// Which GitHub owner (organisation or personal account) `{project}-tickets`
/// is created under: `--org`, when given, otherwise the project name itself.
/// On GitHub, an owner is always either an org or a user account, and the
/// project name is expected to name one of those directly — not `gh`'s own
/// default (the CLI's authenticated account), which has no reason to match
/// the project at all.
fn owner_for<'a>(project: &'a str, org: Option<&'a str>) -> &'a str {
    org.unwrap_or(project)
}

/// Creates `{project}-tickets` under `owner_for(project, org)` and clones it
/// as a child of `base` (the current directory — `run()` has already
/// refused to get here from inside an existing repository), at `target`.
/// Recovers from a previous run that died after creation but before cloning
/// by cloning the existing remote instead of failing.
fn create_and_clone(base: &Path, project: &str, org: Option<&str>, target: &Path) -> Result<()> {
    let owner = owner_for(project, org);
    let full_name = format!("{owner}/{project}-tickets");

    let create = Command::new("gh")
        .current_dir(base)
        .args(["repo", "create", &full_name, "--private", "--clone"])
        .output()
        .context("running gh repo create — is the gh CLI installed and authenticated?")?;
    if create.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&create.stderr);
    if !stderr.to_lowercase().contains("already exists") {
        bail!("gh repo create {full_name} failed: {}", stderr.trim());
    }
    // Recoverable: a previous run already created the remote repository —
    // clone it instead, the same way `gh repo create --clone` would have.
    let clone = Command::new("gh")
        .current_dir(base)
        .args([
            "repo",
            "clone",
            &full_name,
            target.to_str().unwrap_or(project),
        ])
        .status()
        .context("running gh repo clone")?;
    if !clone.success() {
        bail!("gh repo clone {full_name} failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_owner_defaults_to_the_project_name() {
        assert_eq!(owner_for("agile-md", None), "agile-md");
    }

    #[test]
    fn an_explicit_org_overrides_the_project_name() {
        assert_eq!(
            owner_for("agile-md", Some("some-other-org")),
            "some-other-org"
        );
    }
}
