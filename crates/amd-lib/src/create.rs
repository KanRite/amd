//! Creating a ticket, shared by the CLI and the GUI.
//!
//! Split in two on purpose:
//!
//! - [`Draft::prepare`] resolves everything (id, slug, labels, links, branch)
//!   and *renders* the ticket, touching nothing on disk. Every validation
//!   failure — an unknown parent, a title that can't make a filename, a
//!   template variable nobody supplied — happens here.
//! - [`Draft::write`] commits it: the file, the parent's child list and the
//!   backlinks on any related tickets.
//!
//! That gap is what lets the CLI drop the rendered ticket into `$EDITOR` before
//! anything is written, while the GUI writes the body it already has.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::board::{Board, Column};
use crate::branch;
use crate::git;
use crate::task::{self, Task};
use crate::templates::{TaskContext, Templates};

/// What a front end has to gather to make a ticket.
#[derive(Clone, Debug, Default)]
pub struct NewTicket {
    pub title: String,
    /// Ticket type — the template name (`development`, `admin`, …).
    pub template: String,
    /// Branch type (`feature`, `bugfix`, …); empty for a ticket with no branch.
    pub branch_type: String,
    /// Who it's assigned to; empty for nobody, and assignable later.
    pub assignee: String,
    /// The repository it's about; empty for none. Must be cloned here.
    pub repository: String,
    /// A ref (id or slug) for the ticket this one sits under.
    pub parent: Option<String>,
    /// Refs for the tickets this one relates to.
    pub related: Vec<String>,
    /// A ref for the ticket this one is a copy of. The copy records it as
    /// `clone-of`, and the original gets the copy's id in `clones`.
    pub clone_of: Option<String>,
    pub tags: Vec<String>,
    /// Values for the template's own `extra.*` fields.
    pub extra: BTreeMap<String, String>,
    /// Which column it lands in (ticket 225): the board's landing column
    /// when `None`. One going straight into an epic or sprint lands in the
    /// standard backlog, where groups live.
    pub land_in: Option<Column>,
}

/// A ticket that has been resolved and rendered, but not yet written.
#[derive(Debug)]
pub struct Draft {
    pub path: PathBuf,
    /// The column it lands in (ticket 225).
    pub column: Column,
    pub id: String,
    /// The same id as a number, for the board's counter.
    pub number: u32,
    pub slug: String,
    /// Branch the ticket will be worked on, empty when its type doesn't use one.
    pub branch: String,
    /// The rendered markdown, ready to write or to hand to an editor.
    pub body: String,
    parent: Option<Task>,
    related: Vec<String>,
    clone_of: Option<Task>,
}

/// What changed when a draft was written, so a front end can report it.
#[derive(Debug)]
pub struct Created {
    pub task: Task,
    pub branch: String,
    /// Tickets that gained a link back to this one.
    pub linked: Vec<String>,
    /// The original of a clone that couldn't list it (leased by someone
    /// else, say), with the reason.
    pub unlinked: Option<String>,
    /// Why it was made a draft: a repository with no branch type, or the
    /// reverse (ticket 066). Already worded for a person.
    pub drafted: Option<String>,
}

impl Draft {
    /// Resolve and render, without writing anything.
    pub fn prepare(board: &Board, templates: &Templates, ticket: NewTicket) -> Result<Draft> {
        branch::validate_sluggable(&ticket.title)?;
        let repository = crate::task::repo_name(&ticket.repository);
        crate::task::check_repository(&board.root, &repository)?;

        // A branch type is optional: with one, the ticket is worked on a branch
        // named from it and the title; without, there's simply nothing to check
        // out. That single field is what `admin` and `development` used to be.
        let branch_type = branch::normalise(&ticket.branch_type);
        branch::validate_branch_type(&branch_type)?;
        let branch_name = branch::for_title(&branch_type, &ticket.title)?;

        // Links are stored as ids and resolved now: a typo is an error here
        // rather than a dangling link discovered later.
        let parent_task = match &ticket.parent {
            Some(reference) if !reference.is_empty() => Some(board.find(reference)?),
            _ => None,
        };
        let mut related = Vec::new();
        for reference in &ticket.related {
            related.push(board.find(reference)?.id_display());
        }
        related.sort();
        related.dedup();
        let clone_task = match &ticket.clone_of {
            Some(reference) if !reference.is_empty() => Some(board.find(reference)?),
            _ => None,
        };

        // Every field the template asks for has to have been collected.
        let missing: Vec<String> = templates
            .required_extras(&ticket.template)?
            .into_iter()
            .filter(|key| !ticket.extra.contains_key(key))
            .collect();
        if !missing.is_empty() {
            bail!(
                "template '{}' needs {}",
                ticket.template,
                missing
                    .iter()
                    .map(|key| format!("{key}=…"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }

        let number = board.next_id()?;
        let id = format!("{number:03}");
        let slug = task::slugify(&ticket.title);
        let column = ticket.land_in.unwrap_or_else(|| board.workflow().landing());
        let dir = board.dir(column);
        let path = dir.join(format!("{id}-{slug}.md"));
        if path.exists() {
            bail!("{} already exists", path.display());
        }

        let (created, timestamp) = crate::templates::today();
        let context = TaskContext {
            id: id.clone(),
            number,
            title: ticket.title,
            slug: slug.clone(),
            branch_type: branch_type.clone(),
            branch_name: branch_name.clone(),
            parent: parent_task
                .as_ref()
                .map(|task| task.id_display())
                .unwrap_or_default(),
            parent_link: parent_task
                .as_ref()
                .map(|task| task.stem.clone())
                .unwrap_or_default(),
            assignee: ticket.assignee,
            repository,
            related: related.clone(),
            clone_of: clone_task
                .as_ref()
                .map(|task| task.id_display())
                .unwrap_or_default(),
            tags: crate::task::normalize_tags(&ticket.tags),
            created,
            timestamp,
            column: Column::Backlog.to_string(),
            author: git::config(&board.root, "user.name").unwrap_or_else(|| "unknown".to_string()),
            email: git::config(&board.root, "user.email").unwrap_or_default(),
            created_by: crate::authorship::current_person(board).unwrap_or_default(),
            board: board.name(),
            template: ticket.template.clone(),
            extra: ticket.extra,
        };
        let body = templates.render(&ticket.template, &context)?;

        Ok(Draft {
            path,
            column,
            id,
            number,
            slug,
            branch: branch_name,
            body,
            parent: parent_task,
            related,
            clone_of: clone_task,
        })
    }

    /// Write the ticket, then the links that point back at it. `body` replaces
    /// the rendered markdown — that's the editor's answer, when there was one.
    pub fn write(self, board: &Board, body: Option<String>) -> Result<Created> {
        let body = body.unwrap_or(self.body);
        if body.trim().is_empty() {
            bail!("the ticket is empty; nothing created");
        }
        // Every ticket has a priority — a custom template without the field
        // gets the default; an invalid one typed in the editor is refused.
        let body = crate::task::ensure_priority(&body)?;
        // Who made it (ticket 155), for a custom template that doesn't say.
        let body = match crate::authorship::current_person(board) {
            Some(person) => crate::authorship::ensure_created_by(&body, &person)?,
            None => body,
        };
        // A custom template without the field, or one edited by hand, still
        // records what this is a copy of.
        let body = match &self.clone_of {
            Some(original)
                if crate::task::meta_in(&body, "clone-of").is_none_or(|id| id.is_empty()) =>
            {
                crate::task::set_meta(&body, "clone-of", &format!("\"{}\"", original.id_display()))?
            }
            _ => body,
        };
        // A repository and a branch type go together (ticket 066): one
        // without the other makes it a draft, on every way a ticket is made.
        let (body, drafted) = match crate::pairing::check_text(board, &self.id, self.column, &body)?
        {
            Some(why) => (
                crate::pairing::with_draft_tag(&body)?,
                Some(crate::pairing::drafted(&self.id, &why)),
            ),
            None => (body, None),
        };
        let dir = self
            .path
            .parent()
            .expect("a task path always has a column directory");
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        fs::write(&self.path, body).with_context(|| format!("writing {}", self.path.display()))?;
        // The id is spent only now: a draft that was abandoned leaves no gap.
        board.record_id(self.number)?;

        let stem = format!("{}-{}", self.id, self.slug);
        let mut linked = Vec::new();
        // The parent lists its children, so navigation works from either end.
        if let Some(parent) = &self.parent
            && parent.add_child_link(parent.hash()?, &stem)?
        {
            linked.push(parent.id_display());
        }
        // "related" reads the same from either end, so the tickets this one
        // names get it back.
        for reference in &self.related {
            let other = board.find(reference)?;
            if other.add_related(other.hash()?, std::slice::from_ref(&self.id))? {
                linked.push(other.id_display());
            }
        }

        // A copy is listed on the original. An original that someone else
        // holds can't be written to; the copy is already made, so that is a
        // missing backlink to report rather than a failure.
        let mut unlinked = None;
        if let Some(original) = &self.clone_of {
            match original.add_clone(original.hash()?, &self.id) {
                Ok(true) => linked.push(original.id_display()),
                Ok(false) => {}
                Err(err) => unlinked = Some(format!("{}: {err:#}", original.id_display())),
            }
        }

        let task = Task::from_path(&self.path, self.column)
            .ok_or_else(|| anyhow::anyhow!("{} is not a task file", self.path.display()))?;
        Ok(Created {
            task,
            branch: self.branch,
            linked,
            unlinked,
            drafted,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    /// A throwaway git repo with an initialised board.
    fn board() -> (TempDir, Board) {
        let dir = tempfile::tempdir().expect("temp dir");
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@t.co"],
            vec!["config", "user.name", "t"],
        ] {
            Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .status()
                .expect("git");
        }
        let board = Board {
            root: dir.path().join("tasks"),
        };
        board.create().expect("board");
        (dir, board)
    }

    fn ticket(title: &str) -> NewTicket {
        NewTicket {
            title: title.to_string(),
            template: crate::templates::DEFAULT_TEMPLATE.to_string(),
            branch_type: "feature".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn a_draft_writes_nothing_until_it_is_written() {
        let (_dir, board) = board();
        let templates = Templates::load(&board).unwrap();
        let draft = Draft::prepare(&board, &templates, ticket("Add login")).unwrap();
        assert_eq!(draft.id, "001");
        assert_eq!(draft.branch, "feature/add-login");
        assert!(draft.body.contains("title: \"Add login\""));
        assert!(!draft.path.exists(), "prepare must not touch the disk");

        let created = draft.write(&board, None).unwrap();
        assert!(created.task.path.exists());
        assert_eq!(created.task.id_display(), "001");
    }

    #[test]
    fn the_body_can_be_replaced_on_the_way_out() {
        let (_dir, board) = board();
        let templates = Templates::load(&board).unwrap();
        let draft = Draft::prepare(&board, &templates, ticket("Add login")).unwrap();
        let created = draft
            .write(
                &board,
                Some("---\nid: \"001\"\n---\n\nedited\n".to_string()),
            )
            .unwrap();
        let body = std::fs::read_to_string(&created.task.path).unwrap();
        assert!(body.contains("edited"), "{body}");
    }

    #[test]
    fn an_empty_body_creates_nothing() {
        let (_dir, board) = board();
        let templates = Templates::load(&board).unwrap();
        let draft = Draft::prepare(&board, &templates, ticket("Add login")).unwrap();
        let path = draft.path.clone();
        assert!(draft.write(&board, Some("  \n".to_string())).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn links_are_resolved_before_anything_is_written() {
        let (_dir, board) = board();
        let templates = Templates::load(&board).unwrap();
        let mut wanted = ticket("Depends on nothing");
        wanted.parent = Some("99".to_string());
        let err = Draft::prepare(&board, &templates, wanted)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no task matches '99'"), "{err}");
        assert!(board.tasks().unwrap().is_empty());
    }

    #[test]
    fn writing_links_both_ends() {
        let (_dir, board) = board();
        let templates = Templates::load(&board).unwrap();
        Draft::prepare(&board, &templates, ticket("Parent"))
            .unwrap()
            .write(&board, None)
            .unwrap();

        let mut child = ticket("Child");
        child.parent = Some("001".to_string());
        child.related = vec!["001".to_string()];
        let created = Draft::prepare(&board, &templates, child)
            .unwrap()
            .write(&board, None)
            .unwrap();

        assert_eq!(created.task.parent().as_deref(), Some("001"));
        let parent = board.find("001").unwrap();
        assert!(parent.related().contains(&"002".to_string()));
        let parent_body = std::fs::read_to_string(&parent.path).unwrap();
        assert!(parent_body.contains("- [[002-child]]"), "{parent_body}");
    }

    #[test]
    fn a_ticket_without_a_branch_type_gets_no_branch() {
        let (_dir, board) = board();
        let templates = Templates::load(&board).unwrap();
        let mut wanted = ticket("Renew the certificates");
        wanted.branch_type = String::new();
        let draft = Draft::prepare(&board, &templates, wanted).unwrap();
        assert_eq!(draft.branch, "");
        assert!(draft.body.contains("branch-name: \"\""), "{}", draft.body);
    }

    #[test]
    fn an_unknown_branch_type_is_refused_before_anything_is_written() {
        let (_dir, board) = board();
        let templates = Templates::load(&board).unwrap();
        let mut wanted = ticket("Add login");
        wanted.branch_type = "feat".to_string();
        let err = Draft::prepare(&board, &templates, wanted)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown branch type 'feat'"), "{err}");
        assert!(board.tasks().unwrap().is_empty());
    }

    #[test]
    fn a_clone_and_its_original_point_at_each_other() {
        let (_dir, board) = board();
        let templates = Templates::load(&board).unwrap();
        Draft::prepare(&board, &templates, ticket("Original"))
            .unwrap()
            .write(&board, None)
            .unwrap();

        let mut copy = ticket("Copy");
        copy.clone_of = Some("001".to_string());
        let created = Draft::prepare(&board, &templates, copy)
            .unwrap()
            .write(&board, None)
            .unwrap();

        assert_eq!(created.task.clone_of().as_deref(), Some("001"));
        assert_eq!(created.linked, ["001"]);
        assert!(created.unlinked.is_none());
        assert_eq!(board.find("001").unwrap().clones(), ["002"]);
        // Not a plain relation: `related` is untouched at both ends.
        assert!(created.task.related().is_empty());
        assert!(board.find("001").unwrap().related().is_empty());
    }

    #[test]
    fn a_clone_of_a_ticket_that_is_not_there_is_refused_before_anything_is_written() {
        let (_dir, board) = board();
        let templates = Templates::load(&board).unwrap();
        let mut copy = ticket("Copy");
        copy.clone_of = Some("99".to_string());
        let err = Draft::prepare(&board, &templates, copy)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no task matches '99'"), "{err}");
        assert!(board.tasks().unwrap().is_empty());
    }

    #[test]
    fn a_clone_made_from_a_template_without_the_field_still_records_it() {
        let (_dir, board) = board();
        let templates = Templates::load(&board).unwrap();
        Draft::prepare(&board, &templates, ticket("Original"))
            .unwrap()
            .write(&board, None)
            .unwrap();
        let mut copy = ticket("Copy");
        copy.clone_of = Some("001".to_string());
        let draft = Draft::prepare(&board, &templates, copy).unwrap();
        // What an editor or a custom template might hand back.
        let stripped: String = draft
            .body
            .lines()
            .filter(|line| !line.starts_with("clone-of:"))
            .map(|line| format!("{line}\n"))
            .collect();
        let created = draft.write(&board, Some(stripped)).unwrap();
        assert_eq!(created.task.clone_of().as_deref(), Some("001"));
    }
}
