//! Moving a ticket between columns the way `amd todo`/`doing`/`done` do —
//! the lease, the branch on entering `todo/`, and the auto-commit — as one
//! path that the command line and every other front end share.
//!
//! This used to live in `amd-cli`'s `main.rs`, and the desktop board only
//! reproduced half of it (a lease, but no branch and no commit). The terminal
//! board is the reason it moved: a move from any front end has to leave the
//! board, the git history and the working tree in the same state.
//!
//! Nothing here prints. A front end that owns the terminal (the TUI) must not
//! have a library writing over its screen, so what the CLI used to print is
//! returned as ordered [`Event`]s and the caller decides how to show them.

use std::path::PathBuf;

use anyhow::Result;

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
/// simplification: `Board::record_id` writes `.settings`'s `next-id`
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
    let auto_commit = board_settings::load(board).auto_commit();
    let before = auto_commit.then(|| git::dirty_paths(board.repo_root()).unwrap_or_default());
    let result = mutate()?;
    let mut warning = None;
    if let Some(before) = before {
        let after = git::dirty_paths(board.repo_root()).unwrap_or_default();
        let changed = git::changed_paths(&before, &after);
        if !changed.is_empty() {
            let message = format!("{verb}: {subject} \"{detail}\"");
            if let Err(err) = git::commit_paths(board.repo_root(), &message, &changed) {
                warning = Some(format!("could not commit/push automatically: {err:#}"));
            }
        }
    }
    Ok((result, warning))
}

/// Move a ticket and claim its lease when it enters `todo/` or `doing/`,
/// auto-committing the result. The wording of the `moved` line is the CLI's
/// — `Board::move_task` returns where the ticket landed and leaves the
/// reporting to whoever asked for the move.
pub fn move_to(board: &Board, task: &Task, to: Column, force: bool) -> Result<Outcome> {
    // Captured before the move: id_display()/title()/file_name() read the
    // ticket's own file, which move_task is about to relocate.
    let (id_display, title) = (task.id_display(), task.title());
    let file_name = task.file_name();
    let (mut outcome, warning) = with_auto_commit(board, to.as_str(), &id_display, &title, || {
        let mut outcome = Outcome::default();
        let dest = board.move_task(task, to, force)?;
        outcome
            .events
            .push(Event::Info(format!("moved {file_name} -> {to}/")));
        // Entering todo/doing already means "I'm starting this" — claiming
        // the lease at the same moment matches that intent with no extra
        // step. Every other column move (and every column move that's
        // already leased by someone else) is refused earlier, inside
        // move_task itself.
        if matches!(to, Column::Todo | Column::Doing)
            && let Some(moved) = Task::from_path(&dest, to)
            && let Some(warning) = maybe_take_lease(board, &moved)
        {
            outcome.events.push(Event::Warn(warning));
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

    let mut outcome = move_to(board, task, Column::Todo, options.force)?;

    if options.no_branch || std::env::var("AMD_NO_BRANCH").as_deref() == Ok("1") {
        return Ok(outcome);
    }
    let Some(name) = wanted else {
        outcome.events.push(Event::Warn(
            "no branch type on this ticket; left the branch alone".to_string(),
        ));
        return Ok(outcome);
    };
    branch::validate(&name)?;
    if git::current_branch(&board.root).as_deref() == Some(name.as_str()) {
        outcome
            .events
            .push(Event::Info(format!("already on {name}")));
        return Ok(outcome);
    }
    let exists = git::branch_exists(&board.root, &name);
    git::switch_branch(&board.root, &name, !exists)?;
    outcome.events.push(Event::Info(format!(
        "{} {name}",
        if exists { "switched to" } else { "branch" }
    )));
    Ok(outcome)
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
