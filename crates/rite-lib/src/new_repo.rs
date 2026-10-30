//! A new repository for the project (ticket 228): created (or, when it
//! already exists, connected), cloned beside the tickets repository,
//! registered for the project, and, from a ticket, recorded on it. Someone
//! writing a ticket who finds there's nowhere for the work to go doesn't
//! have to leave rite to make one.
//!
//! Where it's made follows the tickets repository's `origin`: on GitHub
//! under the same owner (through `gh`, passed in so it's tested without
//! GitHub), as a bare repository beside a local one (how the tests and the
//! demo are set up), or, with no origin at all, only on this machine.
//! It's cloned from the same place the tickets repository came from
//! (`clone::sibling_url`), so an SSH host alias is kept.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};

use crate::board::Board;
use crate::registry::{self, Registry};
use crate::task::Task;
use crate::{authorship, board_settings, git, github};

/// Runs `gh` with these arguments, giving back what it printed, or an
/// error when it fails.
pub type Gh<'a> = &'a mut dyn FnMut(&[&str]) -> Result<String>;

/// The real `gh`.
pub fn gh(args: &[&str]) -> Result<String> {
    let out = Command::new("gh")
        .args(args)
        .output()
        .context("running gh — is the GitHub CLI installed and signed in?")?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(anyhow!(
            "gh {}: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Where a new repository goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Forge {
    /// On GitHub, under this owner.
    GitHub { owner: String },
    /// A bare repository beside the tickets repository's local origin.
    Local,
    /// The tickets repository has no origin: this machine only.
    None,
}

/// What the dialog offers before anything is made.
#[derive(Clone, Debug)]
pub struct Defaults {
    pub forge: Forge,
    /// The same as the tickets repository's, private when unknown.
    pub private: bool,
}

/// What to make.
#[derive(Clone, Debug, Default)]
pub struct NewRepo {
    pub name: String,
    pub description: String,
    /// `None` is the same as the tickets repository.
    pub private: Option<bool>,
    /// A starter README, when the repository is new and empty.
    pub readme: bool,
    /// The ticket it's for, `(id, title)`, named in the README.
    pub for_ticket: Option<(String, String)>,
}

/// What [`create`] did.
#[derive(Clone, Debug)]
pub struct Made {
    pub name: String,
    pub path: PathBuf,
    /// Made now, rather than an existing one connected.
    pub created: bool,
    pub private: bool,
    pub forge: Forge,
    /// Anything worth saying: protection unavailable, no remote, …
    pub notes: Vec<String>,
}

/// Where the project's new repositories go, from the tickets repository's
/// `origin`. A forge other than GitHub is refused: make it there, then
/// `rite register` it.
pub fn forge(board: &Board) -> Result<Forge> {
    let Some(origin) = git::remote_url(board.repo_root()) else {
        return Ok(Forge::None);
    };
    if git::is_local_path(&origin) {
        return Ok(Forge::Local);
    }
    let identity = git::url_identity(&origin);
    let mut parts = identity.split('/');
    match (parts.next(), parts.next()) {
        (Some("github.com"), Some(owner)) if !owner.is_empty() => Ok(Forge::GitHub {
            owner: owner.to_string(),
        }),
        _ => bail!(
            "rite can only make repositories on GitHub; make it on {}, then `rite register` it",
            identity.split('/').next().unwrap_or("your forge")
        ),
    }
}

/// The dialog's starting point: where it would be made, and whether
/// private (as the tickets repository is).
pub fn defaults(board: &Board, gh: Gh) -> Result<Defaults> {
    let forge = forge(board)?;
    let private = match &forge {
        Forge::GitHub { owner } => {
            let tickets = repo_name_of(board.repo_root());
            gh(&[
                "repo",
                "view",
                &format!("{owner}/{tickets}"),
                "--json",
                "visibility",
                "-q",
                ".visibility",
            ])
            .map(|v| !v.trim().eq_ignore_ascii_case("public"))
            .unwrap_or(true)
        }
        _ => true,
    };
    Ok(Defaults { forge, private })
}

/// Makes (or connects) the repository and clones it beside the tickets
/// repository. The caller then registers the clone on this machine
/// ([`register`]), and declares it for the project ([`declare`]) and records
/// it on a ticket ([`record_on_ticket`]) in its commit.
pub fn create(board: &Board, new: &NewRepo, gh: Gh) -> Result<Made> {
    // Checked as typed: a path or a space isn't a repository name.
    let name = new.name.trim().to_string();
    board_settings::validate_project_name(&name)?;
    let tickets = board.repo_root();
    if name == repo_name_of(tickets) {
        bail!("{name} is the project's tickets repository");
    }
    if let Some(path) = registry::clone_of(&board.root, &name) {
        bail!(
            "{name} is already cloned for this project, at {}",
            path.display()
        );
    }
    let target = registry::project_dir_of(tickets).join(&name);
    if target.symlink_metadata().is_ok() {
        bail!(
            "{} already exists: move it, or `rite register` it if it's the clone",
            target.display()
        );
    }
    let defaults = defaults(board, gh)?;
    let private = new.private.unwrap_or(defaults.private);
    let mut notes = Vec::new();
    let origin = git::remote_url(tickets);
    let url = origin
        .as_deref()
        .map(|o| crate::clone::sibling_url(o, &name));
    let created = match &defaults.forge {
        Forge::GitHub { owner } => {
            let full = format!("{owner}/{name}");
            let exists = gh(&["repo", "view", &full, "--json", "name"]).is_ok();
            if !exists {
                let mut args = vec![
                    "repo",
                    "create",
                    &full,
                    if private { "--private" } else { "--public" },
                ];
                if !new.description.trim().is_empty() {
                    args.extend(["--description", new.description.trim()]);
                }
                gh(&args)?;
            }
            !exists
        }
        Forge::Local => {
            let bare = local(tickets, url.as_deref().unwrap_or_default());
            let exists = bare.exists();
            if !exists {
                run_git(None, &["init", "-q", "--bare", "-b", "main"], Some(&bare))?;
            }
            !exists
        }
        Forge::None => {
            run_git(None, &["init", "-q", "-b", "main"], Some(&target))?;
            notes.push(format!(
                "{name} has no remote: the tickets repository has none to put it beside"
            ));
            true
        }
    };
    if defaults.forge != Forge::None {
        let from = match &defaults.forge {
            Forge::Local => local(tickets, url.as_deref().unwrap_or_default())
                .display()
                .to_string(),
            _ => url.clone().unwrap_or_default(),
        };
        if let Err(err) = git::clone(&from, &target) {
            let _ = fs::remove_dir_all(&target);
            let what = if created { "made" } else { "found" };
            return Err(err.context(format!(
                "{name} was {what} but couldn't be cloned; run this again to connect it"
            )));
        }
    }
    // A new, empty repository gets a README, so it has a main branch.
    let empty = git::head(&target).is_none();
    let mut pushed = false;
    if new.readme && empty {
        let project = board_settings::load(board)
            .project()
            .map(str::to_string)
            .unwrap_or_else(|| repo_name_of(tickets));
        let mut text = format!("# {name}\n\n");
        if !new.description.trim().is_empty() {
            text.push_str(new.description.trim());
            text.push_str("\n\n");
        }
        text.push_str(&format!("Part of the {project} project"));
        match &new.for_ticket {
            Some((id, title)) => text.push_str(&format!("; made for ticket {id}, {title}.\n")),
            None => text.push_str(".\n"),
        }
        fs::write(target.join("README.md"), text).context("writing the README")?;
        run_git(
            Some(&target),
            &["symbolic-ref", "HEAD", "refs/heads/main"],
            None,
        )?;
        // As whoever the tickets repository knows: a machine without a
        // global identity may have one set only there.
        let mut commit = vec!["-c".to_string(), "commit.gpgsign=false".to_string()];
        for key in ["user.name", "user.email"] {
            if let Some(value) = git::config(tickets, key) {
                commit.extend(["-c".to_string(), format!("{key}={value}")]);
            }
        }
        commit.extend(["commit", "-q", "-m", "Start the repository"].map(str::to_string));
        run_git(Some(&target), &["add", "README.md"], None)?;
        let commit: Vec<&str> = commit.iter().map(String::as_str).collect();
        run_git(Some(&target), &commit, None)?;
        if defaults.forge != Forge::None {
            match run_git(Some(&target), &["push", "-q", "-u", "origin", "main"], None) {
                Ok(()) => pushed = true,
                Err(e) => notes.push(format!("the README couldn't be pushed: {e:#}")),
            }
        }
    }
    if let Forge::GitHub { owner } = &defaults.forge
        && created
    {
        let full = format!("{owner}/{name}");
        if pushed {
            match github::protect_main(&full) {
                Ok(github::ProtectionOutcome::Applied) => {}
                Ok(github::ProtectionOutcome::Unavailable) => notes.push(format!(
                    "GitHub doesn't offer branch protection for {full} on this plan"
                )),
                Err(e) => notes.push(format!("main isn't protected on {full}: {e:#}")),
            }
        } else {
            notes.push(format!(
                "main isn't protected on {full}: it has no commits yet"
            ));
        }
    }
    Ok(Made {
        name,
        path: target,
        created,
        private,
        forge: defaults.forge,
        notes,
    })
}

/// Registers the clone on this machine, so this project finds it.
pub fn register(made: &Made) -> Result<()> {
    let mut registry = Registry::load();
    registry.add(&made.path)?;
    registry.save()
}

/// Declares the repository for the project, in the tickets `.settings`, so
/// the whole team has it.
pub fn declare(board: &Board, name: &str) -> Result<()> {
    let mut settings = board_settings::load(board);
    settings.declare_repo(name)?;
    settings.save(board)
}

/// Sets the ticket's repository and says so in its notes, signed and dated:
/// "Created repository acme-payments (private)". A ticket with no branch
/// type is then a draft in the backlog (ticket 066); past it, it can't be,
/// and the repository is made already, so that's only what it returns to
/// say.
pub fn record_on_ticket(board: &Board, task: &Task, made: &Made) -> Result<Option<String>> {
    task.set_repository(task.hash()?, &made.name)?;
    let task = Task::from_path(&task.path, task.column).unwrap_or_else(|| task.clone());
    let who = authorship::current_person(board)
        .map(|p| p.split(" <").next().unwrap_or(&p).to_string())
        .unwrap_or_else(|| "someone".into());
    let today = time::OffsetDateTime::now_utc().date().to_string();
    let body = crate::task::with_note(&task.body()?, &note_for(made, &who, &today));
    task.set_body(task.hash()?, &body)?;
    let kind = task.branch_type().unwrap_or_default();
    match crate::pairing::check(board, &task.id_display(), task.column, &made.name, &kind) {
        Ok(why) => crate::transition::settle_pairing(&task, why),
        Err(err) => Ok(Some(format!(
            "{err:#}; set one with `rite set {} branch-type <type|none>`",
            task.id_display()
        ))),
    }
}

/// The ticket's note: "**Alex Morgan, 2026-10-16:** Created repository
/// acme-payments (private)."
pub fn note_for(made: &Made, who: &str, today: &str) -> String {
    let what = if made.created { "Created" } else { "Connected" };
    let how = match (&made.forge, made.private) {
        (Forge::None, _) => "on this machine only",
        (_, true) => "private",
        (_, false) => "public",
    };
    format!(
        "**{who}, {today}:** {what} repository {} ({how}).",
        made.name
    )
}

/// The bare repository a local origin's sibling URL names.
fn local(tickets: &Path, url: &str) -> PathBuf {
    let path = Path::new(url);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        tickets.join(path)
    }
}

fn repo_name_of(root: &Path) -> String {
    root.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `git` in `dir` (or with `target` as the last argument), failing on a
/// non-zero exit.
fn run_git(dir: Option<&Path>, args: &[&str], target: Option<&Path>) -> Result<()> {
    let mut cmd = Command::new("git");
    if let Some(dir) = dir {
        cmd.arg("-C").arg(dir);
    }
    cmd.args(args);
    if let Some(target) = target {
        cmd.arg(target);
    }
    let out = cmd.output().context("running git")?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    /// A project folder holding `acme-tickets` (a board, identity set),
    /// with `origin` set to `origin` when given.
    fn project(origin: Option<&str>) -> (tempfile::TempDir, Board) {
        let dir = tempfile::tempdir().unwrap();
        let tickets = dir.path().join("acme-tickets");
        fs::create_dir_all(&tickets).unwrap();
        git(&tickets, &["init", "-q"]);
        git(&tickets, &["config", "user.email", "t@t.co"]);
        git(&tickets, &["config", "user.name", "t"]);
        if let Some(origin) = origin {
            git(&tickets, &["remote", "add", "origin", origin]);
        }
        let board = Board::in_repo(&tickets);
        board.create().unwrap();
        fs::write(tickets.join(".settings"), "commit = off\nproject = acme\n").unwrap();
        (dir, board)
    }

    #[test]
    fn the_forge_follows_the_tickets_origin() {
        let (_d, gh_board) = project(Some("git@github.com:acme-org/acme-tickets.git"));
        assert_eq!(
            forge(&gh_board).unwrap(),
            Forge::GitHub {
                owner: "acme-org".into()
            }
        );
        let (_d, local_board) = project(Some("../origins/acme-tickets.git"));
        assert_eq!(forge(&local_board).unwrap(), Forge::Local);
        let (_d, none) = project(None);
        assert_eq!(forge(&none).unwrap(), Forge::None);
        let (_d, other) = project(Some("git@gitlab.com:acme/acme-tickets.git"));
        assert!(forge(&other).is_err());
    }

    #[test]
    fn on_github_it_creates_with_the_tickets_visibility_or_connects() {
        let (_d, board) = project(Some("git@github.com:acme-org/acme-tickets.git"));
        let mut calls: Vec<String> = Vec::new();
        let mut fake = |args: &[&str]| -> Result<String> {
            calls.push(args.join(" "));
            match args {
                ["repo", "view", "acme-org/acme-tickets", ..] => Ok("PUBLIC\n".into()),
                ["repo", "view", _, ..] => bail!("not found"),
                _ => Ok(String::new()),
            }
        };
        assert!(!defaults(&board, &mut fake).unwrap().private);
        // The clone fails (there's no GitHub here), after creating it.
        let err = create(
            &board,
            &NewRepo {
                name: "acme-payments".into(),
                description: "Payments".into(),
                ..NewRepo::default()
            },
            &mut fake,
        )
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("run this again to connect it"),
            "{err:#}"
        );
        assert!(
            calls
                .iter()
                .any(|c| c == "repo create acme-org/acme-payments --public --description Payments"),
            "{calls:?}"
        );
        // Already there: viewed, not created.
        let mut calls: Vec<String> = Vec::new();
        let mut exists = |args: &[&str]| -> Result<String> {
            calls.push(args.join(" "));
            Ok("PRIVATE\n".into())
        };
        let _ = create(
            &board,
            &NewRepo {
                name: "acme-payments".into(),
                ..NewRepo::default()
            },
            &mut exists,
        );
        assert!(
            !calls.iter().any(|c| c.starts_with("repo create")),
            "{calls:?}"
        );
    }

    #[test]
    fn beside_a_local_origin_it_makes_the_bare_repository_clones_and_pushes_a_readme() {
        let dir = tempfile::tempdir().unwrap();
        let origins = dir.path().join("origins");
        fs::create_dir_all(&origins).unwrap();
        let bare = origins.join("acme-tickets.git");
        Command::new("git")
            .args(["init", "-q", "--bare"])
            .arg(&bare)
            .status()
            .unwrap();
        let (_d, board) = project(Some(bare.to_str().unwrap()));
        let mut no_gh = |_: &[&str]| -> Result<String> { bail!("no gh here") };
        let made = create(
            &board,
            &NewRepo {
                name: "acme-payments".into(),
                description: "Card payments".into(),
                readme: true,
                for_ticket: Some(("012".into(), "Take card payments".into())),
                ..NewRepo::default()
            },
            &mut no_gh,
        )
        .unwrap();
        assert!(made.created && made.forge == Forge::Local);
        assert!(origins.join("acme-payments.git").exists());
        let readme = crate::eol::read(made.path.join("README.md")).unwrap();
        assert!(
            readme.contains("# acme-payments") && readme.contains("ticket 012"),
            "{readme}"
        );
        assert!(
            git::head(&origins.join("acme-payments.git")).is_some(),
            "README pushed"
        );
        // Again: its folder is there now, beside the tickets repository, so
        // it is cloned for the project (ticket 342) though unregistered.
        let err = create(
            &board,
            &NewRepo {
                name: "acme-payments".into(),
                ..NewRepo::default()
            },
            &mut no_gh,
        )
        .unwrap_err();
        assert!(err.to_string().contains("already cloned"), "{err}");
        // Declared for the project, and the ticket's note (setting the
        // ticket's repository needs the clone registered: test.sh does that).
        declare(&board, &made.name).unwrap();
        assert!(
            board_settings::load(&board)
                .repos()
                .contains(&"acme-payments".to_string())
        );
        assert_eq!(
            note_for(&made, "Alex Morgan", "2026-10-16"),
            "**Alex Morgan, 2026-10-16:** Created repository acme-payments (private)."
        );
    }

    #[test]
    fn a_bad_name_or_one_in_the_way_is_refused() {
        let (dir, board) = project(None);
        let mut no_gh = |_: &[&str]| -> Result<String> { bail!("no gh") };
        let bad = NewRepo {
            name: "no spaces/please".into(),
            ..NewRepo::default()
        };
        assert!(create(&board, &bad, &mut no_gh).is_err());
        fs::create_dir_all(dir.path().join("acme-web")).unwrap();
        let taken = NewRepo {
            name: "acme-web".into(),
            ..NewRepo::default()
        };
        let err = create(&board, &taken, &mut no_gh).unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
        let tickets = NewRepo {
            name: "acme-tickets".into(),
            ..NewRepo::default()
        };
        assert!(create(&board, &tickets, &mut no_gh).is_err());
    }
}
