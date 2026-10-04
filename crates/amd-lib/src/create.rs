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
    /// A ref (id or slug) for the ticket this one sits under.
    pub parent: Option<String>,
    /// Refs for the tickets this one relates to.
    pub related: Vec<String>,
    pub tags: Vec<String>,
    /// Values for the template's own `extra.*` fields.
    pub extra: BTreeMap<String, String>,
}

/// A ticket that has been resolved and rendered, but not yet written.
#[derive(Debug)]
pub struct Draft {
    pub path: PathBuf,
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
}

/// What changed when a draft was written, so a front end can report it.
#[derive(Debug)]
pub struct Created {
    pub task: Task,
    pub branch: String,
    /// Tickets that gained a link back to this one.
    pub linked: Vec<String>,
}

impl Draft {
    /// Resolve and render, without writing anything.
    pub fn prepare(board: &Board, templates: &Templates, ticket: NewTicket) -> Result<Draft> {
        branch::validate_sluggable(&ticket.title)?;

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
        let dir = board.dir(Column::Backlog);
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
            related: related.clone(),
            tags: ticket.tags,
            created,
            timestamp,
            column: Column::Backlog.to_string(),
            author: git::config(&board.root, "user.name").unwrap_or_else(|| "unknown".to_string()),
            email: git::config(&board.root, "user.email").unwrap_or_default(),
            board: board.name(),
            template: ticket.template.clone(),
            extra: ticket.extra,
        };
        let body = templates.render(&ticket.template, &context)?;

        Ok(Draft {
            path,
            id,
            number,
            slug,
            branch: branch_name,
            body,
            parent: parent_task,
            related,
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

        let task = Task::from_path(&self.path, Column::Backlog)
            .ok_or_else(|| anyhow::anyhow!("{} is not a task file", self.path.display()))?;
        Ok(Created {
            task,
            branch: self.branch,
            linked,
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
}
