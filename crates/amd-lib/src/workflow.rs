//! A board's workflow (ticket 223): its columns in order, each a folder
//! with a label and a [`Kind`] that decides how it behaves. Every rule asks
//! a column's kind, never its name, so a team can have a Triage before the
//! backlog or a Review after doing and the lease, the WIP limit, the branch
//! and the done rules all follow.
//!
//! The standard workflow is the four columns every board has always had.
//! A custom one is written `Label=folder:kind`, comma separated, in the
//! order shown (`Triage=triage:backlog, Backlog=backlog:backlog, …`); the
//! four standard folders are always among them, so a board never loses the
//! places its tickets already are.

use anyhow::{Result, bail};

use crate::board::{Column, Kind};

/// One column as the board shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnDef {
    pub column: Column,
    /// What it's called on screen.
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workflow {
    columns: Vec<ColumnDef>,
}

/// Folders a column can't use: they're the board's own.
const RESERVED: [&str; 2] = ["archive", "templates"];

impl Workflow {
    /// Backlog, To do, Doing, Done.
    pub fn standard() -> Workflow {
        Workflow {
            columns: Column::STANDARD
                .into_iter()
                .map(|column| ColumnDef {
                    label: standard_label(column).to_string(),
                    column,
                })
                .collect(),
        }
    }

    /// A custom workflow from its written form, checked: every kind at
    /// least once, folders unique, plain and not the board's own, and the
    /// four standard folders present with their standard kinds.
    pub fn parse(text: &str) -> Result<Workflow> {
        let mut columns: Vec<ColumnDef> = Vec::new();
        for entry in text.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            let (label, rest) = entry.split_once('=').unwrap_or(("", entry));
            let Some((folder, kind)) = rest.split_once(':') else {
                bail!("column {entry:?} needs a folder and a kind: Label=folder:kind");
            };
            let folder = folder.trim();
            let Some(kind) = Kind::parse(kind) else {
                bail!(
                    "column {folder:?} has kind {kind:?}: it must be backlog, todo, doing or done"
                );
            };
            let plain = !folder.is_empty()
                && folder
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if !plain {
                bail!("column folder {folder:?} must be lowercase letters, digits and dashes");
            }
            if RESERVED.contains(&folder) {
                bail!("{folder:?} is the board's own folder, not a column");
            }
            if columns.iter().any(|c| c.column.as_str() == folder) {
                bail!("column folder {folder:?} appears twice");
            }
            let column = Column::new(folder, kind);
            let label = match label.trim() {
                "" => standard_label(column).to_string(),
                label => label.to_string(),
            };
            columns.push(ColumnDef { column, label });
        }
        for kind in Kind::ALL {
            if !columns.iter().any(|c| c.column.is(kind)) {
                bail!("a workflow needs at least one {} column", kind.as_str());
            }
        }
        for standard in Column::STANDARD {
            match columns
                .iter()
                .find(|c| c.column.as_str() == standard.as_str())
            {
                Some(c) if c.column.kind() == standard.kind() => {}
                Some(_) => bail!(
                    "the {} column must stay a {} column",
                    standard.as_str(),
                    standard.kind().as_str()
                ),
                None => bail!(
                    "the {} column can be renamed but not left out",
                    standard.as_str()
                ),
            }
        }
        Ok(Workflow { columns })
    }

    /// Columns as given, unchecked: an edit in progress, checked by
    /// [`parse`](Self::parse) before it's saved.
    pub(crate) fn from_defs(columns: Vec<ColumnDef>) -> Workflow {
        Workflow { columns }
    }

    /// Its written form, the inverse of [`parse`](Self::parse).
    pub fn to_text(&self) -> String {
        self.columns
            .iter()
            .map(|c| {
                format!(
                    "{}={}:{}",
                    c.label,
                    c.column.as_str(),
                    c.column.kind().as_str()
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Whether it's the standard four, as they come.
    pub fn is_standard(&self) -> bool {
        *self == Workflow::standard()
    }

    /// The columns, left to right.
    pub fn columns(&self) -> impl Iterator<Item = Column> + '_ {
        self.columns.iter().map(|c| c.column)
    }

    /// The columns with their labels, left to right.
    pub fn defs(&self) -> &[ColumnDef] {
        &self.columns
    }

    /// Where `column` sits, left to right; `None` if it isn't one of this
    /// board's.
    pub fn position(&self, column: Column) -> Option<usize> {
        self.columns.iter().position(|c| c.column == column)
    }

    /// What `column` is called on screen.
    pub fn label(&self, column: Column) -> String {
        self.columns
            .iter()
            .find(|c| c.column == column)
            .map(|c| c.label.clone())
            .unwrap_or_else(|| column.as_str().to_string())
    }

    /// Whether a ticket may move straight from `from` to `to`: one column
    /// at a time, forwards or back (ticket 083). Work goes through every
    /// column, so the history shows each step.
    pub fn next_to(&self, from: Column, to: Column) -> bool {
        match (self.position(from), self.position(to)) {
            (Some(a), Some(b)) => a.abs_diff(b) == 1,
            _ => false,
        }
    }

    /// The first column of `kind`, left to right. Every workflow has one.
    pub fn first(&self, kind: Kind) -> Column {
        self.columns()
            .find(|c| c.is(kind))
            .unwrap_or(Column::STANDARD[kind as usize])
    }

    /// Every column of `kind`, left to right.
    pub fn of_kind(&self, kind: Kind) -> impl Iterator<Item = Column> + '_ {
        self.columns().filter(move |c| c.is(kind))
    }

    /// The column called `name`: its folder or its label, ignoring case.
    pub fn find(&self, name: &str) -> Option<Column> {
        let name = name.trim();
        self.columns
            .iter()
            .find(|c| {
                c.column.as_str().eq_ignore_ascii_case(name) || c.label.eq_ignore_ascii_case(name)
            })
            .map(|c| c.column)
    }

    /// The column whose folder is `folder`, exactly: how a ticket's path is
    /// read back into a column.
    pub fn by_folder(&self, folder: &str) -> Option<Column> {
        self.columns().find(|c| c.as_str() == folder)
    }

    /// Where new tickets land: the standard backlog.
    pub fn landing(&self) -> Column {
        Column::Backlog
    }
}

impl Default for Workflow {
    fn default() -> Workflow {
        Workflow::standard()
    }
}

/// The standard columns' labels, and a folder's own name otherwise.
fn standard_label(column: Column) -> &'static str {
    match column.as_str() {
        "backlog" => "Backlog",
        "todo" => "To do",
        "doing" => "Doing",
        "done" => "Done",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEAM: &str = "Triage=triage:backlog, Backlog=backlog:backlog, To do=todo:todo, \
                        Doing=doing:doing, Review=review:doing, Done=done:done, Released=released:done";

    #[test]
    fn the_standard_workflow_is_the_four_columns() {
        let w = Workflow::standard();
        assert_eq!(w.columns().collect::<Vec<_>>(), Column::STANDARD);
        assert_eq!(w.label(Column::Todo), "To do");
        assert!(w.next_to(Column::Backlog, Column::Todo));
        assert!(!w.next_to(Column::Backlog, Column::Doing));
        assert_eq!(w.find("TODO"), Some(Column::Todo));
        assert_eq!(w.find("to do"), Some(Column::Todo));
        assert!(w.is_standard());
        assert_eq!(Workflow::parse(&w.to_text()).unwrap(), w);
    }

    #[test]
    fn a_custom_workflow_reads_back_with_kinds_and_order() {
        let w = Workflow::parse(TEAM).unwrap();
        let review = w.find("review").unwrap();
        assert_eq!(review.kind(), Kind::Doing);
        assert_eq!(w.position(review), Some(4));
        assert!(w.next_to(Column::Doing, review));
        assert!(w.next_to(review, Column::Done));
        assert!(!w.next_to(Column::Doing, Column::Done));
        assert_eq!(w.first(Kind::Backlog).as_str(), "triage");
        assert_eq!(
            w.of_kind(Kind::Done)
                .map(Column::as_str)
                .collect::<Vec<_>>(),
            ["done", "released"]
        );
        assert_eq!(w.landing(), Column::Backlog);
        assert_eq!(Workflow::parse(&w.to_text()).unwrap(), w);
        assert!(!w.is_standard());
        // The same folder interned twice is the same column.
        assert_eq!(Column::new("review", Kind::Doing), review);
    }

    #[test]
    fn a_workflow_that_would_lose_something_is_refused() {
        let refused = |text: &str, says: &str| {
            let err = Workflow::parse(text).unwrap_err().to_string();
            assert!(err.contains(says), "{text}: {err}");
        };
        refused(
            "backlog:backlog, todo:todo, doing:doing",
            "at least one done column",
        );
        refused(
            "backlog:backlog, todo:todo, doing:doing, review:doing, review:doing, done:done",
            "appears twice",
        );
        refused(
            "backlog:backlog, todo:todo, wip:doing, done:done",
            "doing column can be renamed but not left out",
        );
        refused(
            "backlog:backlog, todo:todo, doing:done, review:doing, done:done",
            "doing column must stay a doing column",
        );
        refused(
            "backlog:backlog, todo:todo, doing:doing, done:finished",
            "kind",
        );
        refused(
            "backlog:backlog, todo:todo, doing:doing, done:done, Q A:doing",
            "lowercase",
        );
        refused(
            "backlog:backlog, todo:todo, doing:doing, done:done, archive:done",
            "board's own",
        );
    }
}
