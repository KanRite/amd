//! `amd init <project>` — create or connect a project's dedicated tickets
//! *and* documents repositories, or (`--local-only`) just scaffold the
//! board here.
//!
//! Two sibling repositories, not one (`rfc/0003-ticket-repository-topology-and-team-collaboration.md`
//! in `agile-md-documents`): `{project}-tickets` behaves exactly as it
//! always has — a board, direct pushes, no PR. `{project}-documents` is
//! new — ADRs, RFCs, docs, and (ticket 057) the settings that are worth a
//! review before they take effect — PR-reviewed. One repository can't
//! carry both policies at once, since GitHub branch protection is
//! repository-wide, not path-scoped.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use agile_md::board::Board;
use agile_md::{alias, board_settings, git, registry::Registry};

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

    // Tickets first, documents second: `init_tickets`'s own `set_project`
    // runs before documents exists to be found (ticket 057's sibling
    // derivation needs the directory already there), so `project` lands in
    // the tickets repo's own file too, not just documents's. Harmless —
    // once both repos exist, `board_settings::load` always prefers
    // documents's copy — but worth knowing before "why is `project` in
    // both places" looks like a bug.
    let tickets = init_tickets(&base, &project, org.as_deref())?;
    let documents = init_documents(&base, &project, org.as_deref())?;

    println!("Initialised project '{project}':");
    println!("  tickets:    {}", tickets.display());
    if let Some(url) = git::remote_url(&tickets) {
        println!("              {url}");
    }
    println!("  documents: {}", documents.display());
    if let Some(url) = git::remote_url(&documents) {
        println!("              {url}");
    }
    crate::project::keep_existing(&base, &project)
}

/// Creates or connects `{project}-tickets`, scaffolds the board, registers
/// it, aliases it `tickets`, and — for a freshly created repository — sets
/// its (lighter) branch protection tier. Unchanged in behaviour from
/// before this repository had a documents sibling; only pulled out of
/// `run()` so both repositories share the same creation path.
fn init_tickets(base: &Path, project: &str, org: Option<&str>) -> Result<PathBuf> {
    let target = base.join(format!("{project}-tickets"));
    let full_name = format!("{}/{project}-tickets", owner_for(project, org));

    let created_new = if target.exists() {
        warn_if_already_present(&target)?;
        false
    } else {
        create_and_clone(base, &full_name, &target)?;
        // A freshly cloned, still-empty repository has no commits yet, so
        // its branch name comes from the local git's own defaultBranch
        // config rather than anything GitHub set — force it to `main`
        // before the first commit, regardless of that config.
        git::set_head_branch(&target, "main")?;
        true
    };

    let board = Board::in_repo(&target);
    board.create()?;
    set_project(&board, project)?;

    if created_new {
        git::commit_all(&target, &format!("Initialise {project} tickets board"))?;
        git::push(&target)?;
        warn_if_protection_unavailable(&full_name, github::protect_main(&full_name, false)?);
    }

    register_and_alias(&target, "tickets")?;
    Ok(target)
}

/// Creates or connects `{project}-documents`: `adr/`, `rfc/`, `docs/`,
/// `README.md`, `CHANGELOG.md`, and a `.settings/` **directory** (ticket
/// 057's shape — one small file per key, so each setting reviews as its
/// own legible diff) holding just `project` for now. `repos`/`wip-limit`/
/// `lease-hours` (057) stay unset here — a team sets them later via a PR,
/// same as any other documents change. Registered and aliased
/// `documents`, same as tickets, since ticket 057's settings-reader needs
/// this repository to be locatable on disk the same way.
fn init_documents(base: &Path, project: &str, org: Option<&str>) -> Result<PathBuf> {
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

    if created_new {
        git::commit_all(&target, &format!("Initialise {project} documents"))?;
        git::push(&target)?;
        warn_if_protection_unavailable(&full_name, github::protect_main(&full_name, true)?);
    }

    register_and_alias(&target, "documents")?;
    Ok(target)
}

/// Registers `target` locally and assigns it `alias`, warning (not
/// failing `init`) if the alias is already someone else's — 048's own
/// "must not silently overwrite an existing alias assignment".
pub(crate) fn register_and_alias(target: &Path, alias_name: &str) -> Result<()> {
    let mut registry = Registry::load();
    registry.add(target)?;
    registry.save()?;

    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut aliases = alias::Aliases::load();
    match aliases.set(&registry, &name, alias_name) {
        Ok(()) => aliases.save()?,
        Err(err) => eprintln!("warning: could not assign alias '{alias_name}': {err:#}"),
    }
    Ok(())
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

fn warn_if_protection_unavailable(full_name: &str, outcome: github::ProtectionOutcome) {
    if let github::ProtectionOutcome::Unavailable = outcome {
        eprintln!(
            "warning: {full_name} has no branch protection — GitHub Pro is required for a \
             private repository on this plan. Direct pushes still work; nothing beyond a \
             force-push or deletion is blocked, and (for documents) review is a convention \
             for now, not something GitHub enforces yet."
        );
    }
}

/// `adr/`, `rfc/`, `docs/`, `README.md`, an empty `CHANGELOG.md` header, and
/// a `.settings/` directory carrying just the declared `project` — the
/// three fields ticket 057 also moves here (`repos`/`wip-limit`/
/// `lease-hours`) are left for a team to set later via a PR, not written
/// with defaults up front.
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
            format!(
                "# {project} documents\n\nADRs, RFCs, docs, and reviewed settings for {project}.\n"
            ),
        )?;
    }
    let changelog = target.join("CHANGELOG.md");
    if !changelog.exists() {
        std::fs::write(&changelog, "# Changelog\n\n")?;
    }
    let settings_dir = target.join(".settings");
    std::fs::create_dir_all(&settings_dir)
        .with_context(|| format!("creating {}", settings_dir.display()))?;
    let project_file = settings_dir.join("project");
    if !project_file.exists() {
        std::fs::write(&project_file, format!("{project}\n"))?;
    }
    Ok(())
}

/// `--local-only`: today's plain `amd init` behaviour, plus recording the
/// declared project name.
fn init_here(project: &str) -> Result<()> {
    let board = Board::locate()?;
    board.create()?;
    set_project(&board, project)?;
    // Best-effort, same as the non-local-only path above: registering this
    // machine's own copy of the board it was just created in, so it's
    // findable without a separate `amd register` immediately afterward.
    let mut registry = Registry::load();
    if registry.add(board.repo_root()).is_ok() {
        let _ = registry.save();
    }
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
pub(crate) fn owner_for<'a>(project: &'a str, org: Option<&'a str>) -> &'a str {
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
        assert_eq!(
            std::fs::read_to_string(target.join(".settings").join("project")).unwrap(),
            "proj\n"
        );
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
    fn scaffold_documents_never_overwrites_an_existing_project_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("proj-documents");
        std::fs::create_dir_all(target.join(".settings")).unwrap();
        std::fs::write(target.join(".settings").join("project"), "hand-edited\n").unwrap();
        scaffold_documents(&target, "proj").unwrap();
        assert_eq!(
            std::fs::read_to_string(target.join(".settings").join("project")).unwrap(),
            "hand-edited\n"
        );
    }
}
