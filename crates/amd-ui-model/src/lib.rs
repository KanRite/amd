//! Types and rules shared by the Tauri backend and the Leptos frontend.
//!
//! Everything here is plain data plus pure functions, so it compiles both
//! natively and to `wasm32-unknown-unknown`.

use serde::{Deserialize, Serialize};

pub mod appearance;
pub mod epic;
pub mod rich;
pub mod ticket;
pub mod wip;
pub mod zoom;

/// A board column. Each one is a directory under `tasks/`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Column {
    Backlog,
    Todo,
    Doing,
    Done,
}

impl Column {
    /// Every column, in board order.
    pub const ALL: [Column; 4] = [Column::Backlog, Column::Todo, Column::Doing, Column::Done];

    /// The directory name under `tasks/`.
    pub fn dir_name(self) -> &'static str {
        match self {
            Column::Backlog => "backlog",
            Column::Todo => "todo",
            Column::Doing => "doing",
            Column::Done => "done",
        }
    }

    /// The heading shown on the board.
    pub fn title(self) -> &'static str {
        match self {
            Column::Backlog => "Backlog",
            Column::Todo => "To do",
            Column::Doing => "Doing",
            Column::Done => "Done",
        }
    }

    /// Looks a column up by its directory name.
    pub fn from_dir_name(name: &str) -> Option<Column> {
        Column::ALL.into_iter().find(|c| c.dir_name() == name)
    }

    /// Position on the board, left to right.
    pub fn index(self) -> usize {
        self as usize
    }

    /// Only the outer columns can be collapsed out of the way.
    pub fn is_collapsible(self) -> bool {
        matches!(self, Column::Backlog | Column::Done)
    }

    /// A ticket moves one column at a time, forwards or back.
    pub fn can_move_to(self, to: Column) -> bool {
        self.index().abs_diff(to.index()) == 1
    }
}

/// What a card on the board shows.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TicketSummary {
    /// The repository whose board it is on.
    #[serde(default)]
    pub repo: String,
    /// The repository it's about (tickets 101/107), a bare name; empty if
    /// it doesn't say. What the repository filter matches.
    #[serde(default)]
    pub repository: String,
    /// File name within its column directory, e.g. `001-some-title.md`.
    pub file: String,
    pub column: Column,
    pub id: String,
    pub title: String,
    pub assignee: String,
    pub tags: Vec<String>,
    /// Empty for tickets saved before priorities existed.
    pub priority: String,
    /// `YYYY-MM-DD`; empty if the ticket does not say.
    pub created: String,
    /// The epic it is filed under (backlog only), by directory.
    #[serde(default)]
    pub epic: Option<String>,
    /// The ticket's body (everything after the frontmatter), for searching.
    #[serde(default)]
    pub text: String,
    /// The epic it belongs to by its `epic:` label, kept after it leaves the
    /// epic's folder (where `epic` above says it is filed). Empty if none.
    #[serde(default)]
    pub epic_label: String,
    /// Its size estimate (`amd`'s `points:`), empty if unsized.
    #[serde(default)]
    pub points: String,
    /// Its lock, while one is in force.
    #[serde(default)]
    pub lock: Option<Lock>,
    /// A lease still recorded but past its time (ticket 110). It blocks
    /// nothing; it's shown so a lapsed lock isn't mistaken for no lock.
    #[serde(default)]
    pub lapsed: Option<Lock>,
    /// The ticket it sits under (`parent:`), as written; empty if none.
    #[serde(default)]
    pub parent: String,
    /// The tickets it's related to (`related:`), as written.
    #[serde(default)]
    pub related: Vec<String>,
}

/// The ticket number a `parent`/`related` entry names: `042`, `"042"` or
/// `[[042-some-title]]` all give 42.
pub fn ticket_number(reference: &str) -> Option<u32> {
    let digits: String = reference
        .trim()
        .trim_matches(|c| c == '"' || c == '[')
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// A lock (lease) on a ticket that is in force: who holds it and until when.
/// While it holds, nobody else can edit, move or archive the ticket.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Lock {
    /// The holder's git identity (`user.email`).
    pub by: String,
    /// When it lapses, as RFC 3339 UTC (`locked-until`).
    pub until: String,
    /// The same moment in local time, for display.
    pub until_local: String,
    /// Whether the person using the board holds it.
    pub mine: bool,
}

impl TicketSummary {
    /// Whether the ticket matches a search: every word of `query` must
    /// appear, ignoring case, somewhere in its id, title, file name, body,
    /// tags, assignee, priority or epic. An empty query matches everything.
    pub fn matches(&self, query: &str) -> bool {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return true;
        }
        let haystack = [
            self.id.as_str(),
            self.title.as_str(),
            self.file.as_str(),
            self.text.as_str(),
            self.assignee.as_str(),
            self.priority.as_str(),
            self.epic.as_deref().unwrap_or(""),
            self.epic_label.as_str(),
        ]
        .into_iter()
        .chain(self.tags.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
        words.iter().all(|w| haystack.contains(w.as_str()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardColumn {
    pub column: Column,
    pub tickets: Vec<TicketSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Board {
    /// The project's name (`project` in its `.settings`), shown in the
    /// header; the repository's name when the board doesn't declare one.
    #[serde(default)]
    pub project: String,
    /// Absolute path of the `tasks/` directory being shown.
    pub tasks_dir: String,
    /// Every repository the project spans, the board's own first. Tickets
    /// and epics from all available ones are merged into the lists below.
    #[serde(default)]
    pub repos: Vec<RepoInfo>,
    /// The columns' loose tickets; tickets inside an epic are under `epics`.
    pub columns: Vec<BoardColumn>,
    /// The backlog's epics, by name.
    #[serde(default)]
    pub epics: Vec<Epic>,
    /// Every project in the folder this one sits in (ticket 093), for the
    /// header's switcher. Just this one when it's alone there.
    #[serde(default)]
    pub projects: Vec<ProjectChoice>,
    /// The repositories tickets can be about, for the filter and the
    /// Repository fields: the project's declared ones plus any a ticket
    /// already names, sorted.
    #[serde(default)]
    pub ticket_repos: Vec<RepoChoice>,
    /// The project's users' names, for the assignee picker (ticket 089).
    #[serde(default)]
    pub users: Vec<String>,
    /// Who's using the board, by git `user.name`: the picker's "Me".
    #[serde(default)]
    pub me: Option<String>,
    /// The project's tags (ticket 122): every listed tag, and every tag in
    /// use on its boards, each with how many tickets carry it. Sorted;
    /// always UPPERCASE.
    #[serde(default)]
    pub tags: Vec<TagUse>,
    /// Only listed tags may be put on a ticket (the project's `prevent-arbitrary-tags`, formerly `tags-only`).
    #[serde(default)]
    pub tags_only: bool,
    /// Whether this person gets Admin (ticket 149), and why.
    #[serde(default)]
    pub admin: AdminAccess,
    /// The documents repository every project shares (ticket 129); empty
    /// when the project names none.
    #[serde(default)]
    pub shared_documents: String,
    /// The priority an imported GitHub issue starts at (ticket 158).
    #[serde(default)]
    pub triage_priority: String,
    /// Whether the landing page pulls in this project's GitHub issues
    /// (ticket 173).
    #[serde(default)]
    pub import_issues: bool,
}

/// Whether to show Admin, and the note that says why (ticket 149).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminAccess {
    pub allowed: bool,
    pub note: String,
}

impl Default for AdminAccess {
    /// Open, as before ticket 149: never lock someone out on a guess.
    fn default() -> Self {
        AdminAccess {
            allowed: true,
            note: String::new(),
        }
    }
}

/// One of a project's tags and how much it's used.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagUse {
    pub name: String,
    /// Tickets carrying it, across the project's boards (not the archive).
    pub count: usize,
    /// On the project's tag list (Admin), as opposed to only in use.
    pub listed: bool,
    /// Built into the tool (DRAFT, ticket 114; TRIAGE, 158): always offered and allowed,
    /// never on the project's list, never removable.
    #[serde(default)]
    pub reserved: bool,
}

/// A repository tickets can be about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoChoice {
    pub name: String,
    /// Cloned on this machine. One that isn't can't be chosen for a ticket
    /// (tickets 101/107) and is shown greyed out.
    pub available: bool,
}

/// A `repository` value as a bare name: `agile-md/amd`, `amd.git` and `amd`
/// all name `amd`. Must match `agile_md::task::repo_name`.
pub fn repo_name(value: &str) -> String {
    let value = value.trim().trim_end_matches('/');
    let value = value.strip_suffix(".git").unwrap_or(value);
    value
        .rsplit(['/', ':'])
        .next()
        .unwrap_or_default()
        .to_string()
}

/// A project the header can switch to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectChoice {
    pub name: String,
    /// Whether it's the one on screen.
    pub current: bool,
}

/// The user filter: whose work to show.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Assigned {
    /// Everyone's tickets ("All").
    #[default]
    Anyone,
    /// Only tickets assigned to this user.
    User(String),
    /// Only tickets nobody is assigned to.
    Nobody,
}

impl Assigned {
    pub fn matches(&self, ticket: &TicketSummary) -> bool {
        let assignee = ticket.assignee.trim();
        match self {
            Assigned::Anyone => true,
            Assigned::User(name) => assignee == name,
            Assigned::Nobody => assignee.is_empty(),
        }
    }

    /// The dropdown's `value` for this choice. `""` is All; a user's own
    /// name; a value no assignee can have (it starts with a space) for
    /// unassigned.
    pub fn key(&self) -> String {
        match self {
            Assigned::Anyone => String::new(),
            Assigned::User(name) => name.clone(),
            Assigned::Nobody => " unassigned".into(),
        }
    }

    pub fn from_key(key: &str) -> Assigned {
        match key {
            "" => Assigned::Anyone,
            " unassigned" => Assigned::Nobody,
            name => Assigned::User(name.to_string()),
        }
    }
}

impl Board {
    /// Everyone with a ticket assigned anywhere on the board, in alphabetical
    /// order (ignoring case), for the user filter.
    pub fn assignees(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .columns
            .iter()
            .flat_map(|c| c.tickets.iter())
            .chain(self.epics.iter().flat_map(|e| e.tickets.iter()))
            .map(|t| t.assignee.trim().to_string())
            .filter(|a| !a.is_empty())
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        names
    }
}

/// A repository the project spans (`amd register`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoInfo {
    pub name: String,
    /// Cloned on this machine with a `tasks/` board; otherwise only declared.
    pub available: bool,
    /// Its board's WIP limit (on doing).
    #[serde(default)]
    pub wip: wip::WipLimits,
    /// How long a lock lasts on its board, in hours.
    #[serde(default = "default_lease_hours")]
    pub lease_hours: u32,
}

/// How long a lock lasts unless a board says otherwise: a working day.
pub const DEFAULT_LEASE_HOURS: u32 = 8;

fn default_lease_hours() -> u32 {
    DEFAULT_LEASE_HOURS
}

/// A backlog container — an epic or a sprint — and the tickets filed in it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Epic {
    /// The repository whose backlog it is in.
    #[serde(default)]
    pub repo: String,
    pub name: String,
    #[serde(default)]
    pub kind: epic::GroupKind,
    /// A sprint's length; 0 for an epic.
    #[serde(default)]
    pub days: u32,
    /// Whether a sprint has been started (epics never are).
    #[serde(default)]
    pub started: bool,
    pub tickets: Vec<TicketSummary>,
}

/// A ticket's full markdown, as opened in the editor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketDocument {
    #[serde(default)]
    pub repo: String,
    pub file: String,
    pub column: Column,
    /// Set when the ticket lives inside an epic in the backlog.
    #[serde(default)]
    pub epic: Option<String>,
    pub content: String,
    /// Its lock, while one is in force.
    #[serde(default)]
    pub lock: Option<Lock>,
    /// A lease still recorded but past its time (ticket 110). It blocks
    /// nothing; it's shown so a lapsed lock isn't mistaken for no lock.
    #[serde(default)]
    pub lapsed: Option<Lock>,
    /// A fingerprint of `content` as it was opened. Saving hands it back,
    /// and the save is refused if the ticket has changed since.
    #[serde(default)]
    pub version: String,
    /// "created by … on …; last edited by … on …" (ticket 155), or empty.
    #[serde(default)]
    pub authorship: String,
}

/// What happened in git after a ticket change was written to disk.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncReport {
    /// The commit message used, e.g. `edit: 004 "title"`.
    pub message: String,
    pub committed: bool,
    pub pushed: bool,
    /// Why it was not committed or pushed, when it should have been.
    pub warning: Option<String>,
}

/// What importing GitHub issues did, or would do (ticket 158).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueImport {
    /// One line per issue: `#12 -> 160 "Title" (owner/repo) [TRIAGE, BUG]`.
    pub lines: Vec<String>,
    /// `imported 2 issues, 1 already imported`, or the preview's version.
    pub summary: String,
    /// The commit, after a real import that brought something in.
    pub sync: Option<SyncReport>,
}

/// A project's documents repository (ticket 129): its own, or the one every
/// project shares.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocSource {
    /// The repository's name.
    pub name: String,
    /// The one every project shares, not the project's own.
    pub shared: bool,
    /// Whether it's cloned on this machine; when not, `documents` is empty.
    pub cloned: bool,
    pub documents: Vec<DocEntry>,
    /// Every folder, empty ones included (ticket 176).
    #[serde(default)]
    pub folders: Vec<String>,
}

/// One document in a [`DocSource`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocEntry {
    /// Relative to the repository, `/` between parts.
    pub path: String,
    pub title: String,
    /// Marked `status: archived` (ticket 176).
    #[serde(default)]
    pub archived: bool,
}

/// A document, rendered for reading.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedDoc {
    /// The repository it's in.
    pub source: String,
    pub path: String,
    /// HTML made by amdui from the markdown: raw HTML in the document is
    /// shown as text, a link to another document is `#doc:<path>`, a link
    /// amd won't follow is `#`, and images are inlined as data.
    pub html: String,
    /// Set when this is an earlier version of the document, not the
    /// current one (ticket 169).
    #[serde(default)]
    pub version: Option<DocVersion>,
    /// Marked `status: archived` (ticket 176).
    #[serde(default)]
    pub archived: bool,
}

/// A document opened for editing (ticket 176).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocEdit {
    pub source: String,
    pub path: String,
    /// Its frontmatter block, fences included, kept as it is; empty for
    /// none.
    pub frontmatter: String,
    /// The markdown below it: what's edited.
    pub body: String,
    /// A fingerprint of the whole text as opened; saving hands it back.
    pub version: String,
    /// What the rich editor can't keep (tables, images…): when not empty,
    /// it's edited as Markdown only.
    pub unsupported: Vec<String>,
}

/// A change to a documents repository, and where its document now is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocChange {
    pub sync: SyncReport,
    /// The document's path after the change (moved or created), if any.
    pub path: Option<String>,
}

/// One version of a document, from git (ticket 169).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocVersion {
    pub commit: String,
    pub short: String,
    pub author: String,
    /// `YYYY-MM-DD HH:MM`.
    pub date: String,
    pub subject: String,
    /// The document's path at that commit.
    pub path: String,
}

/// The landing page (ticket 020): your work across every project on this
/// machine, and the projects themselves.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Home {
    /// Your todo and doing tickets, highest priority first, then oldest.
    pub work: Vec<WorkItem>,
    /// When `work` is empty, backlog tickets you could pick (ticket 160):
    /// the highest priority there is any of, oldest first.
    #[serde(default)]
    pub to_pick: Vec<WorkItem>,
    /// Whether work to pick includes unassigned tickets (My work's
    /// checkbox, remembered on this machine).
    #[serde(default)]
    pub include_unassigned: bool,
    /// Every project on this machine, by name; hidden ones marked.
    pub projects: Vec<HomeProject>,
    /// Who "you" is (git's `user.email`); empty when git doesn't say.
    pub me: String,
    /// The project amdui has open, and remembers for next time, by its
    /// board repository's path (`HomeProject.root`); empty when none
    /// (ticket 168: the top bar's picker and documents buttons act on it).
    #[serde(default)]
    pub current: String,
}

/// A project on the landing page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeProject {
    pub name: String,
    /// Its board's repository: what opening it opens.
    pub root: String,
    /// The folder holding its repositories, to tell two clones apart.
    pub dir: String,
    /// Left out of My work by you.
    pub hidden: bool,
    /// Whether its own documents repository is cloned here (ticket 168):
    /// Documents, ADRs and Ideas are greyed out when not.
    #[serde(default)]
    pub documents: bool,
    /// The shared documents repository it names, if any.
    #[serde(default)]
    pub shared: Option<String>,
    /// Whether that shared repository is cloned here.
    #[serde(default)]
    pub shared_cloned: bool,
}

/// Which documents the Documents view shows (ticket 168): the project's
/// own repository or the shared one, and a folder in it (`docs`, `adr`,
/// `ideas`), or everything when empty.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocScope {
    pub shared: bool,
    pub folder: String,
}

impl DocScope {
    /// Whether `path` (relative to the repository) is inside the folder.
    pub fn holds(&self, path: &str) -> bool {
        self.folder.is_empty()
            || path
                .strip_prefix(self.folder.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    }
}

/// One ticket in My work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItem {
    pub project: String,
    /// The project's board repository, opened to show the ticket.
    pub project_root: String,
    /// The repository whose board the ticket is on.
    pub repo: String,
    pub column: Column,
    pub id: String,
    /// File name, `NNN-slug.md`.
    pub file: String,
    pub title: String,
    pub priority: String,
    pub points: String,
    pub created: String,
    /// The backlog folder (epic or sprint) it's filed in, to find it there.
    #[serde(default)]
    pub folder: Option<String>,
}

/// A newly created ticket and what happened when it was recorded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Created {
    pub doc: TicketDocument,
    pub sync: SyncReport,
}

/// The template new tickets are rendered from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketTemplate {
    /// Where an overriding template lives (or would live) on disk.
    pub path: String,
    pub source: String,
    /// `true` when `source` comes from the board's own override file rather
    /// than the built-in default.
    pub customised: bool,
}

/// The frontmatter fields the board needs from a ticket.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frontmatter {
    pub id: String,
    pub title: String,
    pub assignee: String,
    /// The repository it's about, as a bare name.
    pub repository: String,
    pub tags: Vec<String>,
    pub branch_type: String,
    pub priority: String,
    /// `YYYY-MM-DD`, as `amd` writes it.
    pub created: String,
    /// The epic the ticket belongs to, if any (`amd`'s `epic:` field).
    pub epic: String,
    /// Its size estimate (`amd`'s `points:` field).
    pub points: String,
    /// Who holds its lock (`locked-by`), and until when (`locked-until`).
    pub locked_by: String,
    pub locked_until: String,
}

/// Reads the handful of fields the board displays from a ticket's
/// `---`-delimited frontmatter.
///
/// This is deliberately not a YAML parser: tickets are edited as raw text, so
/// the board only needs to *read* a few flat `key: value` lines, and anything
/// it cannot read simply shows as empty.
pub fn parse_frontmatter(content: &str) -> Frontmatter {
    let mut fm = Frontmatter::default();
    let mut lines = content.lines();
    if lines.next().map(str::trim) != Some("---") {
        return fm;
    }
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "id" => fm.id = unquote(value),
            "title" => fm.title = unquote(value),
            "assignee" => fm.assignee = unquote(value),
            "repository" => fm.repository = repo_name(&unquote(value)),
            "tags" => fm.tags = parse_list(value),
            "branch-type" => fm.branch_type = unquote(value),
            "priority" => fm.priority = unquote(value),
            "created" => fm.created = unquote(value),
            "epic" => fm.epic = unquote(value),
            "points" => fm.points = unquote(value),
            "locked-by" => fm.locked_by = unquote(value),
            "locked-until" => fm.locked_until = unquote(value),
            _ => {}
        }
    }
    fm
}

fn unquote(value: &str) -> String {
    let v = value.trim();
    let quoted = v.len() >= 2
        && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\'')));
    if quoted {
        v[1..v.len() - 1].replace("\\\"", "\"")
    } else {
        v.to_string()
    }
}

fn parse_list(value: &str) -> Vec<String> {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(unquote)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Turns a title into the file-name slug `amd` uses — the same rules as
/// `agile_md::task::slugify` (letters from other scripts transliterated,
/// symbols as separators, at most 50 characters), so a branch name the
/// editor derives matches the one `amd` would. `amd-ui` tests the two agree.
pub fn slugify(title: &str) -> String {
    const MAX_SLUG: usize = 50;
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
    fn moves_only_to_adjacent_columns() {
        assert!(Column::Backlog.can_move_to(Column::Todo));
        assert!(Column::Todo.can_move_to(Column::Backlog));
        assert!(Column::Doing.can_move_to(Column::Done));
        assert!(!Column::Backlog.can_move_to(Column::Doing));
        assert!(!Column::Backlog.can_move_to(Column::Done));
        assert!(!Column::Todo.can_move_to(Column::Todo));
    }

    #[test]
    fn only_outer_columns_collapse() {
        let collapsible: Vec<_> = Column::ALL
            .into_iter()
            .filter(|c| c.is_collapsible())
            .collect();
        assert_eq!(collapsible, [Column::Backlog, Column::Done]);
    }

    #[test]
    fn column_dir_names_round_trip() {
        for c in Column::ALL {
            assert_eq!(Column::from_dir_name(c.dir_name()), Some(c));
        }
        assert_eq!(Column::from_dir_name("archive"), None);
    }

    #[test]
    fn reads_amd_frontmatter() {
        let ticket = "---\nid: \"001\"\ntitle: \"create a Tauri application board\"\n\
                      assignee: \"claude\"\nrelated: []\ntags: [board, ui]\n---\n\n## Notes\n";
        let fm = parse_frontmatter(ticket);
        assert_eq!(fm.id, "001");
        assert_eq!(fm.title, "create a Tauri application board");
        assert_eq!(fm.assignee, "claude");
        assert_eq!(fm.tags, ["board", "ui"]);
    }

    #[test]
    fn title_may_contain_a_colon() {
        let fm = parse_frontmatter("---\ntitle: \"Hello world: test\"\n---\n");
        assert_eq!(fm.title, "Hello world: test");
    }

    #[test]
    fn missing_frontmatter_is_empty() {
        assert_eq!(
            parse_frontmatter("# just a heading\n"),
            Frontmatter::default()
        );
        assert_eq!(
            parse_frontmatter("---\ntags: []\n---\n").tags,
            Vec::<String>::new()
        );
    }

    #[test]
    fn slugs_match_amd() {
        assert_eq!(slugify("Hello world: test"), "hello-world-test");
        assert_eq!(
            slugify("create a Tauri application board"),
            "create-a-tauri-application-board"
        );
        assert_eq!(slugify("  --Trim me!!  "), "trim-me");
    }
}

#[cfg(test)]
mod assigned_tests {
    use super::*;

    fn ticket(assignee: &str) -> TicketSummary {
        TicketSummary {
            repo: String::new(),
            file: "001-t.md".into(),
            column: Column::Todo,
            id: "001".into(),
            title: "T".into(),
            assignee: assignee.into(),
            tags: Vec::new(),
            priority: String::new(),
            created: String::new(),
            epic: None,
            text: String::new(),
            epic_label: String::new(),
            points: String::new(),
            repository: String::new(),
            lock: None,
            lapsed: None,
            parent: String::new(),
            related: Vec::new(),
        }
    }

    #[test]
    fn a_user_sees_only_their_tickets_and_all_sees_everyone() {
        let mine = ticket("mrdoodles");
        let theirs = ticket("claude");
        let nobody = ticket("");
        let me = Assigned::User("mrdoodles".into());
        assert!(me.matches(&mine) && !me.matches(&theirs) && !me.matches(&nobody));
        assert!(
            [&mine, &theirs, &nobody]
                .iter()
                .all(|t| Assigned::Anyone.matches(t))
        );
        assert!(Assigned::Nobody.matches(&nobody) && !Assigned::Nobody.matches(&mine));
    }

    #[test]
    fn filter_keys_round_trip() {
        for a in [
            Assigned::Anyone,
            Assigned::Nobody,
            Assigned::User("claude".into()),
        ] {
            assert_eq!(Assigned::from_key(&a.key()), a);
        }
    }

    #[test]
    fn relationship_entries_name_a_ticket_number() {
        assert_eq!(ticket_number("042"), Some(42));
        assert_eq!(ticket_number("\"150\""), Some(150));
        assert_eq!(ticket_number("[[007-some-title]]"), Some(7));
        assert_eq!(ticket_number(""), None);
        assert_eq!(ticket_number("abc"), None);
    }

    #[test]
    fn assignees_are_listed_once_in_order() {
        let mut board = Board {
            project: String::new(),
            tasks_dir: String::new(),
            repos: Vec::new(),
            columns: vec![BoardColumn {
                column: Column::Todo,
                tickets: vec![
                    ticket("mrdoodles"),
                    ticket(""),
                    ticket("claude"),
                    ticket("mrdoodles"),
                ],
            }],
            epics: Vec::new(),
            projects: Vec::new(),
            ticket_repos: Vec::new(),
            users: Vec::new(),
            me: None,
            tags: Vec::new(),
            tags_only: false,
            admin: AdminAccess::default(),
            shared_documents: String::new(),
            triage_priority: String::new(),
            import_issues: true,
        };
        board.epics.push(Epic {
            repo: String::new(),
            name: "E".into(),
            kind: epic::GroupKind::Epic,
            days: 0,
            started: false,
            tickets: vec![ticket("Alice")],
        });
        assert_eq!(board.assignees(), ["Alice", "claude", "mrdoodles"]);
    }

    #[test]
    fn a_doc_scope_holds_its_folder_only() {
        let adr = DocScope {
            shared: false,
            folder: "adr".into(),
        };
        assert!(adr.holds("adr/adr/0001-x.md"));
        assert!(adr.holds("adr/rfc/0002-y.md"));
        assert!(!adr.holds("adrift.md"));
        assert!(!adr.holds("docs/adr.md"));
        assert!(DocScope::default().holds("anything/at/all.md"));
    }
}
