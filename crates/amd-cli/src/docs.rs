//! `amd docs` (ticket 179): the project's documents from the command line —
//! the start page, a search, and the page tree the start page links to.
//! Reading only: the documents repository is a clone like any other, so
//! changes are made there directly.

use anyhow::{Context, Result, bail};

use agile_md::board::Board;
use agile_md::documents::{self, Kind};
use agile_md::documents_edit;

/// What `amd docs` was asked to do.
pub enum Action {
    Start { path: bool },
    Search { query: String },
    Tree,
}

pub fn run(action: Action) -> Result<()> {
    let board = Board::discover()?;
    let sources = documents::sources(&board);
    let own = sources
        .iter()
        .find(|s| s.kind == Kind::Own)
        .context("this project has no documents repository")?;
    let own_root = || {
        own.root
            .clone()
            .with_context(|| format!("{} isn't cloned on this machine", own.name))
    };
    match action {
        Action::Start { path } => {
            let root = own_root()?;
            let Some(start) = documents_edit::start_page(&root) else {
                bail!(
                    "{} has no start page: pin one in amdui, or add a README.md",
                    own.name
                );
            };
            if path {
                println!("{}", root.join(&start).display());
            } else {
                print!("{}", documents::read(&root, &start)?);
            }
            Ok(())
        }
        Action::Search { query } => {
            let mut found = 0;
            for source in &sources {
                let Some(root) = &source.root else { continue };
                for hit in documents_edit::search(root, &query, 3)? {
                    found += 1;
                    if hit.line == 0 {
                        println!("{}/{}: {}", source.name, hit.path, hit.title);
                    } else {
                        println!("{}/{}:{}: {}", source.name, hit.path, hit.line, hit.text);
                    }
                }
            }
            if found == 0 {
                eprintln!("nothing matches '{}'", query.trim());
            }
            Ok(())
        }
        Action::Tree => {
            let root = own_root()?;
            let Some(start) = documents_edit::start_page(&root) else {
                bail!(
                    "{} has no start page: pin one in amdui, or add a README.md",
                    own.name
                );
            };
            for page in documents_edit::link_tree(&root, &start)? {
                println!("{}{}  {}", "  ".repeat(page.depth), page.title, page.path);
            }
            Ok(())
        }
    }
}
