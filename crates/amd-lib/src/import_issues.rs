//! `amd import issues` (ticket 158): a repository's open GitHub issues
//! brought into the loose backlog as tickets tagged TRIAGE, so new work
//! reported on GitHub gets sorted like any other.
//!
//! Issues are read with `gh issue list`, the same `gh` that `amd init` and
//! Admin's access check already use. Each becomes a ticket through
//! [`Draft`], exactly as `amd new` makes one (template, `created-by`,
//! priority, the id counter), with a `source: "github:owner/repo#N"` key
//! saying where it came from.
//!
//! **An issue is imported once, ever.** Before importing, every `source`
//! the board has ever recorded is gathered from its git history as well as
//! from the files on disk, so a ticket that was abandoned or deleted still
//! counts: abandoning a TRIAGE ticket is how an issue is turned down, and a
//! re-run never brings it back.
//!
//! A label becomes a tag only when the project lists that tag; the rest are
//! named in the ticket's notes, so an import never fails over a label on a
//! board that prevents arbitrary tags. TRIAGE is reserved
//! ([`task::TRIAGE_TAG`]), so it's always allowed.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::board::Board;
use crate::create::{Draft, NewTicket};
use crate::templates::{DEFAULT_TEMPLATE, Templates};
use crate::{board_settings, branch, git, registry, task};

/// Where a ticket's issue came from, as written in its `source` key.
const SOURCE_KEY: &str = "source";

/// How many issues one run asks `gh` for, per repository.
const LIMIT: &str = "1000";

/// An open issue, as `gh issue list --json` describes it.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub author: Option<Author>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Label {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Author {
    pub login: String,
}

/// A GitHub repository to read issues from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// `owner/repo`, as `gh --repo` takes it.
    pub slug: String,
    /// The project's name for it, written into each ticket's `repository`
    /// when it's cloned on this machine; `None` leaves the field empty.
    pub repository: Option<String>,
}

/// One issue that will become (or, on a dry run, would become) a ticket.
#[derive(Debug, Clone)]
pub struct Planned {
    pub target: Target,
    pub issue: Issue,
    /// The id it gets, worked out from the board's counter.
    pub id: String,
    /// The ticket's title: the issue's, or `Issue N` when that has nothing
    /// to make a filename from.
    pub title: String,
    /// TRIAGE, then the issue's labels the project lists.
    pub tags: Vec<String>,
    /// Labels the project doesn't list, named in the notes instead.
    pub dropped: Vec<String>,
}

impl Planned {
    /// `github:owner/repo#12`, the value of the ticket's `source` key.
    pub fn source(&self) -> String {
        source_of(&self.target.slug, self.issue.number)
    }

    /// `#12 -> 160 "Title" (owner/repo) [TRIAGE, BUG]`: one line of what an
    /// import did, the same in `amd` and amdui.
    pub fn line(&self) -> String {
        let mut line = format!(
            "#{} -> {} \"{}\" ({}) [{}]",
            self.issue.number,
            self.id,
            self.title,
            self.target.slug,
            self.tags.join(", ")
        );
        if !self.dropped.is_empty() {
            line.push_str(&format!(
                ", labels not on the tag list: {}",
                self.dropped.join(", ")
            ));
        }
        line
    }
}

/// What an import will do: the issues it brings in, and how many it skips
/// because they were imported before.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub new: Vec<Planned>,
    pub already: usize,
}

impl Plan {
    /// `imported 2 issues, 1 already imported`, or the dry run's version.
    pub fn summary(&self, dry_run: bool) -> String {
        let n = self.new.len();
        if n == 0 {
            return match self.already {
                0 => "no open issues to import".to_string(),
                already => format!("nothing new to import, {already} already imported"),
            };
        }
        let mut line = format!(
            "{} {n} issue{}",
            if dry_run { "would import" } else { "imported" },
            if n == 1 { "" } else { "s" }
        );
        if self.already > 0 {
            line.push_str(&format!(", {} already imported", self.already));
        }
        if dry_run {
            line.push_str(" — nothing written");
        }
        line
    }
}

fn source_of(slug: &str, number: u64) -> String {
    format!("github:{slug}#{number}")
}

/// The repositories to read: `repo` when given (`owner/repo`, or a
/// repository the project declares), else every declared repository cloned
/// here whose `origin` is on GitHub, else the board's own repository when
/// that's on GitHub.
pub fn targets(board: &Board, repo: Option<&str>) -> Result<Vec<Target>> {
    let declared = board_settings::load(board).repos();
    let cloned: Vec<(String, String)> = declared
        .iter()
        .filter_map(|name| {
            let path = registry::clone_of(&board.root, name)?;
            Some((name.clone(), github_slug(&path)?))
        })
        .collect();

    if let Some(repo) = repo.map(str::trim).filter(|r| !r.is_empty()) {
        if let Some((name, slug)) = cloned.iter().find(|(name, _)| name == repo) {
            return Ok(vec![Target {
                slug: slug.clone(),
                repository: Some(name.clone()),
            }]);
        }
        if declared.iter().any(|name| name == repo) {
            bail!(
                "{repo} isn't cloned on this machine, or its origin isn't on GitHub — name it as owner/repo instead"
            );
        }
        let slug = repo.trim_start_matches("github.com/");
        if !is_slug(slug) {
            bail!("{repo} isn't a repository the project declares, or owner/repo");
        }
        let slug = slug.to_lowercase();
        return Ok(vec![Target {
            repository: cloned
                .iter()
                .find(|(_, s)| *s == slug)
                .map(|(name, _)| name.clone()),
            slug,
        }]);
    }

    if !cloned.is_empty() {
        return Ok(cloned
            .into_iter()
            .map(|(name, slug)| Target {
                slug,
                repository: Some(name),
            })
            .collect());
    }
    if let Some(slug) = github_slug(board.repo_root()) {
        return Ok(vec![Target {
            slug,
            repository: None,
        }]);
    }
    bail!(
        "no repository of this project is on GitHub here — name one: `amd import issues owner/repo`"
    )
}

/// `owner/repo` when `repo`'s `origin` is on github.com.
fn github_slug(repo: &Path) -> Option<String> {
    git::remote_identity(repo)?
        .strip_prefix("github.com/")
        .filter(|slug| is_slug(slug))
        .map(str::to_string)
}

fn is_slug(text: &str) -> bool {
    let mut parts = text.split('/');
    let ok = |part: Option<&str>| {
        part.is_some_and(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
    };
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

/// Every `source` this board has ever recorded: from its history, so a
/// ticket abandoned or deleted since still counts, and from the files on
/// disk (the archive included), for one not committed yet.
pub fn imported_sources(board: &Board) -> Result<HashSet<String>> {
    let mut sources = HashSet::new();
    let repo = board.repo_root();
    // No commits yet (a brand-new board) is simply no history to read.
    if git::head(repo).is_some() {
        let log = git::checked(
            repo,
            &[
                "log",
                "-p",
                "--all",
                "--format=",
                "-G",
                "^source: ",
                "--",
                &board.root.display().to_string(),
            ],
        )?;
        for line in log.lines() {
            if let Some(added) = line.strip_prefix('+')
                && let Some(value) = source_line(added)
            {
                sources.insert(value);
            }
        }
    }
    let mut files = Vec::new();
    collect_markdown(&board.root, &mut files);
    for path in files {
        if let Ok(text) = fs::read_to_string(&path)
            && let Some(value) = task::meta_in(&text, SOURCE_KEY)
        {
            sources.insert(value.to_string());
        }
    }
    Ok(sources)
}

/// The value of a `source: "…"` frontmatter line.
fn source_line(line: &str) -> Option<String> {
    let value = line.strip_prefix(SOURCE_KEY)?.strip_prefix(':')?;
    let value = task::unquote(value.trim());
    value.starts_with("github:").then(|| value.to_string())
}

fn collect_markdown(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_markdown(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

/// Asks `gh` for `slug`'s open issues, optionally only those with `label`.
pub fn fetch(slug: &str, label: Option<&str>) -> Result<Vec<Issue>> {
    let mut command = Command::new("gh");
    command.args([
        "issue",
        "list",
        "--repo",
        slug,
        "--state",
        "open",
        "--limit",
        LIMIT,
        "--json",
        "number,title,body,labels,url,author",
    ]);
    if let Some(label) = label {
        command.args(["--label", label]);
    }
    let out = command
        .stdin(Stdio::null())
        .output()
        .context("running gh issue list (is the GitHub CLI, gh, installed?)")?;
    if !out.status.success() {
        let detail = String::from_utf8_lossy(&out.stderr);
        bail!(
            "gh issue list --repo {slug} failed: {} (signed in? try `gh auth login`)",
            detail.trim().lines().last().unwrap_or_default()
        );
    }
    parse(&String::from_utf8_lossy(&out.stdout))
}

/// Reads `gh issue list --json`'s output.
pub fn parse(json: &str) -> Result<Vec<Issue>> {
    serde_json::from_str(json).context("reading gh's list of issues")
}

/// What importing from `targets` would do, asking GitHub through `gh`.
/// Writes nothing and spends no ids.
pub fn plan(board: &Board, targets: &[Target], label: Option<&str>) -> Result<Plan> {
    plan_with(board, targets, label, fetch)
}

/// [`plan`] with the issues coming from `fetch`, so it's testable without
/// GitHub.
pub fn plan_with(
    board: &Board,
    targets: &[Target],
    label: Option<&str>,
    fetch: impl Fn(&str, Option<&str>) -> Result<Vec<Issue>>,
) -> Result<Plan> {
    let seen = imported_sources(board)?;
    let listed = board_settings::load(board).tags();
    let mut next = board.next_id()?;
    let mut plan = Plan::default();
    for target in targets {
        let mut issues = fetch(&target.slug, label)?;
        issues.sort_by_key(|issue| issue.number);
        for issue in issues {
            if seen.contains(&source_of(&target.slug, issue.number)) {
                plan.already += 1;
                continue;
            }
            let (tags, dropped) = tags_for(&issue, &listed);
            let title = if branch::validate_sluggable(&issue.title).is_ok() {
                issue.title.trim().to_string()
            } else {
                format!("Issue {}", issue.number)
            };
            plan.new.push(Planned {
                target: target.clone(),
                id: format!("{next:03}"),
                title,
                tags,
                dropped,
                issue,
            });
            next += 1;
        }
    }
    Ok(plan)
}

/// TRIAGE plus the labels the project lists (uppercase); and the labels it
/// doesn't, as GitHub spells them.
fn tags_for(issue: &Issue, listed: &[String]) -> (Vec<String>, Vec<String>) {
    let mut tags = vec![task::TRIAGE_TAG.to_string()];
    let mut dropped = Vec::new();
    for label in &issue.labels {
        let tag = task::normalize_tag(&label.name);
        if tag.is_empty() {
            continue;
        }
        if listed.contains(&tag) || task::is_reserved_tag(&tag) {
            if !tags.contains(&tag) {
                tags.push(tag);
            }
        } else {
            dropped.push(label.name.clone());
        }
    }
    (tags, dropped)
}

/// Writes every planned ticket into the loose backlog. Commits nothing:
/// the caller wraps it in one auto-commit for the whole batch.
pub fn apply(board: &Board, plan: &Plan) -> Result<Vec<Planned>> {
    let templates = Templates::load(board)?;
    let mut done = Vec::new();
    for planned in &plan.new {
        let draft = Draft::prepare(
            board,
            &templates,
            NewTicket {
                title: planned.title.clone(),
                template: DEFAULT_TEMPLATE.to_string(),
                repository: planned.target.repository.clone().unwrap_or_default(),
                tags: planned.tags.clone(),
                ..Default::default()
            },
        )
        .with_context(|| format!("issue #{} ({})", planned.issue.number, planned.target.slug))?;
        let mut done_one = planned.clone();
        done_one.id = draft.id.clone();
        let text = with_notes(&draft.body, &notes(planned));
        let text = task::set_meta(&text, SOURCE_KEY, &format!("{:?}", planned.source()))?;
        draft.write(board, Some(text))?;
        done.push(done_one);
    }
    Ok(done)
}

/// The ticket's notes: where it came from, labels left off, then the
/// issue's own text.
fn notes(planned: &Planned) -> String {
    let issue = &planned.issue;
    let mut notes = format!(
        "Imported from GitHub issue [#{}]({})",
        issue.number, issue.url
    );
    if let Some(author) = &issue.author {
        notes.push_str(&format!(", opened by @{}", author.login));
    }
    notes.push_str(".\n");
    if planned.title != issue.title.trim() {
        notes.push_str(&format!("\nIssue title: {}\n", issue.title.trim()));
    }
    if !planned.dropped.is_empty() {
        notes.push_str(&format!(
            "\nLabels not on the project's tag list: {}\n",
            planned.dropped.join(", ")
        ));
    }
    let body = issue.body.trim();
    if !body.is_empty() {
        notes.push('\n');
        notes.push_str(&body.replace("\r\n", "\n"));
        notes.push('\n');
    }
    notes
}

/// `rendered` with `notes` under its `## Notes` heading, or appended as a
/// `## Notes` section when the template has none.
fn with_notes(rendered: &str, notes: &str) -> String {
    const HEADING: &str = "## Notes\n";
    match rendered.find(HEADING) {
        Some(at) => {
            let (head, rest) = rendered.split_at(at + HEADING.len());
            let rest = rest.trim_start_matches('\n');
            format!("{head}\n{notes}\n{rest}")
        }
        None => format!("{}\n\n{HEADING}\n{notes}", rendered.trim_end()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("git")
            .success();
        assert!(ok, "git {args:?}");
    }

    fn board() -> (TempDir, Board) {
        let dir = tempfile::tempdir().expect("temp dir");
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.email", "t@t.co"]);
        git(dir.path(), &["config", "user.name", "t"]);
        let board = Board {
            root: dir.path().join("tasks"),
        };
        board.create().expect("board");
        (dir, board)
    }

    const JSON: &str = r#"[
      {"number": 7, "title": "Basket total is wrong", "body": "Off by a penny.\r\nEvery time.",
       "labels": [{"name": "bug"}, {"name": "needs-info"}], "url": "https://github.com/acme/web/issues/7",
       "author": {"login": "sam"}},
      {"number": 3, "title": "🎉🎉", "body": "", "labels": [], "url": "https://github.com/acme/web/issues/3",
       "author": {"login": "jo"}},
      {"number": 9, "title": "Café menu in Ελληνικά", "labels": [], "url": "u"}
    ]"#;

    fn web() -> Vec<Target> {
        vec![Target {
            slug: "acme/web".into(),
            repository: None,
        }]
    }

    fn canned(_: &str, _: Option<&str>) -> Result<Vec<Issue>> {
        parse(JSON)
    }

    #[test]
    fn gh_json_is_read() {
        let issues = parse(JSON).unwrap();
        assert_eq!(issues.len(), 3);
        assert_eq!(issues[0].labels[1].name, "needs-info");
        assert_eq!(issues[0].author.as_ref().unwrap().login, "sam");
        assert_eq!(issues[2].body, "");
        assert!(issues[2].author.is_none());
    }

    #[test]
    fn listed_labels_become_tags_and_the_rest_are_noted() {
        let issue = &parse(JSON).unwrap()[0];
        let (tags, dropped) = tags_for(issue, &["BUG".to_string()]);
        assert_eq!(tags, ["TRIAGE", "BUG"]);
        assert_eq!(dropped, ["needs-info"]);
    }

    #[test]
    fn slugs_are_owner_and_repo() {
        assert!(is_slug("acme/web"));
        assert!(is_slug("a-b/c.d_e"));
        assert!(!is_slug("acme"));
        assert!(!is_slug("acme/web/extra"));
        assert!(!is_slug("acme/ web"));
    }

    #[test]
    fn a_plan_writes_nothing_and_numbers_from_the_counter() {
        let (_dir, board) = board();
        let plan = plan_with(&board, &web(), None, canned).unwrap();
        let ids: Vec<_> = plan
            .new
            .iter()
            .map(|p| (p.issue.number, p.id.clone()))
            .collect();
        assert_eq!(
            ids,
            [(3, "001".into()), (7, "002".into()), (9, "003".into())]
        );
        // An emoji-only title can't make a filename.
        assert_eq!(plan.new[0].title, "Issue 3");
        assert!(board.tasks().unwrap().is_empty());
        assert_eq!(board.next_id().unwrap(), 1);
    }

    #[test]
    fn applied_tickets_carry_triage_the_source_and_the_issue_text() {
        let (_dir, board) = board();
        let plan = plan_with(&board, &web(), None, canned).unwrap();
        let done = apply(&board, &plan).unwrap();
        assert_eq!(done.len(), 3);
        let tasks = board.tasks().unwrap();
        let basket = tasks
            .iter()
            .find(|t| t.title() == "Basket total is wrong")
            .unwrap();
        assert!(basket.has_tag("TRIAGE"));
        assert_eq!(basket.meta("source").as_deref(), Some("github:acme/web#7"));
        let text = fs::read_to_string(&basket.path).unwrap();
        assert!(text.contains("[#7](https://github.com/acme/web/issues/7), opened by @sam"));
        assert!(text.contains("Labels not on the project's tag list: bug, needs-info"));
        assert!(text.contains("Off by a penny.\nEvery time."));
        assert!(text.contains("## Acceptance criteria"));
        let party = tasks.iter().find(|t| t.title() == "Issue 3").unwrap();
        assert!(
            fs::read_to_string(&party.path)
                .unwrap()
                .contains("Issue title: 🎉🎉")
        );
    }

    #[test]
    fn an_issue_is_never_imported_twice_even_once_its_ticket_is_gone() {
        let (dir, board) = board();
        let plan = plan_with(&board, &web(), None, canned).unwrap();
        apply(&board, &plan).unwrap();
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-qm", "import"]);

        let again = plan_with(&board, &web(), None, canned).unwrap();
        assert!(again.new.is_empty());
        assert_eq!(again.already, 3);

        // Deleted and committed: only history remembers it.
        for task in board.tasks().unwrap() {
            fs::remove_file(&task.path).unwrap();
        }
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-qm", "gone"]);
        let after = plan_with(&board, &web(), None, canned).unwrap();
        assert!(after.new.is_empty());
        assert_eq!(after.already, 3);
    }

    #[test]
    fn uncommitted_tickets_count_as_imported_too() {
        let (_dir, board) = board();
        let plan = plan_with(&board, &web(), None, canned).unwrap();
        apply(&board, &plan).unwrap();
        assert_eq!(plan_with(&board, &web(), None, canned).unwrap().already, 3);
    }

    #[test]
    fn notes_go_under_the_notes_heading() {
        let rendered = "---\nid: \"001\"\n---\n\n## Notes\n\n\n## Acceptance criteria\n\n- [ ]\n";
        assert_eq!(
            with_notes(rendered, "Hello.\n"),
            "---\nid: \"001\"\n---\n\n## Notes\n\nHello.\n\n## Acceptance criteria\n\n- [ ]\n"
        );
        assert_eq!(
            with_notes("---\n---\nbody\n", "Hello.\n"),
            "---\n---\nbody\n\n## Notes\n\nHello.\n"
        );
    }
}
