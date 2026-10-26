//! A ticket's repository and branch type go together (ticket 066).
//!
//! On a dedicated tickets board ([`Board::is_tickets_board`]) a ticket's
//! branch has to go in the repository it names (ticket 053), so the two
//! fields only make sense together. A repository and no branch type hasn't
//! decided whether it wants a branch; a branch type and no repository has
//! nowhere to put one. Either way the ticket is a **draft** (ticket 114)
//! until it's settled, rather than refused: nothing typed is lost, and a
//! draft can't leave the backlog anyway. [`branch::NO_BRANCH`] (`none`)
//! settles "no branch, on purpose".
//!
//! Past the backlog a ticket can't be a draft, so a change there that would
//! leave it mismatched is refused. Tickets already mismatched are only
//! reported ([`mismatched`]), never rewritten. A board inside a code
//! repository (`rite init --local-only`) is exempt: its own repository is the
//! code, so a branch type with no repository is right there.

use anyhow::{Result, bail};

use crate::board::{Board, Column, Kind};
use crate::branch::{self, NO_BRANCH};
use crate::task::{self, DRAFT_TAG, Task};

/// Why a repository and branch type don't go together, or `None` when they
/// do. Takes the values as written; an empty repository is none.
pub fn mismatch(repository: &str, branch_type: &str) -> Option<String> {
    let repository = task::repo_name(repository);
    let kind = branch::normalise(branch_type);
    if !repository.is_empty() && kind.is_empty() {
        Some(format!(
            "it names a repository ({repository}) but no branch type: choose one, or `{NO_BRANCH}` for no branch"
        ))
    } else if repository.is_empty() && branch::makes_branch(&kind) {
        Some(format!(
            "it has a branch type ({kind}) but no repository for its branch to go in"
        ))
    } else {
        None
    }
}

/// What saving ticket `id`, in `column`, with these values means: `None`
/// when they go together (or the board is exempt), `Some(reason)` when it
/// must be a draft, and an error past the backlog, where it can't be one.
pub fn check(
    board: &Board,
    id: &str,
    column: Column,
    repository: &str,
    branch_type: &str,
) -> Result<Option<String>> {
    if !board.is_tickets_board() {
        return Ok(None);
    }
    let Some(why) = mismatch(repository, branch_type) else {
        return Ok(None);
    };
    if column.kind() == Kind::Backlog {
        return Ok(Some(why));
    }
    bail!(
        "{id} is in {column}, past the backlog, so it can't be a draft and can't be saved like this: {why}"
    )
}

/// [`check`] for a whole ticket's text, as a save or a new ticket has it.
pub fn check_text(board: &Board, id: &str, column: Column, text: &str) -> Result<Option<String>> {
    check(
        board,
        id,
        column,
        &value_in(text, "repository"),
        &value_in(text, "branch-type"),
    )
}

/// `text` with [`DRAFT_TAG`] among its tags (once), the others kept.
pub fn with_draft_tag(text: &str) -> Result<String> {
    let mut tags = task::normalize_tags(
        &task::meta_in(text, "tags")
            .map(task::parse_list)
            .unwrap_or_default(),
    );
    if tags.iter().any(|tag| tag == DRAFT_TAG) {
        return Ok(text.to_string());
    }
    tags.push(DRAFT_TAG.to_string());
    task::set_meta(text, "tags", &format!("[{}]", tags.join(",")))
}

/// What a ticket being drafted for this says.
pub fn drafted(id: &str, why: &str) -> String {
    format!("{id} is a draft until that's settled: {why}")
}

/// Refuses to take a mismatched ticket out of draft (`rite ready`).
pub fn ready(board: &Board, task: &Task) -> Result<()> {
    if !board.is_tickets_board() {
        return Ok(());
    }
    if let Some(why) = mismatch(
        &task.repository().unwrap_or_default(),
        &task.branch_type().unwrap_or_default(),
    ) {
        bail!("{} can't be ready yet: {why}", task.id_display());
    }
    Ok(())
}

/// The tickets on the board, not done, whose repository and branch type
/// don't go together, each with why. Read only.
pub fn mismatched(board: &Board) -> Result<Vec<(Task, String)>> {
    if !board.is_tickets_board() {
        return Ok(Vec::new());
    }
    Ok(board
        .tasks()?
        .into_iter()
        .filter(|task| task.column.kind() != Kind::Done)
        .filter_map(|task| {
            let why = mismatch(
                &task.repository().unwrap_or_default(),
                &task.branch_type().unwrap_or_default(),
            )?;
            Some((task, why))
        })
        .collect())
}

/// A frontmatter value with its quotes taken off, empty when missing.
fn value_in(text: &str, key: &str) -> String {
    task::meta_in(text, key)
        .map(|value| value.trim().trim_matches('"').to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create::{Draft, NewTicket};
    use crate::templates::Templates;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    /// A board in a repository called `repo`: `acme-tickets` pairs, `acme`
    /// (a board inside the code) doesn't.
    fn board_in(repo: &str) -> (TempDir, Board) {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join(repo);
        fs::create_dir_all(&root).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@t.co"],
            vec!["config", "user.name", "t"],
        ] {
            Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .status()
                .expect("git");
        }
        let board = Board {
            root: root.join("tasks"),
        };
        board.create().expect("board");
        (dir, board)
    }

    fn make(board: &Board, branch_type: &str) -> crate::create::Created {
        let templates = Templates::load(board).unwrap();
        let ticket = NewTicket {
            title: "Add login".into(),
            template: crate::templates::DEFAULT_TEMPLATE.into(),
            branch_type: branch_type.into(),
            ..Default::default()
        };
        Draft::prepare(board, &templates, ticket)
            .unwrap()
            .write(board, None)
            .unwrap()
    }

    /// A ticket file written by hand, as a teammate's clone might have it.
    fn by_hand(board: &Board, column: Column, repository: &str, kind: &str) -> Task {
        let path = board.dir(column).join("007-by-hand.md");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                "---\nid: \"007\"\ntitle: \"By hand\"\nrepository: \"{repository}\"\n\
                 branch-type: \"{kind}\"\ntags: []\npriority: \"low\"\n---\n\nbody\n"
            ),
        )
        .unwrap();
        Task::from_path(&path, column).unwrap()
    }

    #[test]
    fn a_new_ticket_with_a_branch_type_and_no_repository_is_a_draft() {
        let (_dir, board) = board_in("acme-tickets");
        let created = make(&board, "feature");
        assert!(created.task.is_draft());
        assert!(created.drafted.unwrap().contains("no repository"));
        // `none` settles it, and so does no branch type at all.
        let none = make(&board, NO_BRANCH);
        assert!(!none.task.is_draft() && none.drafted.is_none());
        assert_eq!(none.branch, "");
        assert!(!make(&board, "").task.is_draft());
    }

    #[test]
    fn a_board_inside_its_code_repository_is_exempt() {
        let (_dir, board) = board_in("acme");
        let created = make(&board, "feature");
        assert!(!created.task.is_draft());
        assert!(created.drafted.is_none());
        assert!(mismatched(&board).unwrap().is_empty());
    }

    #[test]
    fn saving_a_backlog_ticket_keeps_it_a_draft_and_past_it_refuses() {
        let (_dir, board) = board_in("acme-tickets");
        let task = by_hand(&board, Column::Backlog, "acme-api", "");
        let text = crate::eol::read(&task.path).unwrap();
        let outcome =
            crate::transition::save_text(&board, &task, task.hash().unwrap(), &text).unwrap();
        let task = Task::from_path(&task.path, Column::Backlog).unwrap();
        assert!(task.is_draft(), "{}", crate::eol::read(&task.path).unwrap());
        assert!(outcome.events.iter().any(|e| matches!(
            e,
            crate::transition::Event::Warn(w) if w.contains("no branch type")
        )));
        assert!(
            ready(&board, &task)
                .unwrap_err()
                .to_string()
                .contains("can't be ready")
        );

        let doing = by_hand(&board, Column::Doing, "acme-api", "");
        let text = crate::eol::read(&doing.path).unwrap();
        let err =
            crate::transition::save_text(&board, &doing, doing.hash().unwrap(), &text).unwrap_err();
        assert!(format!("{err:#}").contains("past the backlog"), "{err:#}");
    }

    #[test]
    fn tickets_already_mismatched_are_listed_and_left_alone() {
        let (_dir, board) = board_in("acme-tickets");
        let task = by_hand(&board, Column::Doing, "acme-api", "");
        let before = crate::eol::read(&task.path).unwrap();
        let found = mismatched(&board).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].1.contains("acme-api"));
        assert_eq!(crate::eol::read(&task.path).unwrap(), before);
    }

    #[test]
    fn a_repository_wants_a_branch_type_and_a_branch_type_a_repository() {
        assert!(mismatch("acme-api", "").unwrap().contains("no branch type"));
        assert!(mismatch("", "feature").unwrap().contains("no repository"));
        assert_eq!(mismatch("acme-api", "feature"), None);
        assert_eq!(mismatch("acme-api", NO_BRANCH), None);
        assert_eq!(mismatch("", NO_BRANCH), None);
        assert_eq!(mismatch("", ""), None);
        // The picker's "not chosen" is empty.
        assert!(mismatch("acme-api", branch::NONE).is_some());
    }

    #[test]
    fn the_draft_tag_is_added_once_and_the_others_kept() {
        let text = "---\ntitle: \"x\"\ntags: [cli]\n---\n\nbody\n";
        let once = with_draft_tag(text).unwrap();
        assert!(once.contains("tags: [CLI,DRAFT]"), "{once}");
        assert_eq!(with_draft_tag(&once).unwrap(), once);
    }

    #[test]
    fn values_are_read_without_their_quotes() {
        let text = "---\nrepository: \"acme-api\"\nbranch-type: \"\"\n---\n";
        assert_eq!(value_in(text, "repository"), "acme-api");
        assert_eq!(value_in(text, "branch-type"), "");
        assert_eq!(value_in(text, "missing"), "");
    }
}
