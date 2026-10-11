//! `amd idea` (ticket 131): the project's ideas, best first; putting one forward, voting, notes, and turning one into a
//! ticket. The work is amd-lib's `ideas`; this is the printing.

use std::ffi::OsStr;
use std::path::PathBuf;

use anyhow::{Context, Result};

use agile_md::board::Board;
use agile_md::documents::{Restored, Source};
use agile_md::ideas::{self, Idea, Person};
use agile_md::transition::Event;

use crate::form;

/// What `amd idea` was asked to do.
pub enum Action {
    List {
        all: bool,
    },
    New {
        title: String,
        anonymous: bool,
        edit: bool,
    },
    Show {
        reference: String,
    },
    Vote {
        reference: String,
        clicks: String,
    },
    Note {
        reference: String,
        text: String,
    },
    Ticket {
        reference: String,
    },
}

pub fn run(action: Action, editor: &str) -> Result<()> {
    let board = Board::discover()?;
    // Ideas belong to the project: its own documents repository only.
    let source = ideas::source(&board);
    let sources = std::slice::from_ref(&source);
    match action {
        Action::List { all } => list(sources, all),
        Action::New {
            title,
            anonymous,
            edit,
        } => {
            let root = cloned(&source)?;
            agile_md::archive::ensure_writable(&board)?;
            let body = if edit && form::available() {
                let written = form::body(&format!("# {title}\n\n"), OsStr::new(editor))?;
                written
                    .trim_start()
                    .strip_prefix(&format!("# {title}"))
                    .unwrap_or(&written)
                    .trim()
                    .to_string()
            } else {
                String::new()
            };
            let me = Person::of(&root)?;
            let (path, done) = ideas::create(&root, &title, &body, (!anonymous).then_some(&me))?;
            report(&done);
            println!("put forward {path} in {}", source.name);
            Ok(())
        }
        Action::Show { reference } => {
            let all = ideas::list(&source);
            show(ideas::find(&all, &reference)?);
            Ok(())
        }
        Action::Vote { reference, clicks } => {
            let delta = clicks_of(&clicks)?;
            let (idea, root) = locate(&board, sources, &reference)?;
            let me = Person::of(&root)?;
            let (now, done) = ideas::vote(&root, &idea.path, delta, &me)?;
            report(&done);
            let after = ideas::read(&root, &idea.source, &idea.path)?;
            println!(
                "your vote on {} is {}; its score is {}",
                after.title,
                ideas::signed(now),
                ideas::signed(after.score())
            );
            Ok(())
        }
        Action::Note { reference, text } => {
            let (idea, root) = locate(&board, sources, &reference)?;
            let me = Person::of(&root)?;
            report(&ideas::note(&root, &idea.path, &text, &me)?);
            println!("added a note to {}", idea.title);
            Ok(())
        }
        Action::Ticket { reference } => {
            let all = ideas::list(&source);
            let idea = ideas::find(&all, &reference)?;
            let root = root_of(sources, idea)?;
            let (id, events, done) = ideas::to_ticket(&board, &root, &idea.source, &idea.path)?;
            for event in events {
                match event {
                    Event::Info(text) => println!("{text}"),
                    Event::Warn(text) => eprintln!("amd: {text}"),
                }
            }
            report(&done);
            println!("{} -> ticket {id}", idea.title);
            Ok(())
        }
    }
}

/// `amd idea`: best first.
fn list(sources: &[Source], all: bool) -> Result<()> {
    let ideas: Vec<Idea> = ideas::list(&sources[0])
        .into_iter()
        .filter(|i| all || !i.archived)
        .collect();
    if ideas.is_empty() {
        println!("no ideas yet — `amd idea new \"<title>\"` puts one forward");
        return Ok(());
    }
    let width = ideas.iter().map(|i| i.slug().len()).max().unwrap_or(0);
    for idea in &ideas {
        let mut line = format!(
            "{:>3}  ({} vote{}, {} note{})  {:<width$}  {}",
            ideas::signed(idea.score()),
            idea.votes.len(),
            if idea.votes.len() == 1 { "" } else { "s" },
            idea.notes.len(),
            if idea.notes.len() == 1 { "" } else { "s" },
            idea.slug(),
            idea.title,
        );
        if let Some(author) = &idea.author {
            line.push_str(&format!(" — {}", name_of(author)));
        }
        if !idea.tickets.is_empty() {
            line.push_str(&format!("  -> {}", idea.tickets.join(", ")));
        }
        if idea.archived {
            line.push_str("  (archived)");
        }
        println!("{line}");
    }
    Ok(())
}

/// `amd idea show`: the write-up, then who voted and the notes.
fn show(idea: &Idea) {
    let (_, body) = agile_md::documents_edit::split_frontmatter(&idea.text);
    let body = body.split("\n## Notes").next().unwrap_or(body);
    println!("{}", body.trim_end());
    println!();
    println!(
        "{} in {} — score {}{}",
        idea.path,
        idea.source,
        ideas::signed(idea.score()),
        idea.author
            .as_deref()
            .map(|a| format!(", put forward by {}", name_of(a)))
            .unwrap_or_default()
    );
    if !idea.votes.is_empty() {
        let votes: Vec<String> = idea
            .votes
            .iter()
            .map(|v| format!("{} {}", v.name, ideas::signed(v.value)))
            .collect();
        println!("votes: {}", votes.join(", "));
    }
    if !idea.tickets.is_empty() {
        println!("tickets: {}", idea.tickets.join(", "));
    }
    if !idea.notes.is_empty() {
        println!("notes:");
        for note in &idea.notes {
            println!(
                "  {} ({}): {}",
                note.name,
                note.date,
                note.text.replace('\n', "\n    ")
            );
        }
    }
}

/// `+`, `++`, `---`, or a number such as `-2` or `+3`.
fn clicks_of(clicks: &str) -> Result<i32> {
    let clicks = clicks.trim();
    let n = if !clicks.is_empty() && clicks.chars().all(|c| c == '+') {
        clicks.len() as i32
    } else if !clicks.is_empty() && clicks.chars().all(|c| c == '-') {
        -(clicks.len() as i32)
    } else {
        clicks
            .trim_start_matches('+')
            .parse::<i32>()
            .ok()
            .filter(|n| *n != 0)
            .with_context(|| format!("a vote is +, ++, +++, -, -2 or -3, not {clicks:?}"))?
    };
    Ok(n.clamp(-2 * ideas::MAX_VOTE, 2 * ideas::MAX_VOTE))
}

/// The idea `reference` names, and the clone it's in; refused in an
/// archived project's own documents.
fn locate(board: &Board, sources: &[Source], reference: &str) -> Result<(Idea, PathBuf)> {
    let all = ideas::list(&sources[0]);
    let idea = ideas::find(&all, reference)?.clone();
    agile_md::archive::ensure_writable(board)?;
    let root = root_of(sources, &idea)?;
    Ok((idea, root))
}

fn root_of(sources: &[Source], idea: &Idea) -> Result<PathBuf> {
    sources
        .iter()
        .find(|s| s.name == idea.source)
        .context("that idea's repository isn't one of this project's")
        .and_then(cloned)
}

fn cloned(source: &Source) -> Result<PathBuf> {
    source
        .root
        .clone()
        .with_context(|| format!("{} isn't cloned on this machine", source.name))
}

/// `Sam Lee` from `Sam Lee <sam@acme.io>`.
fn name_of(author: &str) -> &str {
    author.split('<').next().unwrap_or(author).trim()
}

fn report(done: &Restored) {
    if let Some(warning) = &done.warning {
        eprintln!("amd: {warning}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_read_as_plus_and_minus_signs_or_numbers() {
        assert_eq!(clicks_of("+").unwrap(), 1);
        assert_eq!(clicks_of("+++").unwrap(), 3);
        assert_eq!(clicks_of("-").unwrap(), -1);
        assert_eq!(clicks_of("---").unwrap(), -3);
        assert_eq!(clicks_of("-2").unwrap(), -2);
        assert_eq!(clicks_of("+2").unwrap(), 2);
        assert!(clicks_of("0").is_err());
        assert!(clicks_of("up").is_err());
    }
}
