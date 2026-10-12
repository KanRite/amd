//! Moving a ticket between columns the way `amd todo`/`doing`/`done` do —
//! the lease, the branch on entering `todo/`, and the auto-commit — as one
//! path that the command line and every other front end share.
//!
//! This used to live in `amd-cli`'s `main.rs`, and the desktop board only
//! reproduced half of it (a lease, but no branch and no commit). It moved so
//! that a move from any front end leaves the board, the git history and the
//! working tree in the same state.
//!
//! Nothing here prints. A front end that owns the screen must not have a
//! library writing over it, so what the CLI used to print is returned as
//! ordered [`Event`]s and the caller decides how to show them.

use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::board::{Board, Column};
use crate::task::Task;
use crate::{board_settings, branch, git};

/// One thing a transition wants the user to know, in the order it happened.
/// `Info` is what the CLI prints on stdout; `Warn` is what it prints on
/// stderr, for a best-effort step that failed without undoing the move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Info(String),
    Warn(String),
}

/// What a transition did.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Where the ticket landed, when it moved.
    pub dest: Option<PathBuf>,
    /// Everything to tell the user, in order.
    pub events: Vec<Event>,
}

/// How to enter `todo/`: which branch to put the working tree on.
#[derive(Debug, Default, Clone)]
pub struct TodoOptions {
    /// Skip the WIP limit.
    pub force: bool,
    /// Use this branch instead of the ticket's own.
    pub branch: Option<String>,
    /// Leave the branch alone entirely (`--no-branch`).
    pub no_branch: bool,
}

/// Runs `mutate`, then — if `commit` is on for this board — commits and
/// pushes exactly the paths it changed, determined by diffing `git status`
/// before and after rather than a hand-maintained list. That's not a
/// simplification: `Board::record_id` writes the `.next-id` counter
/// alongside a new ticket, so `amd new` genuinely touches two files in one
/// operation, and a hand-picked paths argument at each call site is exactly
/// the shape of place a second incidental write like that gets missed.
/// Anything already dirty *before* `mutate` runs — this repo's own
/// unrelated in-progress state — is excluded by construction, since it
/// appears in both snapshots.
///
/// Best-effort in the same spirit as the lease: a commit or push failure (a
/// locked signing key, no agent, the network down, a moved remote) must
/// never undo a board change that already succeeded on disk. It comes back as
/// the second tuple element for the caller to warn about — never an error,
/// never a rollback — because failing at this point would misreport a move
/// that actually worked, and there is nothing sane to roll back to that
/// wouldn't be its own surprise.
pub fn with_auto_commit<T>(
    board: &Board,
    verb: &str,
    subject: &str,
    detail: &str,
    mutate: impl FnOnce() -> Result<T>,
) -> Result<(T, Option<String>)> {
    let before = snapshot(board);
    let result = mutate()?;
    let warning = commit_since(board, before, &commit_message(verb, subject, detail))?;
    Ok((result, warning))
}

/// `verb: subject "detail"`, or `verb: subject` when there is no detail
/// (`register`, `group start`, …), rather than ending in an empty `""`.
/// Public so a front end's own report of the change says what the commit
/// says (ticket 117).
pub fn commit_message(verb: &str, subject: &str, detail: &str) -> String {
    if detail.is_empty() {
        format!("{verb}: {subject}")
    } else {
        format!("{verb}: {subject} \"{detail}\"")
    }
}

/// The repository's dirty paths as they stand, when auto-commit is on for
/// this board; `None` when it is off, so a caller can pass it straight back
/// to [`commit_since`] without asking again.
fn snapshot(
    board: &Board,
) -> Option<std::collections::BTreeMap<PathBuf, Option<std::time::SystemTime>>> {
    board_settings::load(board)
        .auto_commit()
        .then(|| git::dirty_paths(board.repo_root()).unwrap_or_default())
}

/// Commit and push whatever changed since `before` was taken, with `message`.
/// Returns the warning to show if that couldn't be done.
///
/// A push rejected because someone else pushed in the meantime goes to
/// `sync::after_rejected_push` (ticket 102): the change is replayed on top
/// of theirs when that's clean, and otherwise undone — the one case this
/// returns an `Err` (`sync::ConcurrentChange`) rather than a warning, since
/// nothing was applied.
pub fn commit_since(
    board: &Board,
    before: Option<std::collections::BTreeMap<PathBuf, Option<std::time::SystemTime>>>,
    message: &str,
) -> Result<Option<String>> {
    let Some(before) = before else {
        return Ok(None);
    };
    let repo = board.repo_root();
    let after = git::dirty_paths(repo).unwrap_or_default();
    let changed = git::changed_paths(&before, &after);
    if changed.is_empty() {
        return Ok(None);
    }
    let pre = git::head(repo);
    if let Err(err) = git::commit_paths_locally(repo, message, &changed) {
        return Ok(Some(format!(
            "could not commit/push automatically: {err:#}"
        )));
    }
    // A board with no remote (`amd init --local-only`, a scratch board) has
    // nowhere to push: the commit is the whole job, said nothing about
    // (ticket 142). Same as `sync::catch_up`, which does nothing there.
    if git::remote_url(repo).is_none() {
        return Ok(None);
    }
    match git::push(repo) {
        Ok(()) => Ok(None),
        Err(err) => crate::sync::after_rejected_push(board, pre.as_deref(), &changed, &err),
    }
}

/// Move a ticket, claiming its lease when it enters `doing/` and giving it
/// back when it leaves (ticket 156),
/// auto-committing the result. The wording of the `moved` line is the CLI's
/// — `Board::move_task` returns where the ticket landed and leaves the
/// reporting to whoever asked for the move.
pub fn move_to(board: &Board, task: &Task, to: Column, force: bool) -> Result<Outcome> {
    // Captured before the move: id_display()/title()/file_name() read the
    // ticket's own file, which move_task is about to relocate.
    let (id_display, title) = (task.id_display(), task.title());
    let file_name = task.file_name();
    let from = task.column;
    let (mut outcome, warning) = with_auto_commit(board, to.as_str(), &id_display, &title, || {
        let mut outcome = Outcome::default();
        let dest = board.move_task(task, to, force)?;
        outcome
            .events
            .push(Event::Info(format!("moved {file_name} -> {to}/")));
        // A ticket is locked exactly while it's in doing (ticket 156): the
        // lease is claimed on entering doing ("I'm working on this now"),
        // not earlier, since todo only means it's next. A move onto a
        // ticket someone else has leased is refused earlier, in move_task.
        if to == Column::Doing
            && let Some(moved) = Task::from_path(&dest, to)
            && let Some(warning) = maybe_take_lease(board, &moved)
        {
            outcome.events.push(Event::Warn(warning));
        }
        // ...and given back on leaving doing in any direction, or on
        // reaching done or the backlog (ticket 092), in the same commit as
        // the move. Only your own lease is ever released.
        if (from == Column::Doing || matches!(to, Column::Done | Column::Backlog))
            && to != Column::Doing
            && let Some(moved) = Task::from_path(&dest, to)
            && let Err(err) = moved.release_if_mine()
        {
            outcome.events.push(Event::Warn(format!(
                "could not release the lease automatically: {err:#}"
            )));
        }
        outcome.dest = Some(dest);
        Ok(outcome)
    })?;
    if let Some(warning) = warning {
        outcome.events.push(Event::Warn(warning));
    }
    Ok(outcome)
}

/// Move a task into `todo/` and put the working tree on its branch — what
/// `amd start` used to do on entry to `doing/`; the trigger moved earlier, to
/// `todo/`-entry, because "todo/ is what you have committed to" — the
/// workspace should be ready once you've committed to the work, not only once
/// you start typing (docs/adr/0006).
///
/// Order matters: the `git mv` is staged first so it travels with the switch
/// and lands on the task's own branch.
pub fn enter_todo(board: &Board, task: &Task, options: &TodoOptions) -> Result<Outcome> {
    // Read everything off the ticket before the move — afterwards the path is
    // stale. A ticket with no branch type has nothing to check out, so there's
    // nothing to switch to unless a branch was asked for explicitly.
    let wanted = match &options.branch {
        Some(name) => Some(name.clone()),
        None => match task.branch() {
            Some(name) => Some(name),
            // A ticket edited by hand may name a branch type but no branch.
            None => match task.branch_type() {
                Some(kind) => {
                    let name = branch::for_title(&kind, &task.title())?;
                    (!name.is_empty()).then_some(name)
                }
                None => None,
            },
        },
    };

    // Where the branch goes (ticket 107): the clone of the repository the
    // ticket is about, when it names one — not whichever repository amd
    // happens to be run from. Read before the move, like everything else.
    let repository = task.repository();
    let branching = !(options.no_branch || std::env::var("AMD_NO_BRANCH").as_deref() == Ok("1"));

    // A dedicated tickets repository holds tickets, not code (ticket 053):
    // a ticket that wants a branch must say which repository it's about,
    // or its branch would land here. Refused before the move, so nothing
    // is half done. A board inside a code repository (`amd init
    // --local-only` there) is that code repository, and branches as before.
    if branching && wanted.is_some() && repository.is_none() && is_tickets_repo(board) {
        let id = task.id_display();
        bail!(
            "{id} wants a branch but names no repository, and this board's repository only holds tickets: \
             set one with `amd set {id} repository <name>`, or move it with --no-branch"
        );
    }

    let mut outcome = move_to(board, task, Column::Todo, options.force)?;

    if !branching {
        return Ok(outcome);
    }
    let target = match &repository {
        None => board.root.clone(),
        Some(name) => match crate::registry::clone_of(&board.root, name) {
            Some(clone) => clone,
            None => {
                outcome.events.push(Event::Info(format!(
                    "{name} isn't cloned on this machine; left its branch alone"
                )));
                return Ok(outcome);
            }
        },
    };
    let Some(name) = wanted else {
        outcome.events.push(Event::Warn(
            "no branch type on this ticket; left the branch alone".to_string(),
        ));
        return Ok(outcome);
    };
    branch::validate(&name)?;
    let place = repository
        .as_deref()
        .map(|repo| format!(" in {repo}"))
        .unwrap_or_default();
    if git::current_branch(&target).as_deref() == Some(name.as_str()) {
        outcome
            .events
            .push(Event::Info(format!("already on {name}{place}")));
        return Ok(outcome);
    }
    let exists = git::branch_exists(&target, &name);
    git::switch_branch(&target, &name, !exists)?;
    outcome.events.push(Event::Info(format!(
        "{} {name}{place}",
        if exists { "switched to" } else { "branch" }
    )));
    Ok(outcome)
}

/// Whether `board` lives in a dedicated tickets repository, the
/// `{project}-tickets` that `amd init <project>` and `amd join` make, rather
/// than inside the code it tracks.
fn is_tickets_repo(board: &Board) -> bool {
    board
        .repo_root()
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with("-tickets"))
}

/// Best-effort: a failure here (no git email configured, or someone else
/// grabbing the lease in the instant between the move and this call) must
/// never undo a move that already succeeded — it only gets a note, the same
/// tone as "no branch type on this ticket; left the branch alone".
fn maybe_take_lease(board: &Board, task: &Task) -> Option<String> {
    let Ok(hash) = task.hash() else { return None };
    let hours = board_settings::load(board).lease_hours();
    task.take_lease(hash, hours)
        .err()
        .map(|err| format!("could not claim the lease automatically: {err:#}"))
}

/// Everything a front end gathers to make or change a ticket.
#[derive(Debug, Default, Clone)]
pub struct TicketFields {
    pub title: String,
    pub assignee: String,
    /// The repository it's about. On creation, `None` or empty means none;
    /// on an edit, `None` leaves it as it is and `Some("")` clears it.
    pub repository: Option<String>,
    /// One of `task::VALID_POINTS`, or empty.
    pub points: String,
    /// The epic or sprint to file it under, if any.
    pub epic: Option<String>,
    pub tags: Vec<String>,
    /// A branch type, or empty for a ticket with no branch.
    pub branch_type: String,
    /// A ref (id or slug), or empty.
    pub parent: String,
    pub related: Vec<String>,
    /// The ticket body — everything after the frontmatter.
    pub body: String,
    /// `low`, `medium` or `high`; empty keeps what the template (on
    /// creation) or the ticket (on an edit) says.
    pub priority: String,
}

/// Make a ticket the way `amd new` does: render it from the templates, write
/// it into `backlog/`, size it, file it under its epic, and auto-commit.
/// `outcome.dest` is where it landed.
pub fn create_ticket(board: &Board, fields: &TicketFields) -> Result<Outcome> {
    let templates = crate::templates::Templates::load(board)?;
    let draft = crate::create::Draft::prepare(
        board,
        &templates,
        crate::create::NewTicket {
            title: fields.title.trim().to_string(),
            template: crate::templates::DEFAULT_TEMPLATE.to_string(),
            branch_type: fields.branch_type.trim().to_string(),
            assignee: fields.assignee.trim().to_string(),
            repository: fields.repository.clone().unwrap_or_default(),
            parent: Some(fields.parent.trim().to_string()).filter(|p| !p.is_empty()),
            related: fields.related.clone(),
            tags: fields.tags.clone(),
            extra: Default::default(),
        },
    )?;
    // `write` takes the whole file, not just the notes — the same shape `amd
    // new` hands it after the editor has been over the rendered ticket.
    let text = crate::model::splice_body(&draft.body, &fields.body);
    // A new ticket's id isn't allocated until `write` runs, so the commit
    // message can't be built up front the way `with_auto_commit` wants it.
    let before = snapshot(board);
    let created = draft.write(board, Some(text))?;
    if !fields.points.trim().is_empty() {
        created
            .task
            .set_points(created.task.hash()?, fields.points.trim())?;
    }
    if !fields.priority.trim().is_empty() {
        created
            .task
            .set_priority(created.task.hash()?, fields.priority.trim())?;
    }
    // Read what the report and the commit message need before filing it under
    // an epic: inside `backlog/` that moves the file, after which `created.task`
    // points at a path that is gone and would read back as an empty title.
    let (id_display, title, file_name) = (
        created.task.id_display(),
        created.task.title(),
        created.task.file_name(),
    );
    let dest = board.set_epic(&created.task, fields.epic.as_deref())?;
    let mut outcome = Outcome::default();
    for id in &created.linked {
        outcome
            .events
            .push(Event::Info(format!("{id} now links to {id_display}")));
    }
    outcome.events.push(Event::Info(format!(
        "created {}/{file_name}",
        Column::Backlog
    )));
    let message = format!("new: {id_display} \"{title}\"");
    if let Some(warning) = commit_since(board, before, &message)? {
        outcome.events.push(Event::Warn(warning));
    }
    outcome.dest = Some(dest);
    Ok(outcome)
}

/// Save every editable field of an existing ticket in one write, checked
/// against `expected` — the fingerprint taken when the form opened — so a
/// ticket that changed underneath it is refused with the conflict rather than
/// overwritten. Auto-commits like `amd edit`.
pub fn edit_ticket(
    board: &Board,
    task: &Task,
    expected: crate::task::ContentHash,
    fields: &TicketFields,
) -> Result<Outcome> {
    let (id_display, title) = (task.id_display(), task.title());
    let ((), warning) = with_auto_commit(board, "edit", &id_display, &title, || {
        task.apply_edit(
            expected,
            board,
            crate::task::EditFields {
                title: &fields.title,
                assignee: &fields.assignee,
                points: &fields.points,
                tags: &fields.tags,
                branch_type: &fields.branch_type,
                parent: &fields.parent,
                related: &fields.related,
                body: &fields.body,
            },
        )?;
        if let Some(repository) = &fields.repository
            && crate::task::repo_name(repository) != task.repository().unwrap_or_default()
        {
            task.set_repository(task.hash()?, repository)?;
        }
        if !fields.priority.trim().is_empty() && fields.priority.trim() != task.priority() {
            task.set_priority(task.hash()?, fields.priority.trim())?;
        }
        board.set_epic(task, fields.epic.as_deref())?;
        Ok(())
    })?;
    let mut outcome = Outcome::default();
    outcome
        .events
        .push(Event::Info(format!("saved {id_display}")));
    if let Some(warning) = warning {
        outcome.events.push(Event::Warn(warning));
    }
    Ok(outcome)
}

/// Save a ticket's whole text, as an editor showing the raw file does —
/// hash-checked against `expected` (taken when the editor opened) and
/// auto-committed like `amd edit`. See [`Task::set_text`] for the checks.
pub fn save_text(
    board: &Board,
    task: &Task,
    expected: crate::task::ContentHash,
    text: &str,
) -> Result<Outcome> {
    let (id_display, title) = (task.id_display(), task.title());
    // A ticket from before `created-by` (ticket 155) gets it from git's
    // first commit as it's saved; one that has it keeps it.
    let text = match crate::authorship::created(board, task) {
        Some(c) if !c.email.is_empty() => {
            crate::authorship::ensure_created_by(text, &format!("{} <{}>", c.name, c.email))?
        }
        _ => text.to_string(),
    };
    let ((), warning) = with_auto_commit(board, "edit", &id_display, &title, || {
        task.set_text(expected, &text)
    })?;
    let mut outcome = Outcome::default();
    outcome
        .events
        .push(Event::Info(format!("saved {id_display}")));
    if let Some(warning) = warning {
        outcome.events.push(Event::Warn(warning));
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_message_leaves_out_an_empty_detail() {
        assert_eq!(
            commit_message("doing", "007", "Fix the thing"),
            "doing: 007 \"Fix the thing\""
        );
        assert_eq!(
            commit_message("register", "typofix", ""),
            "register: typofix"
        );
    }

    fn board_with(dir: &std::path::Path, name: &str, column: Column) -> (Board, Task) {
        let board = Board::at(dir.join("tasks"));
        board.create().expect("board");
        let body = format!("---\nid: \"{}\"\ntitle: \"T\"\n---\n\nbody\n", &name[..3]);
        let path = board.dir(column).join(name);
        std::fs::write(&path, body).unwrap();
        let task = Task::from_path(&path, column).unwrap();
        (board, task)
    }

    #[test]
    fn moving_to_done_reports_the_move_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let (board, task) = board_with(dir.path(), "001-t.md", Column::Doing);
        let outcome = move_to(&board, &task, Column::Done, false).unwrap();
        assert_eq!(
            outcome.events,
            [Event::Info("moved 001-t.md -> done/".to_string())]
        );
        assert!(board.dir(Column::Done).join("001-t.md").exists());
        assert_eq!(outcome.dest, Some(board.dir(Column::Done).join("001-t.md")));
    }

    #[test]
    fn a_refused_move_is_an_error_and_moves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (board, task) = board_with(dir.path(), "001-t.md", Column::Done);
        assert!(move_to(&board, &task, Column::Done, false).is_err());
        assert!(board.dir(Column::Done).join("001-t.md").exists());
    }

    #[test]
    fn entering_todo_without_a_branch_type_says_so_and_leaves_the_branch() {
        let dir = tempfile::tempdir().unwrap();
        let (board, task) = board_with(dir.path(), "002-t.md", Column::Backlog);
        let outcome = enter_todo(&board, &task, &TodoOptions::default()).unwrap();
        assert!(board.dir(Column::Todo).join("002-t.md").exists());
        assert!(
            outcome.events.contains(&Event::Warn(
                "no branch type on this ticket; left the branch alone".to_string()
            )),
            "{:?}",
            outcome.events
        );
    }

    #[test]
    fn no_branch_skips_the_branch_step_entirely() {
        let dir = tempfile::tempdir().unwrap();
        let (board, task) = board_with(dir.path(), "002-t.md", Column::Backlog);
        let outcome = enter_todo(
            &board,
            &task,
            &TodoOptions {
                no_branch: true,
                ..TodoOptions::default()
            },
        )
        .unwrap();
        assert!(
            !outcome
                .events
                .iter()
                .any(|event| matches!(event, Event::Warn(text) if text.contains("branch")))
        );
    }

    fn empty_board(dir: &std::path::Path) -> Board {
        let board = Board::at(dir.join("tasks"));
        board.create().expect("board");
        board
    }

    fn fields(title: &str) -> TicketFields {
        TicketFields {
            title: title.to_string(),
            points: "3".to_string(),
            tags: vec!["ux".to_string()],
            body: "## Notes\nhello\n".to_string(),
            ..TicketFields::default()
        }
    }

    #[test]
    fn creating_a_ticket_writes_it_into_the_backlog_sized_and_tagged() {
        let dir = tempfile::tempdir().unwrap();
        let board = empty_board(dir.path());
        let outcome = create_ticket(&board, &fields("Make a thing")).unwrap();
        let path = outcome.dest.expect("where it landed");
        assert!(path.starts_with(board.dir(Column::Backlog)), "{path:?}");
        let task = Task::from_path(&path, Column::Backlog).unwrap();
        assert_eq!(task.title(), "Make a thing");
        assert_eq!(task.points().as_deref(), Some("3"));
        assert_eq!(task.tags(), ["UX"], "saved in uppercase (ticket 122)");
        assert!(task.body().unwrap().contains("hello"));
        assert!(
            matches!(outcome.events.first(), Some(Event::Info(text)) if text.starts_with("created backlog/001-")),
            "{:?}",
            outcome.events
        );
    }

    #[test]
    fn a_ticket_filed_under_an_epic_reports_where_it_really_is() {
        let dir = tempfile::tempdir().unwrap();
        let board = empty_board(dir.path());
        board
            .create_group(&crate::group::Group {
                dir: board.dir(Column::Backlog).join("Ep"),
                name: "Ep".to_string(),
                kind: crate::group::Kind::Epic,
                description: String::new(),
                days: 0,
                state: crate::group::State::Pending,
            })
            .unwrap();
        let mut with_epic = fields("Filed away");
        with_epic.epic = Some("Ep".to_string());
        let outcome = create_ticket(&board, &with_epic).unwrap();
        let path = outcome.dest.unwrap();
        assert!(
            path.exists(),
            "dest must be where the file is now: {path:?}"
        );
        assert!(path.to_string_lossy().contains("/Ep/"), "{path:?}");
    }

    #[test]
    fn a_ticket_with_nothing_sluggable_in_its_title_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let board = empty_board(dir.path());
        assert!(create_ticket(&board, &fields("!!!")).is_err());
        assert!(board.tasks_in(Column::Backlog).unwrap().is_empty());
    }

    #[test]
    fn editing_saves_every_field_in_one_write() {
        let dir = tempfile::tempdir().unwrap();
        let board = empty_board(dir.path());
        let path = create_ticket(&board, &fields("Old title"))
            .unwrap()
            .dest
            .unwrap();
        let task = Task::from_path(&path, Column::Backlog).unwrap();
        let hash = task.hash().unwrap();
        let mut edited = fields("New title");
        edited.assignee = "sam".to_string();
        edited.body = "## Notes\nchanged\n".to_string();
        edit_ticket(&board, &task, hash, &edited).unwrap();
        let task = Task::from_path(&path, Column::Backlog).unwrap();
        assert_eq!(task.title(), "New title");
        assert_eq!(task.assignee().as_deref(), Some("sam"));
        assert!(task.body().unwrap().contains("changed"));
    }

    #[test]
    fn editing_a_ticket_that_changed_underneath_is_refused_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let board = empty_board(dir.path());
        let path = create_ticket(&board, &fields("Old title"))
            .unwrap()
            .dest
            .unwrap();
        let task = Task::from_path(&path, Column::Backlog).unwrap();
        let stale = task.hash().unwrap();
        // Someone else saves first.
        task.set_title(stale, "Theirs").unwrap();
        let result = edit_ticket(&board, &task, stale, &fields("Mine"));
        assert!(result.is_err());
        let task = Task::from_path(&path, Column::Backlog).unwrap();
        assert_eq!(task.title(), "Theirs");
    }
}
