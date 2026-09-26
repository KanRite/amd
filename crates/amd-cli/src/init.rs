//! `amd init <project>` — create or connect a project's dedicated tickets
//! *and* documents repositories, or (`--local-only`) just scaffold the board
//! here.
//!
//! Two sibling repositories, not one (`041-rename-init-file-name` in
//! `agile-md-tickets`): `{project}-tickets` behaves exactly as it always
//! has — a board, direct pushes, no PR. `{project}-documents` is new — ADRs,
//! RFCs and docs, PR-reviewed. One repository can't carry both policies at
//! once, since GitHub branch protection is repository-wide, not path-scoped.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use agile_md::board::Board;
use agile_md::{board_settings, git, registry::Registry};

use crate::github;

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

    let tickets = init_tickets(&base, &project, org.as_deref())?;
    let documents = init_documents(&base, &project, org.as_deref())?;

    println!("Initialised project '{project}':");
    println!("  tickets:   {}", tickets.display());
    if let Some(url) = git::remote_url(&tickets) {
        println!("             {url}");
    }
    println!("  documents: {}", documents.display());
    if let Some(url) = git::remote_url(&documents) {
        println!("             {url}");
    }
    Ok(())
}

/// Creates or connects `{project}-tickets`, scaffolds the board, registers
/// it, and — for a freshly created repository — sets its (lighter) branch
/// protection tier. Unchanged in spirit from before this repository had a
/// documents sibling; only pulled out of `run()` so both repositories share
/// the same creation path.
fn init_tickets(base: &Path, project: &str, org: Option<&str>) -> Result<std::path::PathBuf> {
    let target = base.join(format!("{project}-tickets"));
    let full_name = format!("{}/{project}-tickets", owner_for(project, org));

    let created_new = if target.exists() {
        warn_if_already_present(&target)?;
        false
    } else {
        create_and_clone(base, &full_name, &target)?;
        git::set_head_branch(&target, "main")?;
        true
    };

    let board = Board::in_repo(&target);
    board.create()?;
    set_project(&board, project)?;
    // The one board shape auto-commit is actually for (`board_settings.rs`'s
    // own doc comment): a dedicated, board-only repository with nothing
    // else in it to compete with for what gets committed. Off is the
    // default everywhere else, including a board scaffolded by
    // `--local-only`, which is exactly the "shares a tree with code" shape
    // this must not fire for.
    let mut settings = board_settings::load(&board);
    settings.set_commit(true);
    settings.save(&board)?;
    scaffold_changelog_workflow(&target)?;

    if created_new {
        git::commit_all(&target, &format!("Initialise {project} tickets board"))?;
        git::push(&target)?;
        if let github::ProtectionOutcome::Unavailable = github::protect_main(&full_name, false)? {
            eprintln!(
                "warning: {full_name} has no branch protection — GitHub Pro is required for a \
                 private repository on this plan. Direct pushes still work; force-pushes and \
                 branch deletion just aren't blocked."
            );
        }
    }

    let mut registry = Registry::load();
    registry.add(&target)?;
    registry.save()?;

    Ok(target)
}

/// Creates or connects `{project}-documents`: `adr/`, `rfc/`, `docs/`,
/// `README.md`, `CHANGELOG.md`, plus the same changelog workflow the
/// tickets repository gets. Not a `Board` — no `tasks/`, no counter, no
/// lease fields — and deliberately not added to `Registry`, which is
/// specifically for board aggregation (`amdm`); a documents repository
/// isn't a board.
fn init_documents(base: &Path, project: &str, org: Option<&str>) -> Result<std::path::PathBuf> {
    let target = base.join(format!("{project}-documents"));
    let full_name = format!("{}/{project}-documents", owner_for(project, org));

    let created_new = if target.exists() {
        warn_if_already_present(&target)?;
        false
    } else {
        create_and_clone(base, &full_name, &target)?;
        git::set_head_branch(&target, "main")?;
        true
    };

    scaffold_documents(&target, project)?;
    scaffold_changelog_workflow(&target)?;

    if created_new {
        git::commit_all(&target, &format!("Initialise {project} documents"))?;
        git::push(&target)?;
        if let github::ProtectionOutcome::Unavailable = github::protect_main(&full_name, true)? {
            eprintln!(
                "warning: {full_name} has no branch protection — GitHub Pro is required for a \
                 private repository on this plan, so PR review is NOT enforced yet. Anyone with \
                 push access can commit straight to main until this repository (or its owning \
                 account/org) is upgraded — set protection up by hand once it is, or treat \
                 review as a convention rather than a guarantee until then."
            );
        }
    }

    Ok(target)
}

fn warn_if_already_present(target: &Path) -> Result<()> {
    if !target.join(".git").exists() {
        bail!(
            "{} already exists and isn't a git repository — remove it or choose a \
             different project name",
            target.display()
        );
    }
    if git::remote_url(target).is_none() {
        eprintln!(
            "warning: {} has no 'origin' remote — it isn't visible to a team yet",
            target.display()
        );
    }
    Ok(())
}

/// `adr/`, `rfc/`, `docs/`, `README.md`, an empty `CHANGELOG.md` header, and
/// a `.settings` file carrying just the declared `project` — reusing
/// `board_settings::Config`'s file format for consistency with the tickets
/// repository, without the ticket-counter/lease keys that don't apply to a
/// repository with no board.
fn scaffold_documents(target: &Path, project: &str) -> Result<()> {
    for dir in ["adr", "rfc", "docs"] {
        std::fs::create_dir_all(target.join(dir))
            .with_context(|| format!("creating {}/{dir}", target.display()))?;
        std::fs::write(target.join(dir).join(".gitkeep"), "")?;
    }
    let readme = target.join("README.md");
    if !readme.exists() {
        std::fs::write(
            &readme,
            format!("# {project} documents\n\nADRs, RFCs and docs for {project}.\n"),
        )?;
    }
    let changelog = target.join("CHANGELOG.md");
    if !changelog.exists() {
        std::fs::write(&changelog, "# Changelog\n\n")?;
    }
    let settings_path = target.join(".settings");
    if !settings_path.exists() {
        std::fs::write(&settings_path, format!("project = {project}\n"))?;
    }
    Ok(())
}

/// `.github/workflows/changelog.yml`: on every push to `main`, installs
/// `amd` and runs `amd changelog`, which regenerates and pushes
/// `CHANGELOG.md` itself if it changed. Shared by both repositories — the
/// workflow doesn't need to know which kind of repository it's running in,
/// since `amd changelog` works the same way in either.
fn scaffold_changelog_workflow(target: &Path) -> Result<()> {
    let dir = target.join(".github/workflows");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join("changelog.yml");
    if path.exists() {
        return Ok(());
    }
    std::fs::write(&path, CHANGELOG_WORKFLOW).with_context(|| format!("writing {}", path.display()))
}

const CHANGELOG_WORKFLOW: &str = r#"name: changelog
on:
  push:
    branches: [main]

permissions:
  contents: write

jobs:
  changelog:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: curl -fsSL https://raw.githubusercontent.com/agile-md/agile-md/v6/install.sh | bash -s -- --dir "$HOME/.local/bin"
      - run: echo "$HOME/.local/bin" >> "$GITHUB_PATH"
      - run: amd changelog
"#;

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

/// Which GitHub owner (organisation or personal account) both repositories
/// are created under: `--org`, when given, otherwise the project name
/// itself. On GitHub, an owner is always either an org or a user account,
/// and the project name is expected to name one of those directly — not
/// `gh`'s own default (the CLI's authenticated account), which has no
/// reason to match the project at all.
fn owner_for<'a>(project: &'a str, org: Option<&'a str>) -> &'a str {
    org.unwrap_or(project)
}

/// Creates `full_name` and clones it as a child of `base` (the current
/// directory — `run()` has already refused to get here from inside an
/// existing repository), at `target`. Recovers from a previous run that
/// died after creation but before cloning by cloning the existing remote
/// instead of failing.
fn create_and_clone(base: &Path, full_name: &str, target: &Path) -> Result<()> {
    let create = Command::new("gh")
        .current_dir(base)
        .args(["repo", "create", full_name, "--private", "--clone"])
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
            full_name,
            target.to_str().unwrap_or(full_name),
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

    #[test]
    fn scaffold_documents_creates_the_three_directories_and_files() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("proj-documents");
        std::fs::create_dir_all(&target).unwrap();
        scaffold_documents(&target, "proj").unwrap();

        for sub in ["adr", "rfc", "docs"] {
            assert!(target.join(sub).is_dir(), "{sub} missing");
        }
        assert!(target.join("README.md").exists());
        assert!(target.join("CHANGELOG.md").exists());
        let settings = std::fs::read_to_string(target.join(".settings")).unwrap();
        assert_eq!(settings, "project = proj\n");
    }

    #[test]
    fn scaffold_documents_never_overwrites_an_existing_readme() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("proj-documents");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("README.md"), "hand-written\n").unwrap();
        scaffold_documents(&target, "proj").unwrap();
        assert_eq!(
            std::fs::read_to_string(target.join("README.md")).unwrap(),
            "hand-written\n"
        );
    }

    #[test]
    fn scaffold_changelog_workflow_writes_the_workflow_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("repo");
        std::fs::create_dir_all(&target).unwrap();
        scaffold_changelog_workflow(&target).unwrap();
        let workflow = target.join(".github/workflows/changelog.yml");
        assert!(workflow.exists());
        assert!(
            std::fs::read_to_string(&workflow)
                .unwrap()
                .contains("amd changelog")
        );
    }

    #[test]
    fn scaffold_changelog_workflow_never_overwrites_a_hand_edited_one() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("repo");
        std::fs::create_dir_all(target.join(".github/workflows")).unwrap();
        std::fs::write(target.join(".github/workflows/changelog.yml"), "custom\n").unwrap();
        scaffold_changelog_workflow(&target).unwrap();
        assert_eq!(
            std::fs::read_to_string(target.join(".github/workflows/changelog.yml")).unwrap(),
            "custom\n"
        );
    }
}
