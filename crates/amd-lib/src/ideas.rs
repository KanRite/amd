//! Ideas (ticket 131): developers' ideas for apps and improvements, before
//! anyone means to do them. Each is a markdown file in the project's own
//! documents repository's `ideas/` folder (never the shared one: ideas
//! belong to a project), pushed straight to main like every document
//! (ADR 0007):
//!
//! ```markdown
//! ---
//! title: "Dark mode for the board"
//! author: "Sam Lee <sam@acme.io>"
//! created: "2026-10-11"
//! votes: ["Sam Lee <sam@acme.io> +3", "Priya Shah <priya@acme.io> -1"]
//! tickets: ["acme-tickets#042"]
//! ---
//! # Dark mode for the board
//!
//! The write-up.
//!
//! ## Notes
//!
//! - **Priya Shah**, 2026-10-11: Worth it if it follows the OS setting.
//! ```
//!
//! `author` is left out when its author would rather not say. Everyone,
//! the author included, votes up to three + or three − (each click one
//! step, kept per email) and adds notes. Ideas are ordered by score, the
//! sum of the votes. A file without this frontmatter is still an idea, with
//! no votes yet.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::board::Board;
use crate::documents::{self, Kind, Restored, Source, commit_change, commit_reapplied};

/// The project's own documents repository, where its ideas live.
pub fn source(board: &Board) -> Source {
    documents::sources(board)
        .into_iter()
        .find(|s| s.kind == Kind::Own)
        .unwrap_or_else(|| Source {
            kind: Kind::Own,
            name: String::new(),
            root: None,
        })
}
use crate::documents_edit::{is_archived, split_frontmatter};
use crate::git;
use crate::task;

/// The folder ideas live in, in any documents repository.
pub const FOLDER: &str = "ideas";
/// The furthest one person's vote goes either way.
pub const MAX_VOTE: i32 = 3;
const NOTES: &str = "## Notes";

/// Someone, as git knows them in a repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub email: String,
}

impl Person {
    /// Git's `user.name` and `user.email` in `repo`.
    pub fn of(repo: &Path) -> Result<Person> {
        let email = git::config(repo, "user.email")
            .filter(|e| !e.trim().is_empty())
            .context("git has no user.email here; set one to vote or add notes")?;
        let name = git::config(repo, "user.name")
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| email.clone());
        Ok(Person { name, email })
    }

    fn display(&self) -> String {
        format!("{} <{}>", self.name, self.email)
    }

    /// `Name <email>`, or a bare name.
    fn parse(text: &str) -> Person {
        match text
            .trim()
            .strip_suffix('>')
            .and_then(|t| t.rsplit_once('<'))
        {
            Some((name, email)) => Person {
                name: name.trim().to_string(),
                email: email.trim().to_string(),
            },
            None => Person {
                name: text.trim().to_string(),
                email: String::new(),
            },
        }
    }
}

/// One person's vote on an idea: −3 to +3, never 0 (that's no vote).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vote {
    pub name: String,
    pub email: String,
    pub value: i32,
}

/// A comment under an idea.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub name: String,
    pub date: String,
    pub text: String,
}

/// An idea, as read from its file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Idea {
    /// The documents repository it's in.
    pub source: String,
    /// Relative to that repository: `ideas/<file>.md`.
    pub path: String,
    pub title: String,
    /// Who put it forward, when they said.
    pub author: Option<String>,
    pub created: String,
    pub votes: Vec<Vote>,
    pub notes: Vec<Note>,
    /// The tickets made from it, as `<tickets repository>#<id>`.
    pub tickets: Vec<String>,
    pub archived: bool,
    /// Its whole text.
    pub text: String,
}

impl Idea {
    /// The sum of everyone's votes.
    pub fn score(&self) -> i32 {
        self.votes.iter().map(|v| v.value).sum()
    }

    /// `email`'s vote, 0 when they haven't voted.
    pub fn vote_of(&self, email: &str) -> i32 {
        self.votes
            .iter()
            .find(|v| v.email.eq_ignore_ascii_case(email))
            .map_or(0, |v| v.value)
    }

    /// The part after `ideas/` and before `.md`, which `<ref>` matches.
    pub fn slug(&self) -> &str {
        let file = self.path.rsplit('/').next().unwrap_or(&self.path);
        file.strip_suffix(".md").unwrap_or(file)
    }
}

/// Reads one idea file.
pub fn parse(source: &str, path: &str, text: &str) -> Idea {
    let (front, body) = split_frontmatter(text);
    let get = |key: &str| front.and_then(|f| value_of(f, key));
    let file = path.rsplit('/').next().unwrap_or(path);
    let title = get("title")
        .map(|v| unquote(&v))
        .filter(|t| !t.is_empty())
        .or_else(|| heading(body))
        .unwrap_or_else(|| file.trim_end_matches(".md").to_string());
    let votes = get("votes")
        .map(|v| flow_list(&v))
        .unwrap_or_default()
        .iter()
        .filter_map(|line| {
            let (who, value) = line.trim().rsplit_once(' ')?;
            let value: i32 = value.trim_start_matches('+').parse().ok()?;
            let person = Person::parse(who);
            (value != 0).then(|| Vote {
                name: person.name,
                email: person.email,
                value: value.clamp(-MAX_VOTE, MAX_VOTE),
            })
        })
        .collect();
    Idea {
        source: source.to_string(),
        path: path.to_string(),
        title,
        author: get("author").map(|v| unquote(&v)).filter(|a| !a.is_empty()),
        created: get("created").map(|v| unquote(&v)).unwrap_or_default(),
        votes,
        notes: notes_in(body),
        tickets: get("tickets").map(|v| flow_list(&v)).unwrap_or_default(),
        archived: is_archived(text),
        text: text.to_string(),
    }
}

/// Every idea in `source`'s `ideas/` folder, best first: by score, then by
/// how many voted, then newest. Archived ones are included, marked.
pub fn list(source: &Source) -> Vec<Idea> {
    let mut ideas = Vec::new();
    for source in std::iter::once(source) {
        let Some(root) = &source.root else { continue };
        let Ok(entries) = fs::read_dir(root.join(FOLDER)) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_file = entry.file_type().is_ok_and(|t| t.is_file());
            if name.starts_with('.') || !is_file || !name.to_ascii_lowercase().ends_with(".md") {
                continue;
            }
            let path = format!("{FOLDER}/{name}");
            let Ok(text) = fs::read_to_string(entry.path()) else {
                continue;
            };
            ideas.push(parse(&source.name, &path, &text));
        }
    }
    sort(&mut ideas);
    ideas
}

/// Best first: score, then voters, then newest, then title.
pub fn sort(ideas: &mut [Idea]) {
    ideas.sort_by(|a, b| {
        b.score()
            .cmp(&a.score())
            .then(b.votes.len().cmp(&a.votes.len()))
            .then(b.created.cmp(&a.created))
            .then(a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
}

/// The one idea `reference` names among `ideas`: its exact slug, else the
/// only one whose slug contains it.
pub fn find<'a>(ideas: &'a [Idea], reference: &str) -> Result<&'a Idea> {
    let reference = reference.trim().trim_end_matches(".md");
    let reference = reference.strip_prefix("ideas/").unwrap_or(reference);
    if let Some(idea) = ideas.iter().find(|i| i.slug() == reference) {
        return Ok(idea);
    }
    let matches: Vec<&Idea> = ideas
        .iter()
        .filter(|i| i.slug().contains(reference))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one),
        [] => bail!("no idea matches '{reference}'"),
        many => bail!(
            "'{reference}' matches {} ideas: {}",
            many.len(),
            many.iter().map(|i| i.slug()).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// Reads the idea at `path` in `root`.
pub fn read(root: &Path, source: &str, path: &str) -> Result<Idea> {
    let text = documents::read(root, path)?;
    Ok(parse(source, path, &text))
}

/// Puts a new idea forward: `ideas/<slug>.md`, with `author` when given.
/// `body` is the write-up, under the title. Returns its path.
pub fn create(
    root: &Path,
    title: &str,
    body: &str,
    author: Option<&Person>,
) -> Result<(String, Restored)> {
    let title = title.trim();
    let slug = task::slugify(title);
    if slug.is_empty() {
        bail!("an idea needs a title with letters or numbers in it");
    }
    let path = format!("{FOLDER}/{slug}.md");
    let full = root.join(&path);
    if full.exists() {
        bail!("there's already an idea at {path}");
    }
    let (today, _) = crate::templates::today();
    let mut front = vec![
        format!("title: {}", quote(title)),
        format!("created: {}", quote(&today)),
    ];
    if let Some(author) = author {
        front.insert(1, format!("author: {}", quote(&author.display())));
    }
    let body = body.trim();
    let mut text = format!("---\n{}\n---\n# {title}\n", front.join("\n"));
    if !body.is_empty() {
        text.push('\n');
        text.push_str(body);
        text.push('\n');
    }
    let done = commit_change(root, &format!("idea: {title}"), || {
        fs::create_dir_all(root.join(FOLDER)).context("making ideas/")?;
        fs::write(&full, &text).with_context(|| format!("writing {path}"))?;
        Ok(vec![PathBuf::from(&path)])
    })?;
    Ok((path, done))
}

/// Moves `who`'s vote on the idea at `path` by `delta` clicks (+1 or −1
/// each), kept within ±[`MAX_VOTE`]. Returns their vote now.
pub fn vote(root: &Path, path: &str, delta: i32, who: &Person) -> Result<(i32, Restored)> {
    let full = documents::resolve(root, path)?;
    documents::ensure_clean(root, &[path])?;
    let mut now = 0;
    let done = commit_reapplied(root, || {
        let text = fs::read_to_string(&full).with_context(|| format!("reading {path}"))?;
        let idea = parse("", path, &text);
        let before = idea.vote_of(&who.email);
        now = (before + delta).clamp(-MAX_VOTE, MAX_VOTE);
        if now == before {
            bail!(
                "your vote on {} is {} already, as far as it goes",
                idea.title,
                signed(before)
            );
        }
        let mut votes: Vec<String> = idea
            .votes
            .iter()
            .filter(|v| !v.email.eq_ignore_ascii_case(&who.email))
            .map(|v| format!("{} <{}> {}", v.name, v.email, signed(v.value)))
            .collect();
        if now != 0 {
            votes.push(format!("{} {}", who.display(), signed(now)));
        }
        let value = if votes.is_empty() {
            None
        } else {
            Some(json_list(&votes))
        };
        fs::write(&full, with_key(&text, "votes", value.as_deref()))
            .with_context(|| format!("writing {path}"))?;
        Ok((
            format!(
                "vote: {} {} (now {})",
                idea.title,
                signed(delta),
                signed(now)
            ),
            vec![PathBuf::from(path)],
        ))
    })?;
    Ok((now, done))
}

/// Adds `who`'s note under the idea at `path`, dated today.
pub fn note(root: &Path, path: &str, text: &str, who: &Person) -> Result<Restored> {
    let note = text.trim();
    if note.is_empty() {
        bail!("a note needs some text");
    }
    let full = documents::resolve(root, path)?;
    documents::ensure_clean(root, &[path])?;
    let (today, _) = crate::templates::today();
    commit_reapplied(root, || {
        let current = fs::read_to_string(&full).with_context(|| format!("reading {path}"))?;
        let title = parse("", path, &current).title;
        fs::write(&full, with_note(&current, &who.name, &today, note))
            .with_context(|| format!("writing {path}"))?;
        Ok((format!("note: {title}"), vec![PathBuf::from(path)]))
    })
}

/// Makes a backlog ticket on `board` from the idea at `path` in `root`,
/// carrying `source: "idea:<repository>/<path>"`, and records the ticket
/// on the idea. Returns the ticket's id and what both commits said.
pub fn to_ticket(
    board: &Board,
    root: &Path,
    source: &str,
    path: &str,
) -> Result<(String, Vec<crate::transition::Event>, Restored)> {
    use crate::create::{Draft, NewTicket};
    use crate::templates::{DEFAULT_TEMPLATE, Templates};
    crate::archive::ensure_writable(board)?;
    documents::ensure_clean(root, &[path])?;
    let idea = read(root, source, path)?;
    let tickets_repo = board
        .repo_root()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let templates = Templates::load(board)?;
    let draft = Draft::prepare(
        board,
        &templates,
        NewTicket {
            title: idea.title.clone(),
            template: DEFAULT_TEMPLATE.to_string(),
            ..Default::default()
        },
    )?;
    let id = draft.id.clone();
    let mut notes = format!(
        "From the idea [{}]({source}/{path}) in {source}",
        idea.title
    );
    if let Some(author) = &idea.author {
        notes.push_str(&format!(", put forward by {}", Person::parse(author).name));
    }
    notes.push_str(&format!(" (score {}).\n", signed(idea.score())));
    let text = crate::model::splice_body(&draft.body, &notes);
    let text = task::set_meta(
        &text,
        "source",
        &format!("{:?}", format!("idea:{source}/{path}")),
    )?;
    let title = idea.title.clone();
    let ((), warning) = crate::transition::with_auto_commit(board, "new", &id, &title, || {
        draft.write(board, Some(text)).map(|_| ())
    })?;
    let mut events = vec![crate::transition::Event::Info(format!(
        "created backlog ticket {id} \"{title}\""
    ))];
    if let Some(warning) = warning {
        events.push(crate::transition::Event::Warn(warning));
    }

    let reference = format!("{tickets_repo}#{id}");
    let full = documents::resolve(root, path)?;
    let done = commit_reapplied(root, || {
        let current = fs::read_to_string(&full).with_context(|| format!("reading {path}"))?;
        let mut tickets = parse("", path, &current).tickets;
        if !tickets.contains(&reference) {
            tickets.push(reference.clone());
        }
        fs::write(
            &full,
            with_key(&current, "tickets", Some(&json_list(&tickets))),
        )
        .with_context(|| format!("writing {path}"))?;
        Ok((
            format!("idea: {title} -> {reference}"),
            vec![PathBuf::from(path)],
        ))
    })?;
    Ok((id, events, done))
}

/// `+2`, `-1`, `0`.
pub fn signed(value: i32) -> String {
    if value > 0 {
        format!("+{value}")
    } else {
        value.to_string()
    }
}

/// The raw value of `key` in a frontmatter block.
fn value_of(front: &str, key: &str) -> Option<String> {
    front
        .lines()
        .filter(|l| l.trim_end() != "---")
        .find_map(|line| {
            let (k, v) = line.split_once(':')?;
            (k.trim() == key).then(|| v.trim().to_string())
        })
}

/// `text` with `key` set to `value` (already written as YAML), or taken
/// away with `None`; other lines kept as they were. A file without
/// frontmatter gains it.
fn with_key(text: &str, key: &str, value: Option<&str>) -> String {
    let (front, body) = split_frontmatter(text);
    let mut lines: Vec<String> = front
        .map(|f| {
            f.lines()
                .filter(|l| l.trim_end() != "---")
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let at = lines
        .iter()
        .position(|l| l.split_once(':').is_some_and(|(k, _)| k.trim() == key));
    match (at, value) {
        (Some(i), Some(v)) => lines[i] = format!("{key}: {v}"),
        (Some(i), None) => {
            lines.remove(i);
        }
        (None, Some(v)) => lines.push(format!("{key}: {v}")),
        (None, None) => {}
    }
    if lines.is_empty() {
        return body.to_string();
    }
    format!("---\n{}\n---\n{body}", lines.join("\n"))
}

/// `text` with a note appended under `## Notes`, which is made at the end
/// when there isn't one. A note's later lines are indented to stay in it.
fn with_note(text: &str, name: &str, date: &str, note: &str) -> String {
    let name = name.replace('*', "");
    let item = format!(
        "- **{name}**, {date}: {}",
        note.lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines.iter().position(|l| l.trim_end() == NOTES) else {
        return format!("{}\n\n{NOTES}\n\n{item}\n", text.trim_end());
    };
    // The end of the Notes section: the next heading of its level or above.
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.starts_with("# ") || l.starts_with("## "))
        .map_or(lines.len(), |i| start + 1 + i);
    let mut kept: Vec<&str> = lines[..end].to_vec();
    while kept.last().is_some_and(|l| l.trim().is_empty()) && kept.len() > start + 1 {
        kept.pop();
    }
    let mut out = kept.join("\n");
    out.push_str(if kept.len() == start + 1 {
        "\n\n"
    } else {
        "\n"
    });
    out.push_str(&item);
    out.push('\n');
    if end < lines.len() {
        out.push('\n');
        out.push_str(&lines[end..].join("\n"));
        out.push('\n');
    }
    out
}

/// The notes under `## Notes`.
fn notes_in(body: &str) -> Vec<Note> {
    let mut notes: Vec<Note> = Vec::new();
    let mut inside = false;
    for line in body.lines() {
        if line.trim_end() == NOTES {
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        if line.starts_with("# ") || line.starts_with("## ") {
            break;
        }
        if let Some(rest) = line.strip_prefix("- **")
            && let Some((name, rest)) = rest.split_once("**")
        {
            let rest = rest.trim_start_matches(',').trim_start();
            let (date, text) = rest.split_once(':').unwrap_or(("", rest));
            notes.push(Note {
                name: name.trim().to_string(),
                date: date.trim().to_string(),
                text: text.trim().to_string(),
            });
        } else if let Some(more) = line.strip_prefix("  ")
            && let Some(last) = notes.last_mut()
        {
            last.text.push('\n');
            last.text.push_str(more.trim_end());
        }
    }
    notes
}

/// The first `# ` heading.
fn heading(body: &str) -> Option<String> {
    body.lines()
        .find_map(|l| l.strip_prefix("# "))
        .map(|t| t.trim().trim_end_matches('#').trim().to_string())
        .filter(|t| !t.is_empty())
}

/// A YAML double-quoted string (also valid JSON).
fn quote(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| format!("\"{text}\""))
}

/// A quoted value read back; a plain one as it is.
fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.starts_with('"')
        && let Ok(s) = serde_json::from_str::<String>(value)
    {
        return s;
    }
    value.trim_matches(['"', '\'']).to_string()
}

/// A flow list written by [`json_list`], or a looser one (`[a, b]`).
fn flow_list(value: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(value.trim()).unwrap_or_else(|_| task::parse_list(value))
}

fn json_list(items: &[String]) -> String {
    serde_json::to_string(items).unwrap_or_else(|_| "[]".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sam() -> Person {
        Person {
            name: "Sam Lee".into(),
            email: "sam@acme.io".into(),
        }
    }

    #[test]
    fn a_file_without_frontmatter_is_an_idea_with_no_votes() {
        let idea = parse(
            "acme-documents",
            "ideas/replay.md",
            "# Record and replay\n\nText.\n",
        );
        assert_eq!(idea.title, "Record and replay");
        assert_eq!(idea.score(), 0);
        assert!(idea.votes.is_empty() && idea.notes.is_empty() && idea.author.is_none());
        assert_eq!(idea.slug(), "replay");
    }

    #[test]
    fn frontmatter_votes_notes_and_tickets_read_back() {
        let text = "---\ntitle: \"Dark mode\"\nauthor: \"Sam Lee <sam@acme.io>\"\ncreated: \"2026-10-11\"\n\
votes: [\"Sam Lee <sam@acme.io> +3\", \"Priya Shah <priya@acme.io> -1\"]\ntickets: [\"acme-tickets#042\"]\n---\n\
# Dark mode\n\nWrite-up.\n\n## Notes\n\n- **Priya Shah**, 2026-10-11: Follow the OS.\n  Or a switch.\n- **Sam Lee**, 2026-10-12: Agreed.\n";
        let idea = parse("acme-documents", "ideas/dark-mode.md", text);
        assert_eq!(idea.title, "Dark mode");
        assert_eq!(idea.author.as_deref(), Some("Sam Lee <sam@acme.io>"));
        assert_eq!(idea.score(), 2);
        assert_eq!(idea.vote_of("priya@acme.io"), -1);
        assert_eq!(idea.tickets, vec!["acme-tickets#042"]);
        assert_eq!(idea.notes.len(), 2);
        assert_eq!(idea.notes[0].text, "Follow the OS.\nOr a switch.");
        assert_eq!(idea.notes[1].date, "2026-10-12");
    }

    #[test]
    fn keys_are_set_and_removed_keeping_the_rest() {
        let text = "---\ntitle: \"X\"\nstatus: archived\n---\n# X\n";
        let set = with_key(text, "votes", Some("[\"a <a@b> +1\"]"));
        assert!(
            set.contains("status: archived\nvotes: [\"a <a@b> +1\"]\n---"),
            "{set}"
        );
        let gone = with_key(&set, "votes", None);
        assert_eq!(gone, text);
        assert_eq!(with_key("# X\n", "votes", None), "# X\n");
        assert_eq!(
            with_key("# X\n", "tickets", Some("[]")),
            "---\ntickets: []\n---\n# X\n"
        );
    }

    #[test]
    fn notes_go_under_their_heading_which_is_made_when_missing() {
        let first = with_note("# X\n\nBody.\n", "Sam Lee", "2026-10-11", "Nice.\nReally.");
        assert_eq!(
            first,
            "# X\n\nBody.\n\n## Notes\n\n- **Sam Lee**, 2026-10-11: Nice.\n  Really.\n"
        );
        let second = with_note(&first, "Priya", "2026-10-12", "Me too.");
        assert!(
            second.ends_with("  Really.\n- **Priya**, 2026-10-12: Me too.\n"),
            "{second}"
        );
        let after = with_note(
            "# X\n\n## Notes\n\n- **A**, d: one\n\n## Later\n\nmore\n",
            "B",
            "e",
            "two",
        );
        assert_eq!(
            after,
            "# X\n\n## Notes\n\n- **A**, d: one\n- **B**, e: two\n\n## Later\n\nmore\n"
        );
        assert_eq!(notes_in(&second).len(), 2);
    }

    #[test]
    fn best_first_then_most_voters_then_newest() {
        let make = |title: &str, created: &str, votes: &[i32]| Idea {
            title: title.into(),
            created: created.into(),
            votes: votes
                .iter()
                .enumerate()
                .map(|(i, v)| Vote {
                    name: format!("p{i}"),
                    email: format!("p{i}@x"),
                    value: *v,
                })
                .collect(),
            ..parse("", "ideas/x.md", "")
        };
        let mut ideas = vec![
            make("sinks", "2026-01-01", &[-2]),
            make("none old", "2026-01-01", &[]),
            make("two voters", "2026-01-01", &[1, 1]),
            make("one voter", "2026-01-01", &[2]),
            make("none new", "2026-02-01", &[]),
        ];
        sort(&mut ideas);
        let order: Vec<&str> = ideas.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(
            order,
            ["two voters", "one voter", "none new", "none old", "sinks"]
        );
    }

    #[test]
    fn find_takes_a_whole_slug_or_a_unique_part() {
        let ideas = vec![
            parse("", "ideas/dark-mode.md", "# Dark mode\n"),
            parse("", "ideas/dark-mode-plus.md", "# Dark mode plus\n"),
            parse("", "ideas/replay.md", "# Replay\n"),
        ];
        assert_eq!(find(&ideas, "dark-mode").unwrap().title, "Dark mode");
        assert_eq!(find(&ideas, "rep").unwrap().title, "Replay");
        assert!(find(&ideas, "dark").is_err());
        assert!(find(&ideas, "nothing").is_err());
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?} in {}", dir.display());
    }

    /// A bare origin and `n` clones of it, each its own person.
    fn clones(tmp: &Path, n: usize) -> Vec<PathBuf> {
        let origin = tmp.join("origin.git");
        std::fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "-q", "--bare", "-b", "main"]);
        let seed = tmp.join("seed");
        std::fs::create_dir_all(seed.join("ideas")).unwrap();
        std::fs::write(seed.join("ideas/replay.md"), "# Replay\n").unwrap();
        git(&seed, &["init", "-q", "-b", "main"]);
        git(
            &seed,
            &["-c", "user.email=s@x", "-c", "user.name=s", "add", "-A"],
        );
        git(
            &seed,
            &[
                "-c",
                "user.email=s@x",
                "-c",
                "user.name=s",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "-m",
                "seed",
            ],
        );
        git(&seed, &["push", "-q", origin.to_str().unwrap(), "main"]);
        (0..n)
            .map(|i| {
                let dir = tmp.join(format!("clone{i}"));
                git(
                    tmp,
                    &[
                        "clone",
                        "-q",
                        origin.to_str().unwrap(),
                        dir.to_str().unwrap(),
                    ],
                );
                git(&dir, &["config", "user.email", &format!("p{i}@x")]);
                git(&dir, &["config", "user.name", &format!("Person {i}")]);
                git(&dir, &["config", "commit.gpgsign", "false"]);
                dir
            })
            .collect()
    }

    #[test]
    fn votes_clamp_at_three_and_back_to_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = &clones(tmp.path(), 1)[0];
        let me = Person::of(dir).unwrap();
        for expected in [1, 2, 3] {
            assert_eq!(vote(dir, "ideas/replay.md", 1, &me).unwrap().0, expected);
        }
        assert!(
            vote(dir, "ideas/replay.md", 1, &me).is_err(),
            "+3 is as far as it goes"
        );
        vote(dir, "ideas/replay.md", -1, &me).unwrap();
        vote(dir, "ideas/replay.md", -1, &me).unwrap();
        assert_eq!(vote(dir, "ideas/replay.md", -1, &me).unwrap().0, 0);
        let text = std::fs::read_to_string(dir.join("ideas/replay.md")).unwrap();
        assert_eq!(text, "# Replay\n", "no vote left, no frontmatter left");
    }

    #[test]
    fn votes_from_two_clones_both_count() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = clones(tmp.path(), 2);
        let (a, b) = (&dirs[0], &dirs[1]);
        // Votes from two clones in turn: each catches up first, so every
        // vote is made on the file with the others' in it.
        vote(a, "ideas/replay.md", 1, &Person::of(a).unwrap()).unwrap();
        vote(b, "ideas/replay.md", -1, &Person::of(b).unwrap()).unwrap();
        vote(a, "ideas/replay.md", 1, &Person::of(a).unwrap()).unwrap();
        git(b, &["pull", "-q", "--ff-only"]);
        let idea = parse(
            "",
            "ideas/replay.md",
            &std::fs::read_to_string(b.join("ideas/replay.md")).unwrap(),
        );
        assert_eq!(idea.vote_of("p0@x"), 2);
        assert_eq!(idea.vote_of("p1@x"), -1);
        assert_eq!(idea.score(), 1);
    }

    #[test]
    fn a_rejected_push_is_made_again_on_the_new_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = clones(tmp.path(), 2);
        let (a, b) = (dirs[0].clone(), dirs[1].clone());
        // A pre-push hook in b lets a vote first, once, exactly as two
        // people clicking at the same moment would race.
        let hook = b.join(".git/hooks/pre-push");
        let race = format!(
            "#!/bin/sh\nrm -f \"$0\"\nunset $(git rev-parse --local-env-vars)\ncd '{}' && git -c user.email=p0@x -c user.name='Person 0' -c commit.gpgsign=false commit -q -am race >/dev/null 2>&1; true\n",
            a.display()
        );
        // a's change: its own vote written by hand, then pushed by the hook.
        let text = "---\nvotes: [\"Person 0 <p0@x> +1\"]\n---\n# Replay\n";
        std::fs::write(a.join("ideas/replay.md"), text).unwrap();
        let race = race.replace(
            "; true",
            &format!(
                " && git -C '{}' push -q origin main >/dev/null 2>&1; true",
                a.display()
            ),
        );
        std::fs::write(&hook, race).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let (now, done) = vote(&b, "ideas/replay.md", -1, &Person::of(&b).unwrap()).unwrap();
        assert_eq!(now, -1);
        assert!(done.pushed, "{:?}", done.warning);
        let idea = parse(
            "",
            "ideas/replay.md",
            &std::fs::read_to_string(b.join("ideas/replay.md")).unwrap(),
        );
        assert_eq!(idea.vote_of("p0@x"), 1, "a's vote kept");
        assert_eq!(idea.vote_of("p1@x"), -1, "b's vote made again on top");
    }

    #[test]
    fn create_and_note() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = &clones(tmp.path(), 1)[0];
        let (path, done) = create(dir, "Dark mode", "Follow the OS.", Some(&sam())).unwrap();
        assert!(done.pushed);
        assert_eq!(path, "ideas/dark-mode.md");
        assert!(create(dir, "Dark mode", "", None).is_err(), "no clash");
        let (anon, _) = create(dir, "Quiet one", "", None).unwrap();
        assert!(read(dir, "d", &anon).unwrap().author.is_none());
        note(dir, &path, "Yes please", &Person::of(dir).unwrap()).unwrap();
        let idea = read(dir, "d", &path).unwrap();
        assert_eq!(idea.author.as_deref(), Some("Sam Lee <sam@acme.io>"));
        assert_eq!(idea.notes.len(), 1);
        assert_eq!(idea.notes[0].name, "Person 0");
        assert!(idea.text.contains("Follow the OS."));
    }
}
