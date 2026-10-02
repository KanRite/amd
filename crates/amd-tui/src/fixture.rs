//! Boards on disk for the tests.

use std::path::Path;

use agile_md::board::{Board, Column};
use agile_md::model::{self, RepoBoard};

/// A repository at `root` holding a board with the given tickets, each
/// `(file name, column, tags)`. The title is `T<id>`.
pub fn repo(root: &Path, tickets: &[(&str, Column, &str)]) -> RepoBoard {
    let board = Board::at(root.join("tasks"));
    board.create().expect("board");
    for (name, column, tags) in tickets {
        let id = &name[..3];
        let text = format!(
            "---\nid: \"{id}\"\ntitle: \"T{id}\"\ntags: [{tags}]\n---\n\n## Notes\nbody of {id}\n"
        );
        std::fs::write(board.dir(*column).join(name), text).expect("ticket");
    }
    model::open_repo_board(root.to_path_buf(), board).expect("repo board")
}
