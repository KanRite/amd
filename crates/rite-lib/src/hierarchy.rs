//! Parent and child (ticket 261): the hierarchy around a ticket, as `rite
//! tree <ref> --children` prints it and riteui's hierarchy panel draws it.
//!
//! A different shape from the requires tree ([`crate::requires::family`]):
//! each ticket has at most one parent, so it's a plain tree, from the
//! topmost ancestor down through every child and theirs. A child is a
//! ticket whose `parent` names it, or one its `## Children` list links.
//! The same [`TreeNode`]s as the requires tree, every edge
//! [`Edge::Child`], so a front end draws both alike. One board only, as
//! links are. Done tickets are left out unless `include_done`, except the
//! ticket asked about and its ancestors, which hold the tree together.

use anyhow::{Result, bail};

use crate::board::{Board, Kind};
use crate::links::{children_stems, same_id};
use crate::requires::{Edge, TreeNode, place};
use crate::task::Task;

/// How far up or down it goes, so a loop in hand-edited parents ends.
const MAX_DEPTH: usize = 32;

/// The hierarchy `around` is in, or every hierarchy on the board with
/// `None`: one root each, nested. Empty when it has no parent and no
/// children.
pub fn hierarchy(
    board: &Board,
    around: Option<&Task>,
    include_done: bool,
) -> Result<Vec<TreeNode>> {
    let tasks = board.tasks()?;
    let parents: Vec<Option<usize>> = tasks.iter().map(|t| parent_of(t, &tasks)).collect();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); tasks.len()];
    for (i, task) in tasks.iter().enumerate() {
        for stem in children_stems(task) {
            if let Some(j) = tasks.iter().position(|t| t.stem == stem)
                && j != i
                && !children[i].contains(&j)
            {
                children[i].push(j);
            }
        }
    }
    for (j, parent) in parents.iter().enumerate() {
        if let Some(i) = *parent
            && i != j
            && !children[i].contains(&j)
        {
            children[i].push(j);
        }
    }
    for list in &mut children {
        list.sort_by_key(|&j| tasks[j].id);
    }
    let tree = Tree {
        board,
        tasks: &tasks,
        children: &children,
        include_done,
    };
    match around {
        Some(task) => {
            let Some(asked) = tasks.iter().position(|t| t.path == task.path) else {
                bail!("{} isn't on this board's columns", task.id_display());
            };
            if parents[asked].is_none() && children[asked].is_empty() {
                return Ok(Vec::new());
            }
            // Up to the topmost ancestor; the path down to the ticket is
            // kept even when done.
            let mut path = vec![asked];
            let mut top = asked;
            while let Some(up) = parents[top] {
                if path.contains(&up) || path.len() > MAX_DEPTH {
                    break;
                }
                path.push(up);
                top = up;
            }
            let mut drawn = Vec::new();
            Ok(vec![tree.draw(
                top,
                Edge::Root,
                Some(asked),
                &path,
                &mut drawn,
                0,
            )])
        }
        None => {
            let mut drawn = Vec::new();
            let mut out = Vec::new();
            for i in 0..tasks.len() {
                if parents[i].is_none()
                    && !children[i].is_empty()
                    && (include_done || !done(&tasks[i]))
                {
                    let node = tree.draw(i, Edge::Root, None, &[], &mut drawn, 0);
                    if !node.children.is_empty() {
                        out.push(node);
                    }
                }
            }
            Ok(out)
        }
    }
}

struct Tree<'a> {
    board: &'a Board,
    tasks: &'a [Task],
    children: &'a [Vec<usize>],
    include_done: bool,
}

impl Tree<'_> {
    fn draw(
        &self,
        i: usize,
        edge: Edge,
        current: Option<usize>,
        keep: &[usize],
        drawn: &mut Vec<usize>,
        depth: usize,
    ) -> TreeNode {
        let task = &self.tasks[i];
        let mut node = TreeNode {
            id: task.id_display(),
            title: task.title(),
            file: task.file_name(),
            column: task.column.as_str().to_string(),
            epic: task.epic.clone(),
            place: place(self.board, task),
            done: done(task),
            edge,
            unmet: Vec::new(),
            stub: None,
            seen: false,
            current: current == Some(i),
            children: Vec::new(),
        };
        if drawn.contains(&i) {
            node.seen = true;
            return node;
        }
        drawn.push(i);
        if depth >= MAX_DEPTH {
            return node;
        }
        for &j in &self.children[i] {
            if !self.include_done && done(&self.tasks[j]) && !keep.contains(&j) {
                continue;
            }
            node.children
                .push(self.draw(j, Edge::Child, current, keep, drawn, depth + 1));
        }
        node
    }
}

fn done(task: &Task) -> bool {
    task.column.is(Kind::Done)
}

/// The ticket `task`'s `parent` names: `014`, `14`, or an old `[[014-slug]]`.
fn parent_of(task: &Task, tasks: &[Task]) -> Option<usize> {
    let raw = task.parent()?;
    let raw = raw.trim().trim_start_matches("[[").trim_end_matches("]]");
    let id: String = raw.chars().take_while(char::is_ascii_digit).collect();
    if id.is_empty() {
        return None;
    }
    tasks
        .iter()
        .position(|t| same_id(&id, t) && t.path != task.path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Column;

    fn write(board: &Board, column: Column, id: u32, slug: &str, parent: &str, body: &str) {
        let text =
            format!("---\nid: \"{id:03}\"\ntitle: \"{slug}\"\nparent: \"{parent}\"\n---\n\n{body}");
        std::fs::write(board.dir(column).join(format!("{id:03}-{slug}.md")), text).unwrap();
    }

    fn board() -> (tempfile::TempDir, Board) {
        let dir = tempfile::tempdir().unwrap();
        let board = Board::at(dir.path().join("tasks"));
        board.create().unwrap();
        // 001 › 002 › 004, 001 › 003 (done), and 005 alone.
        write(
            &board,
            Column::Backlog,
            1,
            "big",
            "",
            "## Children\n\n- [[002-part]]\n",
        );
        write(&board, Column::Todo, 2, "part", "001", "");
        write(&board, Column::Done, 3, "finished", "001", "");
        write(&board, Column::Backlog, 4, "smaller", "[[002-part]]", "");
        write(&board, Column::Backlog, 5, "alone", "", "");
        (dir, board)
    }

    fn ids(nodes: &[TreeNode]) -> Vec<String> {
        nodes.iter().map(|n| n.id.clone()).collect()
    }

    #[test]
    fn from_the_top_ancestor_down_with_the_ticket_marked() {
        let (_dir, board) = board();
        let around = board.find("004").unwrap();
        let trees = hierarchy(&board, Some(&around), false).unwrap();
        assert_eq!(ids(&trees), ["001"]);
        let top = &trees[0];
        assert_eq!(top.edge, Edge::Root);
        // 003 is done, so left out.
        assert_eq!(ids(&top.children), ["002"]);
        let part = &top.children[0];
        assert_eq!(part.edge, Edge::Child);
        assert_eq!(ids(&part.children), ["004"]);
        assert!(part.children[0].current);
        assert!(!part.current);
        let with_done = hierarchy(&board, Some(&around), true).unwrap();
        assert_eq!(ids(&with_done[0].children), ["002", "003"]);
    }

    #[test]
    fn a_done_ticket_asked_about_is_shown_and_a_lone_one_has_none() {
        let (_dir, board) = board();
        let done = board.find("003").unwrap();
        let trees = hierarchy(&board, Some(&done), false).unwrap();
        assert_eq!(ids(&trees[0].children), ["002", "003"]);
        let alone = board.find("005").unwrap();
        assert!(hierarchy(&board, Some(&alone), true).unwrap().is_empty());
    }

    #[test]
    fn every_hierarchy_on_the_board_and_a_loop_ends() {
        let (_dir, board) = board();
        assert_eq!(ids(&hierarchy(&board, None, false).unwrap()), ["001"]);
        // Hand-edited into a loop: 006's parent is 007 and 007's is 006.
        write(&board, Column::Backlog, 6, "a", "007", "");
        write(&board, Column::Backlog, 7, "b", "006", "");
        let a = board.find("006").unwrap();
        let trees = hierarchy(&board, Some(&a), false).unwrap();
        assert_eq!(trees.len(), 1);
    }
}
