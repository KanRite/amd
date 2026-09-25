//! A task: one markdown file, `NNN-slug.md`, living in a column directory.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::board::Column;

/// Longest slug we put in a filename (matches the pre-Rust `cut -c1-50`).
const MAX_SLUG: usize = 50;

/// Heading the child wikilinks are collected under.
const CHILDREN_HEADING: &str = "## Children";

/// A cheap fingerprint of a ticket's on-disk content, captured when it's read
/// with the intent of writing back to it, and checked again immediately
/// before the write actually happens — the lost-update fix from
/// `docs/adr/0007-concurrency-and-locking.md`. Not cryptographic: this only
/// needs to notice a change, not resist a deliberate collision.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ContentHash(u64);

fn hash_of(text: &str) -> ContentHash {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    ContentHash(hasher.finish())
}

/// A write was refused because the ticket changed since it was last read.
/// The caller's decision may no longer be valid — re-read and retry, rather
/// than the write silently discarding whatever changed it.
#[derive(Debug)]
pub struct Conflict {
    pub path: PathBuf,
}

impl std::fmt::Display for Conflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} changed since it was last read", self.path.display())
    }
}

impl std::error::Error for Conflict {}

#[derive(Clone, Debug)]
pub struct Task {
    pub path: PathBuf,
    pub column: Column,
    /// The `NNN` prefix, when the filename has one.
    pub id: Option<u32>,
    /// Filename without the `.md` extension, e.g. `001-first-task`.
    pub stem: String,
    /// The epic folder the ticket sits in, for tickets filed under one. The
    /// folder is what decides this — the `epic` frontmatter key follows it,
    /// not the other way round, so moving the file is what changes the epic.
    pub epic: Option<String>,
}

impl Task {
    /// Build a task from a path inside a column directory.
    pub fn from_path(path: &Path, column: Column) -> Option<Self> {
        if path.extension()? != "md" {
            return None;
        }
        let stem = path.file_stem()?.to_str()?.to_string();
        let id = stem
            .split_once('-')
            .and_then(|(head, _)| head.parse::<u32>().ok());
        Some(Self {
            path: path.to_path_buf(),
            column,
            id,
            stem,
            epic: None,
        })
    }

    pub fn file_name(&self) -> String {
        format!("{}.md", self.stem)
    }

    /// Look up a frontmatter key. Tolerant by design: task files are rendered
    /// from user-editable templates, so this reads the first `key: value` line
    /// rather than pretending to be a YAML parser.
    pub fn meta(&self, key: &str) -> Option<String> {
        let text = fs::read_to_string(&self.path).ok()?;
        meta_in(&text, key).map(str::to_string)
    }

    /// Fingerprint the ticket's current on-disk content. Call this right
    /// before deciding what to write, and pass the result to the write —
    /// if the file changed in between, the write is refused rather than
    /// silently discarding whatever changed it.
    pub fn hash(&self) -> Result<ContentHash> {
        let text = fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        Ok(hash_of(&text))
    }

    /// Read-modify-write against this ticket, refusing with [`Conflict`] if
    /// the file no longer matches `expected`. `modify` receives the freshly
    /// re-read, verified-current text and returns the replacement.
    fn write_checked(
        &self,
        expected: ContentHash,
        modify: impl FnOnce(&str) -> Result<String>,
    ) -> Result<()> {
        let text = fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        if hash_of(&text) != expected {
            return Err(Conflict {
                path: self.path.clone(),
            }
            .into());
        }
        let updated = modify(&text)?;
        fs::write(&self.path, updated).with_context(|| format!("writing {}", self.path.display()))
    }

    /// Id as displayed on the board: frontmatter first, then the filename.
    pub fn id_display(&self) -> String {
        self.meta("id")
            .filter(|id| !id.is_empty())
            .or_else(|| self.id.map(|n| format!("{n:03}")))
            .unwrap_or_else(|| "???".to_string())
    }

    /// Title as displayed on the board, falling back to the filename.
    pub fn title(&self) -> String {
        self.meta("title")
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| self.stem.clone())
    }

    /// Retitle the ticket. Only the frontmatter changes: the filename keeps
    /// its original slug, because the id is what every reference uses and
    /// renaming the file would break `git log --follow` for a cosmetic gain.
    pub fn set_title(&self, expected: ContentHash, title: &str) -> Result<()> {
        self.write_checked(expected, |text| {
            set_meta(text, "title", &format!("{title:?}"))
        })
    }

    /// Everything below the closing frontmatter fence — the part a human
    /// actually writes.
    pub fn body(&self) -> Result<String> {
        let text = fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        Ok(split_body(&text)
            .map(|(_, body)| body.to_string())
            .unwrap_or(text))
    }

    /// Replace the body, leaving the frontmatter exactly as it was.
    pub fn set_body(&self, expected: ContentHash, body: &str) -> Result<()> {
        self.write_checked(expected, |text| {
            let head = split_body(text)
                .map(|(head, _)| head.to_string())
                .ok_or_else(|| anyhow::anyhow!("no frontmatter in {}", self.path.display()))?;
            let mut out = head;
            out.push_str(body);
            if !out.ends_with('\n') {
                out.push('\n');
            }
            Ok(out)
        })
    }

    /// Who the ticket is assigned to, if anyone. Also reads `owner`, which the
    /// field was briefly called, so no board is left stranded.
    pub fn assignee(&self) -> Option<String> {
        self.meta("assignee")
            .or_else(|| self.meta("owner"))
            .filter(|who| !who.is_empty())
    }

    /// Assign the ticket, or clear it with an empty name.
    pub fn assign(&self, expected: ContentHash, who: &str) -> Result<()> {
        let quoted = format!("{who:?}");
        self.write_checked(expected, |text| set_meta(text, "assignee", &quoted))
    }

    /// Story points, when the ticket has been sized. Free-form on purpose:
    /// teams use 1/2/3/5/8, or T-shirt sizes, and the board only needs to show
    /// it back.
    pub fn points(&self) -> Option<String> {
        self.meta("points").filter(|points| !points.is_empty())
    }

    /// Size the ticket, or clear the size with an empty string.
    pub fn set_points(&self, expected: ContentHash, points: &str) -> Result<()> {
        self.write_checked(expected, |text| {
            set_meta(text, "points", &format!("{points:?}"))
        })
    }

    /// The epic recorded in the frontmatter. `Task::epic` — set from the
    /// folder — is the authority; this is what a reader outside the board
    /// sees, and the two are kept in step when a ticket is filed.
    pub fn epic_meta(&self) -> Option<String> {
        self.meta("epic").filter(|epic| !epic.is_empty())
    }

    /// Record the epic in the frontmatter, so the ticket says which epic it
    /// belongs to even when read on its own.
    pub fn set_epic_meta(&self, expected: ContentHash, epic: &str) -> Result<()> {
        self.write_checked(expected, |text| {
            set_meta(text, "epic", &format!("{epic:?}"))
        })
    }

    /// Where the ticket sits within its column, for boards ordered by hand.
    /// Absent means unranked, which sorts after everything ranked — the same
    /// treatment an unset priority gets, so a board nobody has dragged still
    /// reads in id order.
    pub fn order(&self) -> Option<f64> {
        self.meta("order")
            .filter(|value| !value.is_empty())
            .and_then(|value| value.trim().parse::<f64>().ok())
    }

    /// Rank the ticket within its column. Fractional on purpose: a card
    /// dropped between two others takes the midpoint of its neighbours, so one
    /// drag rewrites one file instead of renumbering the column — and two
    /// branches that both reorder collide over one ticket rather than all.
    pub fn set_order(&self, expected: ContentHash, rank: f64) -> Result<()> {
        self.write_checked(expected, |text| {
            set_meta(text, "order", &format!("\"{rank}\""))
        })
    }

    /// Title, assignee, points and body, applied together in **one** write.
    /// Required, not just tidier, once writes are hash-checked: a form that
    /// captures one hash when it opens and then calls four separate setters
    /// against it could never have the second one succeed, since the first
    /// write already changed the file the hash was checked against. This is
    /// what makes `docs/adr/0006-the-operation-vocabulary.md`'s "one write
    /// per action" true for the editor, which is also what
    /// `docs/adr/0007-concurrency-and-locking.md`'s optimistic writes assume.
    pub fn apply_edit(
        &self,
        expected: ContentHash,
        title: &str,
        assignee: &str,
        points: &str,
        body: &str,
    ) -> Result<()> {
        self.write_checked(expected, |text| {
            let text = set_meta(text, "title", &format!("{title:?}"))?;
            let text = set_meta(&text, "assignee", &format!("{assignee:?}"))?;
            let text = set_meta(&text, "points", &format!("{points:?}"))?;
            let head = split_body(&text)
                .map(|(head, _)| head.to_string())
                .ok_or_else(|| anyhow::anyhow!("no frontmatter in {}", self.path.display()))?;
            let mut out = head;
            out.push_str(body);
            if !out.ends_with('\n') {
                out.push('\n');
            }
            Ok(out)
        })
    }

    /// The branch type, if the ticket carries one. Falls back to the old
    /// `type` key.
    pub fn branch_type(&self) -> Option<String> {
        self.meta("branch-type")
            .or_else(|| self.meta("type"))
            .filter(|kind| !kind.is_empty())
    }

    /// The id of this ticket's parent, if it has one. Ids rather than paths,
    /// so the link survives a rename or a move between columns.
    pub fn parent(&self) -> Option<String> {
        self.meta("parent").filter(|parent| !parent.is_empty())
    }

    /// The tickets this one is related to: ids as recorded in `related`.
    pub fn related(&self) -> Vec<String> {
        self.meta("related")
            .map(|raw| parse_list(&raw))
            .unwrap_or_default()
    }

    /// This ticket's tags, as recorded in `tags`.
    pub fn tags(&self) -> Vec<String> {
        self.meta("tags")
            .map(|raw| parse_list(&raw))
            .unwrap_or_default()
    }

    /// Whether this ticket carries `tag`, matched exactly (tags are
    /// free-form labels, not normalised the way a title is for duplicate
    /// detection — `amd import` writes `DUPLICATE` verbatim, so filtering
    /// on it is exact too).
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags().iter().any(|known| known == tag)
    }

    /// Add ids to the `related` list, in id order and without duplicates.
    /// Returns whether the file changed.
    pub fn add_related(&self, expected: ContentHash, ids: &[String]) -> Result<bool> {
        let mut related = self.related();
        let mut changed = false;
        for id in ids {
            if !related.iter().any(|known| known == id) {
                related.push(id.clone());
                changed = true;
            }
        }
        if !changed {
            return Ok(false);
        }
        related.sort();
        self.write_checked(expected, |text| {
            set_meta(text, "related", &format!("[{}]", related.join(",")))
        })?;
        Ok(true)
    }

    /// Record a child under a `## Children` heading, as a wikilink an editor
    /// can follow. The heading is added when it isn't there yet.
    pub fn add_child_link(&self, expected: ContentHash, stem: &str) -> Result<bool> {
        let text = fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        let link = format!("- [[{stem}]]");
        if text.lines().any(|line| line.trim() == link) {
            return Ok(false);
        }
        self.write_checked(expected, |text| {
            let mut updated = text.trim_end().to_string();
            if !text.contains(CHILDREN_HEADING) {
                updated.push_str("\n\n");
                updated.push_str(CHILDREN_HEADING);
                updated.push('\n');
            }
            // Append under the heading: children are listed in creation
            // order, which is id order.
            updated.push('\n');
            updated.push_str(&link);
            updated.push('\n');
            Ok(updated)
        })?;
        Ok(true)
    }

    /// The branch this ticket is worked on, as recorded when it was created.
    /// Editing `branch-name` changes what `amd todo` switches to on entry.
    /// Falls back to the old `branch` key.
    pub fn branch(&self) -> Option<String> {
        self.meta("branch-name")
            .or_else(|| self.meta("branch"))
            .filter(|branch| !branch.is_empty())
    }
}

/// `[003,007]` -> `["003", "007"]`. Tolerant of spaces and quotes, since a
/// human may well have typed the line.
pub(crate) fn parse_list(raw: &str) -> Vec<String> {
    raw.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|item| item.trim().trim_matches(['"', '\'']).to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// Replace a frontmatter value, adding the key before the closing fence when
/// the ticket doesn't have it yet — tasks written by an older template, or by
/// hand, still get linked.
pub(crate) fn set_meta(text: &str, key: &str, value: &str) -> Result<String> {
    if !text.starts_with("---") {
        bail!("no frontmatter to update");
    }
    let mut out = String::with_capacity(text.len() + value.len());
    let mut lines = text.lines();
    let Some(open) = lines.next() else {
        bail!("no frontmatter to update");
    };
    out.push_str(open);
    out.push('\n');

    let mut written = false;
    let mut closed = false;
    for line in lines {
        if !closed && line.trim_end() == "---" {
            if !written {
                out.push_str(&format!("{key}: {value}\n"));
                written = true;
            }
            closed = true;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if !closed
            && !written
            && let Some(rest) = line.strip_prefix(key)
            && rest.starts_with(':')
        {
            out.push_str(&format!("{key}: {value}\n"));
            written = true;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !closed {
        bail!("frontmatter is not closed");
    }
    Ok(out)
}

/// Search a task's frontmatter block (or, if it has none, the whole file) for
/// the first `key: value` line.
/// Split a task file into its frontmatter (fences included, trailing newline
/// kept) and the body below it. `None` when there is no closing fence, so a
/// half-written file is left alone rather than rewritten into nonsense.
pub fn split_body(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---\n")?;
    // 1 for the newline the match starts with, 4 for "---\n".
    let head_len = "---\n".len() + end + 1 + "---\n".len();
    Some(text.split_at(head_len))
}

pub(crate) fn meta_in<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let mut lines = text.lines();
    let fenced = text.starts_with("---");
    if fenced {
        lines.next();
    }
    for line in lines {
        if fenced && line.trim_end() == "---" {
            break;
        }
        if let Some(rest) = line.strip_prefix(key)
            && let Some(value) = rest.strip_prefix(':')
        {
            return Some(unquote(value.trim()));
        }
    }
    None
}

pub(crate) fn unquote(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[bytes.len() - 1] == bytes[0]
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

/// `"Publish to Marketplace"` -> `"publish-to-marketplace"`.
///
/// The result is ASCII, because the slug is a filename and a branch name and
/// those are worth keeping boring. Letters and digits from any script are
/// **transliterated** rather than dropped, so `Ünïcödé` becomes `unicode` and
/// `北京` becomes `bei-jing` instead of collapsing to noise.
///
/// Symbols are not transliterated: an emoji in a title is decoration, and
/// `party 🎉` should be `party`, not `party-tada`.
pub fn slugify(title: &str) -> String {
    let mut slug = String::with_capacity(title.len());
    let mut pending_dash = false;
    let push = |ch: char, slug: &mut String, pending_dash: &mut bool| {
        if *pending_dash && !slug.is_empty() {
            slug.push('-');
        }
        slug.push(ch.to_ascii_lowercase());
        *pending_dash = false;
    };
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            push(ch, &mut slug, &mut pending_dash);
        } else if ch.is_alphanumeric() {
            // A letter or digit from another script: use its closest ASCII.
            for ch in deunicode::deunicode_char(ch).unwrap_or_default().chars() {
                if ch.is_ascii_alphanumeric() {
                    push(ch, &mut slug, &mut pending_dash);
                } else {
                    pending_dash = true;
                }
            }
        } else {
            pending_dash = true;
        }
    }
    slug.truncate(MAX_SLUG);
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_lowercases_and_joins_words() {
        assert_eq!(slugify("Publish to Marketplace"), "publish-to-marketplace");
    }

    #[test]
    fn slugify_collapses_runs_and_trims_edges() {
        assert_eq!(slugify("  --Fix:  the *thing*!  "), "fix-the-thing");
    }

    #[test]
    fn slugify_truncates_without_a_trailing_dash() {
        let slug = slugify(&"word ".repeat(30));
        assert!(slug.len() <= MAX_SLUG, "{slug}");
        assert!(!slug.ends_with('-'), "{slug}");
    }

    #[test]
    fn slugify_transliterates_rather_than_dropping() {
        assert_eq!(
            slugify(r#"Ünïcödé "quoted" ticket"#),
            "unicode-quoted-ticket"
        );
        assert_eq!(slugify("Größe ändern"), "grosse-andern");
        assert_eq!(slugify("Añadir sesión"), "anadir-sesion");
        assert_eq!(slugify("Москва"), "moskva");
        assert_eq!(slugify("北京"), "bei-jing");
    }

    #[test]
    fn slugify_treats_symbols_as_separators_not_words() {
        // deunicode would call this one "tada"; a branch name shouldn't say so.
        assert_eq!(slugify("party 🎉 time"), "party-time");
        assert_eq!(slugify("100% done"), "100-done");
    }

    #[test]
    fn slugify_is_empty_when_nothing_survives() {
        // The caller rejects these — an empty slug can't make a branch name.
        assert_eq!(slugify("***"), "");
        assert_eq!(slugify(""), "");
        assert_eq!(slugify("🎉🎉"), "");
    }

    #[test]
    fn from_path_parses_the_id_prefix() {
        let task = Task::from_path(Path::new("/b/todo/007-do-it.md"), Column::Todo).unwrap();
        assert_eq!(task.id, Some(7));
        assert_eq!(task.stem, "007-do-it");
        assert_eq!(task.file_name(), "007-do-it.md");
    }

    #[test]
    fn from_path_ignores_non_markdown_and_unnumbered_files() {
        assert!(Task::from_path(Path::new("/b/todo/.gitkeep"), Column::Todo).is_none());
        let task = Task::from_path(Path::new("/b/todo/notes.md"), Column::Todo).unwrap();
        assert_eq!(task.id, None);
    }

    #[test]
    fn meta_reads_the_frontmatter_block_only() {
        let text = "---\nid: \"001\"\ntitle: \"First task\"\ntags: [x,y]\n---\n\n## Notes\n\ntitle: not this one\n";
        assert_eq!(meta_in(text, "id"), Some("001"));
        assert_eq!(meta_in(text, "title"), Some("First task"));
        assert_eq!(meta_in(text, "tags"), Some("[x,y]"));
        assert_eq!(meta_in(text, "missing"), None);
    }

    #[test]
    fn meta_falls_back_to_the_whole_file_without_frontmatter() {
        assert_eq!(meta_in("# Heading\ntitle: Bare\n", "title"), Some("Bare"));
    }

    #[test]
    fn lists_are_parsed_leniently() {
        assert_eq!(parse_list("[003,007]"), ["003", "007"]);
        assert_eq!(parse_list("[ \"003\", 007 ]"), ["003", "007"]);
        assert_eq!(parse_list("[]"), [] as [String; 0]);
    }

    /// A task backed by a real file, so the read/write pair can be exercised.
    fn ranked(dir: &std::path::Path, name: &str, body: &str) -> Task {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        Task::from_path(&path, Column::Todo).unwrap()
    }

    #[test]
    fn tags_reads_the_list_and_has_tag_matches_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let task = ranked(
            dir.path(),
            "001-a.md",
            "---\nid: \"001\"\ntitle: \"A\"\ntags: [DUPLICATE,urgent]\n---\n\nbody\n",
        );
        assert_eq!(task.tags(), ["DUPLICATE", "urgent"]);
        assert!(task.has_tag("DUPLICATE"));
        assert!(
            !task.has_tag("duplicate"),
            "tag matching is exact, not case-insensitive"
        );
        assert!(!task.has_tag("missing"));
    }

    #[test]
    fn body_is_everything_below_the_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let task = ranked(
            dir.path(),
            "001-a.md",
            "---\nid: \"001\"\ntitle: \"A\"\n---\n\n## Notes\n\nwords\n",
        );
        assert_eq!(task.body().unwrap(), "\n## Notes\n\nwords\n");

        task.set_body(task.hash().unwrap(), "\n## Notes\n\nrewritten\n")
            .unwrap();
        let text = fs::read_to_string(&task.path).unwrap();
        assert!(
            text.starts_with("---\nid: \"001\"\ntitle: \"A\"\n---\n"),
            "{text}"
        );
        assert!(text.ends_with("rewritten\n"), "{text}");
        // Editing the body must not disturb the frontmatter.
        assert_eq!(task.title(), "A");
    }

    #[test]
    fn a_file_without_a_closing_fence_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let task = ranked(dir.path(), "001-a.md", "---\nid: \"001\"\nno fence here\n");
        assert!(task.set_body(task.hash().unwrap(), "new").is_err());
    }

    #[test]
    fn hash_changes_when_content_changes_and_not_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        let task = ranked(
            dir.path(),
            "001-a.md",
            "---\nid: \"001\"\ntitle: \"A\"\n---\n\nbody\n",
        );
        let h1 = task.hash().unwrap();
        assert_eq!(task.hash().unwrap(), h1, "reading twice must not change it");

        task.set_title(h1, "B").unwrap();
        let h2 = task.hash().unwrap();
        assert_ne!(h2, h1, "the write must change the fingerprint");
    }

    /// docs/adr/0007-concurrency-and-locking.md's actual motivating scenario:
    /// a caller reads a ticket, something else changes it before the caller
    /// writes back, and the write must refuse rather than silently
    /// discarding the intervening change.
    #[test]
    fn a_write_against_a_stale_hash_is_refused_and_the_other_change_survives() {
        let dir = tempfile::tempdir().unwrap();
        let task = ranked(
            dir.path(),
            "001-a.md",
            "---\nid: \"001\"\ntitle: \"Original\"\nassignee: \"\"\n---\n\nbody\n",
        );
        let stale = task.hash().unwrap();

        // Someone else's write, after the hash was captured.
        task.assign(task.hash().unwrap(), "sam").unwrap();

        // The first caller, still holding the pre-assignment hash, tries to
        // write based on what it read before sam's change landed.
        let err = task.set_title(stale, "Renamed").unwrap_err();
        assert!(
            err.downcast_ref::<Conflict>().is_some(),
            "must be the typed Conflict error, not a generic one: {err}"
        );

        // sam's change is still there — not overwritten, not merged away.
        assert_eq!(task.assignee(), Some("sam".to_string()));
        assert_eq!(task.title(), "Original", "the stale write must not apply");
    }

    #[test]
    fn a_write_against_the_current_hash_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let task = ranked(
            dir.path(),
            "001-a.md",
            "---\nid: \"001\"\ntitle: \"Original\"\n---\n\nbody\n",
        );
        task.set_title(task.hash().unwrap(), "Renamed").unwrap();
        assert_eq!(task.title(), "Renamed");
    }

    #[test]
    fn apply_edit_writes_every_field_in_one_pass() {
        let dir = tempfile::tempdir().unwrap();
        let task = ranked(
            dir.path(),
            "001-a.md",
            "---\nid: \"001\"\ntitle: \"Old\"\nassignee: \"\"\npoints: \"\"\n---\n\nold body\n",
        );
        task.apply_edit(task.hash().unwrap(), "New", "sam", "5", "new body\n")
            .unwrap();
        assert_eq!(task.title(), "New");
        assert_eq!(task.assignee(), Some("sam".to_string()));
        assert_eq!(task.points(), Some("5".to_string()));
        assert_eq!(task.body().unwrap(), "new body\n");
    }

    /// The bug a naive four-separate-writes save would have: the hash
    /// captured once when a form opens can only ever match the file once,
    /// since the first write already changes it. apply_edit exists so this
    /// never happens — proved directly by showing the sequential version
    /// really would fail on its second call against the same hash.
    #[test]
    fn a_second_sequential_write_against_the_same_captured_hash_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let task = ranked(
            dir.path(),
            "001-a.md",
            "---\nid: \"001\"\ntitle: \"Old\"\nassignee: \"\"\n---\n\nbody\n",
        );
        let opened_at = task.hash().unwrap();
        task.set_title(opened_at, "New").unwrap();
        let err = task.assign(opened_at, "sam").unwrap_err();
        assert!(err.downcast_ref::<Conflict>().is_some(), "{err}");
    }

    #[test]
    fn set_title_changes_the_frontmatter_not_the_filename() {
        let dir = tempfile::tempdir().unwrap();
        let task = ranked(
            dir.path(),
            "001-old-slug.md",
            "---\nid: \"001\"\ntitle: \"Old\"\n---\n\nbody\n",
        );
        task.set_title(task.hash().unwrap(), "Brand new title")
            .unwrap();
        assert_eq!(task.title(), "Brand new title");
        // The slug is how history follows the file; renaming for a retitle
        // would break `git log --follow` for no gain.
        assert_eq!(task.stem, "001-old-slug");
        assert!(task.path.exists());
    }

    #[test]
    fn order_reads_a_rank_and_survives_a_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let task = ranked(
            dir.path(),
            "001-a.md",
            "---\nid: \"001\"\norder: \"1.5\"\n---\n\n## Notes\n",
        );
        assert_eq!(task.order(), Some(1.5));

        task.set_order(task.hash().unwrap(), 0.75).unwrap();
        assert_eq!(task.order(), Some(0.75));
        let text = fs::read_to_string(&task.path).unwrap();
        assert!(text.ends_with("## Notes\n"), "{text}");
        assert_eq!(text.matches("order").count(), 1, "{text}");
    }

    #[test]
    fn an_unranked_task_has_no_order() {
        let dir = tempfile::tempdir().unwrap();
        // Missing, empty and unparseable all mean "not ranked" rather than
        // zero — a rank of 0 would sort a task to the front of its column.
        for body in [
            "---\nid: \"001\"\n---\n",
            "---\nid: \"001\"\norder: \"\"\n---\n",
            "---\nid: \"001\"\norder: \"soon\"\n---\n",
        ] {
            let task = ranked(dir.path(), "001-a.md", body);
            assert_eq!(task.order(), None, "{body}");
        }
    }

    #[test]
    fn set_order_adds_the_key_when_the_task_predates_ordering() {
        let dir = tempfile::tempdir().unwrap();
        let task = ranked(dir.path(), "001-a.md", "---\nid: \"001\"\n---\n\nbody\n");
        task.set_order(task.hash().unwrap(), -1.0).unwrap();
        assert_eq!(task.order(), Some(-1.0));
        assert!(
            fs::read_to_string(&task.path).unwrap().ends_with("body\n"),
            "the body must survive"
        );
    }

    #[test]
    fn set_meta_replaces_an_existing_key() {
        let text = "---\nid: \"001\"\nrelated: []\n---\n\n## Notes\n";
        let updated = set_meta(text, "related", "[003]").unwrap();
        assert!(updated.contains("\nrelated: [003]\n"), "{updated}");
        assert!(updated.ends_with("## Notes\n"), "{updated}");
        assert_eq!(updated.matches("related").count(), 1, "{updated}");
    }

    #[test]
    fn set_meta_adds_a_missing_key_before_the_fence() {
        let text = "---\nid: \"001\"\n---\n\nbody\n";
        let updated = set_meta(text, "related", "[003]").unwrap();
        assert_eq!(updated, "---\nid: \"001\"\nrelated: [003]\n---\n\nbody\n");
    }

    #[test]
    fn set_meta_leaves_the_body_alone() {
        let text = "---\nid: \"001\"\n---\n\nrelated: not frontmatter\n";
        let updated = set_meta(text, "related", "[003]").unwrap();
        assert!(
            updated.contains("\nrelated: not frontmatter\n"),
            "{updated}"
        );
    }

    #[test]
    fn set_meta_needs_frontmatter() {
        assert!(set_meta("no frontmatter here\n", "related", "[1]").is_err());
        assert!(set_meta("---\nid: \"1\"\n", "related", "[1]").is_err());
    }

    #[test]
    fn unquote_strips_matching_quotes_only() {
        assert_eq!(unquote("\"First task\""), "First task");
        assert_eq!(unquote("'First task'"), "First task");
        assert_eq!(unquote("[x,y]"), "[x,y]");
        assert_eq!(unquote("\""), "\"");
    }
}
