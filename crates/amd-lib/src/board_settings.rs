//! Board-local settings: facts about this board itself, not a user's
//! preferences. Lives at `<repo-root>/.settings` — a sibling of the board
//! directory, not inside it, the same way `README.md` sits at the repo root
//! rather than inside `tasks/`. Hidden, matching the legacy `.next-id` file
//! it partly replaces: this is tool-managed state, not something meant to
//! be browsed the way `README.md` is. **Git-tracked** — unlike the shared config
//! file (`settings.rs`) or the registry, both of which are per-machine. A
//! board-local value needs to travel with the repository to every clone,
//! which is exactly why it can't live in either of those.
//!
//! Three keys today:
//! - `project`: the declared identity `amd init <project>` records. It is
//!   immutable once set — a policy this module enforces on its own write
//!   path, not a filesystem lock. Hand-editing the file directly is not
//!   prevented here; `project_trust.rs` is what notices when that's
//!   happened.
//! - `next-id`: the board's id counter, moved here from the legacy
//!   `<board>/.next-id` file so the ticket columns hold nothing but tickets.
//!   Freely mutable — every new ticket advances it — unlike `project`.
//! - `lease-hours`: how long a taken ticket lease lasts before it lapses on
//!   its own (`docs/adr/0007-concurrency-and-locking.md`). Unlike the two
//!   above, nothing writes this programmatically — it's meant to be
//!   hand-edited, the same as `branch-types`/`editor` in the *global*
//!   settings file, just board-local rather than per-machine
//!   because a lease policy is a team decision, not a personal preference.
//! - `wip-todo`: **ignored** (ticket 083). Todo used to carry an optional
//!   limit; a limit there as well as on doing could leave work stuck with
//!   nowhere to go, so only doing is limited now. The key is still
//!   recognised so an old file
//!   keeps loading, but nothing reads it.
//! - `wip-limit`: `doing/`'s WIP limit — the only one. Absent or
//!   unparsable defaults to **2**, not unlimited — a long queue of tickets
//!   "in progress" at once is treated as the mistake, not the norm.
//!   Explicit `0` is what turns it off, and is stored literally rather than
//!   collapsed into "absent", precisely so the two states (nobody set it;
//!   somebody deliberately disabled it) stay distinguishable. `amd
//!   wip-limit <n>` is the CLI setter.
//! - `commit`: whether a mutating command commits (and pushes) its own
//!   change automatically — the audit trail for a dedicated, board-only
//!   `{project}-tickets` repository, which has nothing else in it for a
//!   change to ride in on (`agile-md-documents`'
//!   `adr/0006-auto-commit-for-dedicated-tickets-repos.md`). Absent or
//!   unparsable means **on** — the opposite default from every other
//!   setting here, deliberately: this feature's whole job is broken until
//!   it's active, so off-by-default would ship it in a state where the
//!   common case needs a setting found first. `AMD_COMMIT=0`/`off`/`false`
//!   turns it off; no programmatic setter, hand-edited like `lease-hours`,
//!   a team decision rather than a personal one.
//! - `repos`: the *declared* names of every repository this project spans
//!   — comma-separated, same shape the global settings file's
//!   `branch-types` already uses. Shared and git-tracked deliberately: one
//!   team member adding a repository means everyone sees it listed on their
//!   next pull. This is one half of registration; the other half —
//!   whether *this machine* actually has a declared name cloned, and
//!   where — is `registry.rs`'s job, per-machine and never shared. A name
//!   declared here with no matching local registry entry is what `amdui`
//!   shows greyed out.
//! - `users`: the people on this project (ticket 088), in git's own
//!   identity shape — `Name <email>, Name <email>`. `amd join` adds the
//!   joining user. Like every setting, it's on the direct-push path, so a
//!   join takes effect at once. The email
//!   is the identity (leases use `user.email`); the name is for display.
//!
//! One repository, one settings file — a repository with more than one
//! board (two different `AMD_DIR` trees) shares this file and its counter
//! between them, rather than each getting its own. Not a real scenario:
//! `AMD_DIR` exists so a repository can pick a non-default location for its
//! one board, not to run several boards side by side.
//!
//! **Every setting lives here** (ticket 111). Ticket 057 once moved
//! `project`/`repos`/`wip-limit`/`lease-hours` into `{project}-documents`'s
//! `.settings/` directory so changes to them would be PR-reviewed, but a
//! change there only reached the team once someone opened and merged a
//! documents PR, and the two copies drifted. Here, a setting changes the way
//! a ticket moves: committed and pushed, and synced (ticket 102).
//! `migrate_from_documents` carries anything still in a documents
//! `.settings/` across, once; that directory is never read otherwise.

use anyhow::{Result, bail};

use crate::board::{Board, Column};
use crate::settings::Config;

const PROJECT: &str = "project";
const NEXT_ID: &str = "next-id";
const LEASE_HOURS: &str = "lease-hours";
const DEFAULT_LEASE_HOURS: f64 = 8.0;
const WIP_TODO: &str = "wip-todo";
const WIP_LIMIT: &str = "wip-limit";
/// Doing's WIP limit when nobody has set one.
pub const DEFAULT_WIP_LIMIT: u32 = 2;
const REPOS: &str = "repos";
const AUTO_COMMIT: &str = "commit";
const USERS: &str = "users";
const SYNC: &str = "sync";
const TICKET_NUMBERING: &str = "ticket-numbering";
const TAGS: &str = "tags";
const TAGS_ONLY: &str = "tags-only";

/// One person on the project: git's own `Name <email>` identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub name: String,
    pub email: String,
}

impl User {
    /// Who git says you are in `repo`: `user.name <user.email>`, the
    /// identity your commits carry and leases record. `None` without an
    /// email (the identity key); a missing name falls back to the email.
    pub fn from_git(repo: &std::path::Path) -> Option<User> {
        let email = crate::git::config(repo, "user.email")?;
        let name = crate::git::config(repo, "user.name").unwrap_or_else(|| email.clone());
        Some(User { name, email })
    }

    /// Whether this is the same person as `other` — by email, ignoring case,
    /// the way leases identify people. A changed display name is still them.
    pub fn same_person(&self, other: &User) -> bool {
        self.email.eq_ignore_ascii_case(&other.email)
    }
}

impl std::fmt::Display for User {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} <{}>", self.name, self.email)
    }
}

/// Parses a `users` value. Split on `>` rather than `,`, so a name with a
/// comma in it ("Preece, Tim <t@example.com>") still parses; anything that
/// isn't a `Name <email>` entry is skipped rather than failing the load.
fn parse_users(value: &str) -> Vec<User> {
    value
        .split('>')
        .filter_map(|chunk| {
            let chunk = chunk.trim_start_matches(|c: char| c == ',' || c.is_whitespace());
            let (name, email) = chunk.split_once('<')?;
            let (name, email) = (name.trim(), email.trim());
            (!email.is_empty()).then(|| User {
                name: name.to_string(),
                email: email.to_string(),
            })
        })
        .collect()
}

/// Keys ticket 057 once kept in `{project}-documents/.settings/` (one file
/// per key). Ticket 111 brought every setting back into the tickets
/// repository's own `.settings` — the only place a change reaches the whole
/// team reliably, through the same commit and push as a ticket move — and
/// these are what `migrate_from_documents` carries over from a project that
/// still has documents settings.
const MIGRATED_KEYS: &[&str] = &[
    PROJECT,
    LEASE_HOURS,
    WIP_TODO,
    WIP_LIMIT,
    REPOS,
    TICKET_NUMBERING,
];

/// `<repo-root>/.settings` — a sibling of the board directory, not inside it.
pub fn path(board: &Board) -> std::path::PathBuf {
    board.repo_root().join(".settings")
}

/// Where `{project}-documents`'s old `.settings/` directory can be found for
/// this board, if at all — only ever read now, by `migrate_from_documents`. Sibling-path derivation first — the standing
/// `rfc/0003` layout, `{project}-tickets` next to `{project}-documents` —
/// so this works even on a machine that's never run `amd register`. Falls
/// back to a **project-scoped** registry lookup, `{project}-documents`,
/// using this board's own (possibly stale, pre-migration) locally recorded
/// `project` value — deliberately *not* a machine-wide `documents` alias
/// (aliases were removed in ticket 097): a machine can have more than one
/// project registered at once, and resolving to "whichever repo happens to
/// be aliased documents right now" would silently leak a *different* project's settings into this
/// one — confirmed live, the hard way, as a real `project_trust` violation
/// during this ticket's own testing. `None` means no documents repository
/// is locatable at all — not an error, just "keep using the local file",
/// which is also exactly what every board that predates the documents
/// split needs.
fn documents_root(board: &Board) -> Option<std::path::PathBuf> {
    let repo = board.repo_root();
    if let Some(name) = repo.file_name().and_then(|n| n.to_str())
        && let Some(prefix) = name.strip_suffix("-tickets")
        && let Some(parent) = repo.parent()
    {
        let sibling = parent.join(format!("{prefix}-documents"));
        if sibling.join(".settings").is_dir() {
            return Some(sibling);
        }
    }
    let local = Config::load_from(&path(board));
    let project = local.get(PROJECT)?;
    let declared = format!("{project}-documents");
    crate::registry::Registry::load()
        .find_near(&declared, &crate::registry::project_dir_of(repo))
        .map(|entry| entry.root.clone())
}

/// One key, one file, its entire trimmed content the value — simpler than
/// reusing `Config`'s `key = value` parser for a file that only ever holds
/// one key, and it reviews as an even smaller diff.
fn read_documents_key(documents_root: &std::path::Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(documents_root.join(".settings").join(key)).ok()?;
    let value = text.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[derive(Clone, Debug, Default)]
pub struct BoardSettings {
    /// Every setting, as read from `<tickets-repo>/.settings` (ticket 111).
    config: Config,
}

impl BoardSettings {
    pub fn project(&self) -> Option<&str> {
        self.config.get(PROJECT)
    }

    /// Sets the declared project identity. Setting it to the value it
    /// already has is a no-op — re-running `amd init <project>` against an
    /// already-initialized board (after moving it on disk, say) must not
    /// fail. Setting it to a *different* value is refused: the whole point
    /// of a declared identity is that it doesn't silently drift.
    pub fn set_project(&mut self, name: &str) -> Result<()> {
        validate_project_name(name)?;
        if let Some(existing) = self.project() {
            if existing == name {
                return Ok(());
            }
            bail!("this board's project is already '{existing}' and cannot become '{name}'");
        }
        self.config.set(PROJECT, name);
        Ok(())
    }

    /// The id the next ticket will take, if this file says anything
    /// sensible. `board.rs`'s `next_id()`/`record_id()` migrate the legacy
    /// `.next-id` file's value in here (and remove it) the first time this
    /// is `None` but that file exists, for boards that predate the move.
    pub fn next_id(&self) -> Option<u32> {
        self.config.get(NEXT_ID)?.trim().parse().ok()
    }

    /// Freely mutable, unlike `project` — every ticket recorded advances it.
    pub fn set_next_id(&mut self, id: u32) {
        self.config.set(NEXT_ID, &id.to_string());
    }

    /// How long a taken lease lasts before it lapses on its own —
    /// `AMD_LEASE_HOURS`, then this board's own `.settings`, then eight
    /// hours (a working day). An unparsable or non-positive value falls
    /// back to the default rather than producing a lease that expires
    /// before it's taken or never at all.
    pub fn lease_hours(&self) -> f64 {
        crate::settings::resolved("AMD_LEASE_HOURS", &self.config, LEASE_HOURS)
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|hours| *hours > 0.0)
            .unwrap_or(DEFAULT_LEASE_HOURS)
    }

    /// The most tickets `column` may hold at once. Only doing is ever
    /// limited (ticket 083): `AMD_WIP_LIMIT`, then this board's own
    /// `.settings` `wip-limit`, then [`DEFAULT_WIP_LIMIT`]. Explicit `0`
    /// means "off". Todo, backlog and done always read as unlimited — a
    /// limit on todo as well could leave work stuck with nowhere to go, so
    /// `wip-todo`/`AMD_WIP_TODO` are ignored.
    pub fn wip_limit(&self, column: Column) -> Option<u32> {
        match column {
            Column::Doing => {
                let parsed = crate::settings::resolved("AMD_WIP_LIMIT", &self.config, WIP_LIMIT)
                    .and_then(|value| value.trim().parse::<u32>().ok());
                match parsed {
                    Some(0) => None,
                    Some(limit) => Some(limit),
                    None => Some(DEFAULT_WIP_LIMIT),
                }
            }
            Column::Backlog | Column::Todo | Column::Done => None,
        }
    }

    /// Sets or clears doing's WIP limit in this board's own `.settings`.
    /// This never touches `AMD_WIP_LIMIT` — an env var set at all still
    /// wins over whatever this writes, same precedence `wip_limit` already
    /// documents.
    ///
    /// `0` is stored *literally* rather than collapsed into clearing:
    /// `wip_limit` treats an absent/unparsable value as the default of 2,
    /// not unlimited, so `Some(0)` and `None` have to reach the file as
    /// different states for the read side to tell them apart.
    pub fn set_wip_limit(&mut self, column: Column, limit: Option<u32>) -> Result<()> {
        match column {
            Column::Doing => {
                let value = limit.map(|l| l.to_string());
                self.config.set(WIP_LIMIT, value.as_deref().unwrap_or(""));
                Ok(())
            }
            Column::Todo => {
                bail!("todo has no WIP limit: only doing is limited, so work can't get stuck")
            }
            Column::Backlog | Column::Done => {
                bail!("{} has no WIP limit", column.as_str())
            }
        }
    }

    /// Whether a mutating command should commit (and push) its own change
    /// automatically. Opposite default from everything else in this file:
    /// absent or unparsable means **on**, since the feature's entire job is
    /// broken until it's active. `AMD_COMMIT` wins over `.settings` when
    /// set, same precedence every other setting here uses.
    pub fn auto_commit(&self) -> bool {
        crate::settings::resolved("AMD_COMMIT", &self.config, AUTO_COMMIT)
            .map(|value| {
                !matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "0" | "off" | "false"
                )
            })
            .unwrap_or(true)
    }

    /// Sets how long a lease lasts, in hours. Refuses a non-positive value,
    /// which [`lease_hours`](Self::lease_hours) would only read back as the
    /// default anyway.
    pub fn set_lease_hours(&mut self, hours: f64) -> Result<()> {
        if !(hours > 0.0 && hours.is_finite()) {
            bail!("a lease lasts a positive number of hours");
        }
        self.config.set(LEASE_HOURS, &hours.to_string());
        Ok(())
    }

    /// Turns auto-commit-and-push on or off for this board (`commit`).
    pub fn set_auto_commit(&mut self, on: bool) {
        self.config.set(AUTO_COMMIT, if on { "on" } else { "off" });
    }

    /// The declared repository names this project spans, in the order they
    /// were added. Whether any of them are actually cloned on this machine
    /// is `registry.rs`'s question, not this one.
    pub fn repos(&self) -> Vec<String> {
        self.config
            .get(REPOS)
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// Declares a repository name as part of this project. Idempotent —
    /// declaring a name that's already there changes nothing, the same way
    /// re-running `amd init <project>` against its own project is a no-op —
    /// registering something twice, locally or for the team, should never
    /// be an error.
    pub fn declare_repo(&mut self, name: &str) -> Result<()> {
        validate_project_name(name)?;
        let mut names = self.repos();
        if !names.iter().any(|existing| existing == name) {
            names.push(name.to_string());
            self.config.set(REPOS, &names.join(","));
        }
        Ok(())
    }

    /// Whether amd keeps this clone in step with its remote (ticket 102):
    /// `AMD_SYNC`, then this board's `sync` key, then **on**. `0`/`off`/
    /// `false` turns it off — the same shape as `commit`.
    pub fn sync(&self) -> bool {
        crate::settings::resolved("AMD_SYNC", &self.config, SYNC)
            .map(|value| {
                !matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "0" | "off" | "false"
                )
            })
            .unwrap_or(true)
    }

    /// Where new ticket numbers come from: `AMD_TICKET_NUMBERING`, then the
    /// project's `ticket-numbering` key, then `local`. A team decision, so it
    /// lives with the project's reviewed settings (see `numbering.rs`).
    pub fn ticket_numbering(&self) -> String {
        crate::settings::resolved("AMD_TICKET_NUMBERING", &self.config, TICKET_NUMBERING)
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "local".to_string())
    }

    /// The project's users, in the order they joined.
    pub fn users(&self) -> Vec<User> {
        self.config.get(USERS).map(parse_users).unwrap_or_default()
    }

    /// The users' names, once each, sorted ignoring case: what an assignee
    /// is picked from (ticket 089). An assignee is stored as a name.
    pub fn user_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.users().into_iter().map(|u| u.name).collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        names
    }

    /// Adds `user` unless they're already listed (by email). Returns whether
    /// anything changed, so a caller knows whether there's a commit to make —
    /// joining twice is not an error, the same as declaring a repo twice.
    pub fn add_user(&mut self, user: &User) -> Result<bool> {
        if user.email.is_empty() || user.email.contains(['<', '>', ',']) {
            bail!("'{}' isn't a usable email address", user.email);
        }
        if user.name.contains(['<', '>']) {
            bail!("a user's name can't contain '<' or '>'");
        }
        let mut users = self.users();
        if users.iter().any(|existing| existing.same_person(user)) {
            return Ok(false);
        }
        users.push(user.clone());
        let value = users
            .iter()
            .map(User::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        self.config.set(USERS, &value);
        Ok(true)
    }

    /// The project's tag list (ticket 122): uppercase, sorted. Empty when
    /// the project hasn't defined one, and then any tag goes.
    pub fn tags(&self) -> Vec<String> {
        let mut tags = crate::task::normalize_tags(&crate::task::parse_list(
            self.config.get(TAGS).unwrap_or(""),
        ));
        tags.sort();
        tags
    }

    /// Adds a tag to the list, in UPPERCASE, and returns it as stored.
    /// Refused if it isn't one word, or is already listed (ignoring case).
    pub fn add_tag(&mut self, tag: &str) -> Result<String> {
        let tag = crate::task::normalize_tag(tag);
        crate::task::validate_tag(&tag)?;
        if crate::task::is_reserved_tag(&tag) {
            bail!("{tag} is built in: every project has it");
        }
        let mut tags = self.tags();
        if tags.contains(&tag) {
            bail!("{tag} is already a tag");
        }
        tags.push(tag.clone());
        tags.sort();
        self.config.set(TAGS, &tags.join(", "));
        Ok(tag)
    }

    /// Takes a tag off the list (ignoring case). Returns whether it was
    /// there. Whether any ticket still carries it is the caller's check
    /// ([`crate::model::remove_unused_tag`]); this only edits the list.
    pub fn remove_tag(&mut self, tag: &str) -> bool {
        let tag = crate::task::normalize_tag(tag);
        let mut tags = self.tags();
        let before = tags.len();
        tags.retain(|known| *known != tag);
        if tags.len() == before {
            return false;
        }
        self.config.set(TAGS, &tags.join(", "));
        true
    }

    /// Whether only listed tags may be put on a ticket (`tags-only`), off
    /// unless set. Has no effect while the list is empty.
    pub fn only_listed_tags(&self) -> bool {
        self.config.get(TAGS_ONLY).is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "on" | "true"
            )
        })
    }

    pub fn set_only_listed_tags(&mut self, on: bool) {
        self.config.set(TAGS_ONLY, if on { "on" } else { "off" });
    }

    /// Checks a ticket's tags against the list: `Ok(None)` when they're all
    /// listed (or there's no list), `Ok(Some(warning))` naming the unlisted
    /// ones, or an error naming them when only listed tags are allowed.
    pub fn check_tags(&self, tags: &[String]) -> Result<Option<String>> {
        let listed = self.tags();
        if listed.is_empty() {
            return Ok(None);
        }
        let unlisted: Vec<String> = crate::task::normalize_tags(tags)
            .into_iter()
            .filter(|tag| !listed.contains(tag) && !crate::task::is_reserved_tag(tag))
            .collect();
        if unlisted.is_empty() {
            return Ok(None);
        }
        let names = unlisted.join(", ");
        if self.only_listed_tags() {
            bail!(
                "not on the project's tag list: {names} (only listed tags are allowed; add them in Admin or with `amd tags add`)"
            );
        }
        Ok(Some(format!("not on the project's tag list: {names}")))
    }

    /// Takes a repository off the project's declared list (`amd register
    /// --remove`). Returns whether it was there.
    pub fn undeclare_repo(&mut self, name: &str) -> bool {
        let mut names = self.repos();
        let before = names.len();
        names.retain(|existing| existing != name);
        if names.len() == before {
            return false;
        }
        self.config.set(REPOS, &names.join(","));
        true
    }

    /// Writes every setting to `<tickets-repo>/.settings` (ticket 111).
    pub fn save(&self, board: &Board) -> Result<()> {
        self.config.save_to(&path(board))
    }
}

/// Non-empty, and every character a letter, digit, hyphen, period, colon or
/// underscore — safe to use as (part of) a repository name at any forge.
/// Deliberately excludes `/`: unlike the others, a slash isn't just a
/// forge-compatibility question — `amd init` joins the project name
/// straight into a filesystem path and into `{owner}/{project}-tickets`, so
/// a slash would silently nest directories or produce a malformed owner/repo
/// pair rather than failing cleanly. Note this set is also broader than a
/// GitHub organisation or user *name* accepts (no periods, colons or
/// underscores there) — relevant because `amd init`, on GitHub, uses the
/// project name as the default repository owner too. A project name outside
/// that narrower set still validates here; it just needs `--org` to say
/// where the tickets repository actually goes.
/// The warning for assigning `who` when they aren't one of the project's
/// users (ticket 089): allowed — someone may not have joined yet — but
/// said. Nothing for nobody, a listed name, or a project with no users
/// list at all (a board made before ticket 088 has none).
pub fn unknown_assignee(board: &Board, who: &str) -> Option<String> {
    let who = who.trim();
    let names = load(board).user_names();
    (!who.is_empty() && !names.is_empty() && !names.iter().any(|n| n == who))
        .then(|| format!("{who} isn't one of this project's users — `amd join` adds them"))
}

pub fn validate_project_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("a project name can't be empty");
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':' | '_')))
    {
        bail!(
            "project name '{name}' contains '{bad}' — only letters, numbers, hyphens, \
             periods, colons and underscores are allowed"
        );
    }
    Ok(())
}

pub fn load(board: &Board) -> BoardSettings {
    BoardSettings {
        config: Config::load_from(&path(board)),
    }
}

/// Carries any settings still kept in `{project}-documents/.settings/`
/// (ticket 057's split) into the tickets `.settings` (ticket 111), so a
/// project that had them loses nothing. `repos` is the union of both; any
/// other key is taken from documents only where the tickets file doesn't
/// set it. Saves only when something changed, and returns the keys it
/// carried — empty once everything is across, so it's safe (and silent) to
/// call before every command. The documents files are never written, only
/// left behind to be deleted by a documents PR.
pub fn migrate_from_documents(board: &Board) -> Result<Vec<String>> {
    let Some(root) = documents_root(board) else {
        return Ok(Vec::new());
    };
    let mut settings = load(board);
    let mut moved = Vec::new();
    for key in MIGRATED_KEYS {
        let Some(value) = read_documents_key(&root, key) else {
            continue;
        };
        if *key == REPOS {
            let before = settings.repos();
            for name in value.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                settings.declare_repo(name)?;
            }
            if settings.repos() != before {
                moved.push(key.to_string());
            }
        } else if settings.config.get(key).is_none_or(|v| v.trim().is_empty()) {
            settings.config.set(key, &value);
            moved.push(key.to_string());
        }
    }
    if !moved.is_empty() {
        settings.save(board)?;
    }
    Ok(moved)
}

/// Where the old documents settings are, for a message after migrating.
pub fn documents_settings_dir(board: &Board) -> Option<std::path::PathBuf> {
    documents_root(board).map(|root| root.join(".settings"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(dir: &std::path::Path) -> Board {
        Board::at(dir.join("tasks"))
    }

    #[test]
    fn tags_are_listed_in_uppercase_without_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        assert!(settings.tags().is_empty());
        assert_eq!(settings.add_tag(" future ").unwrap(), "FUTURE");
        assert_eq!(settings.add_tag("business").unwrap(), "BUSINESS");
        assert!(
            settings.add_tag("Future").is_err(),
            "already listed, ignoring case"
        );
        assert!(settings.add_tag("two words").is_err());
        assert!(settings.add_tag("").is_err());
        settings.save(&board).unwrap();
        assert_eq!(load(&board).tags(), ["BUSINESS", "FUTURE"]);

        let mut settings = load(&board);
        assert!(settings.remove_tag("business"));
        assert!(!settings.remove_tag("business"));
        assert_eq!(settings.tags(), ["FUTURE"]);
    }

    #[test]
    fn draft_is_built_in_and_always_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = load(&board(dir.path()));
        assert!(settings.add_tag("draft").is_err(), "built in, not listed");
        settings.add_tag("FUTURE").unwrap();
        settings.set_only_listed_tags(true);
        let tags = vec!["draft".to_string(), "FUTURE".to_string()];
        assert_eq!(settings.check_tags(&tags).unwrap(), None);
    }

    #[test]
    fn unlisted_tags_warn_or_are_refused_when_only_listed_tags_are_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = load(&board(dir.path()));
        let tags = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            settings.check_tags(&tags(&["anything"])).unwrap(),
            None,
            "no list, any tag"
        );

        settings.add_tag("FUTURE").unwrap();
        assert_eq!(settings.check_tags(&tags(&["future"])).unwrap(), None);
        let warning = settings
            .check_tags(&tags(&["future", "misc"]))
            .unwrap()
            .unwrap();
        assert!(
            warning.contains("MISC") && !warning.contains("FUTURE"),
            "{warning}"
        );

        assert!(!settings.only_listed_tags());
        settings.set_only_listed_tags(true);
        let refused = settings
            .check_tags(&tags(&["misc"]))
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("MISC") && refused.contains("only listed tags"),
            "{refused}"
        );
        assert_eq!(settings.check_tags(&tags(&["FUTURE"])).unwrap(), None);
    }

    #[test]
    fn a_board_with_no_settings_file_has_no_project() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(&board(dir.path())).project(), None);
    }

    #[test]
    fn setting_the_project_survives_a_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.set_project("agile-md").unwrap();
        settings.save(&board).unwrap();

        assert_eq!(load(&board).project(), Some("agile-md"));
    }

    #[test]
    fn setting_the_same_project_twice_is_a_silent_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.set_project("agile-md").unwrap();
        settings.set_project("agile-md").unwrap();
        assert_eq!(settings.project(), Some("agile-md"));
    }

    #[test]
    fn setting_a_different_project_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.set_project("agile-md").unwrap();
        let err = settings.set_project("widgets").unwrap_err().to_string();
        assert!(err.contains("agile-md"), "{err}");
        assert!(err.contains("widgets"), "{err}");
        // The refused change must not have taken.
        assert_eq!(settings.project(), Some("agile-md"));
    }

    #[test]
    fn the_settings_file_sits_beside_the_board_not_inside_it() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        assert_eq!(path(&board), dir.path().join(".settings"));
    }

    #[test]
    fn next_id_round_trips_and_is_independently_mutable_from_project() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        assert_eq!(settings.next_id(), None);
        settings.set_project("agile-md").unwrap();
        settings.set_next_id(7);
        settings.save(&board).unwrap();

        let reloaded = load(&board);
        assert_eq!(reloaded.next_id(), Some(7));
        assert_eq!(reloaded.project(), Some("agile-md"));
    }

    #[test]
    fn other_keys_in_the_file_survive_a_project_write() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        std::fs::create_dir_all(&board.root).unwrap();
        std::fs::write(path(&board), "forge = gitea\n").unwrap();

        let mut settings = load(&board);
        settings.set_project("agile-md").unwrap();
        settings.save(&board).unwrap();

        let reread = Config::load_from(&path(&board));
        assert_eq!(reread.get("forge"), Some("gitea"));
        assert_eq!(reread.get(PROJECT), Some("agile-md"));
    }

    #[test]
    fn project_names_allow_letters_digits_hyphens_periods_colons_and_underscores() {
        assert!(validate_project_name("agile-md").is_ok());
        assert!(validate_project_name("Agile-MD-2").is_ok());
        assert!(validate_project_name("agile_md").is_ok());
        assert!(validate_project_name("agile.md").is_ok());
        assert!(validate_project_name("agile:md").is_ok());
        assert!(validate_project_name("").is_err());
        assert!(validate_project_name("agile md").is_err());
    }

    #[test]
    fn lease_hours_defaults_to_eight_with_nothing_set() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(&board(dir.path())).lease_hours(), 8.0);
    }

    #[test]
    fn lease_hours_reads_the_board_setting_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.config.set(LEASE_HOURS, "2");
        settings.save(&board).unwrap();
        assert_eq!(load(&board).lease_hours(), 2.0);
    }

    #[test]
    fn lease_hours_falls_back_to_the_default_when_unparsable_or_non_positive() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.config.set(LEASE_HOURS, "not a number");
        assert_eq!(settings.lease_hours(), 8.0);
        settings.config.set(LEASE_HOURS, "0");
        assert_eq!(settings.lease_hours(), 8.0);
        settings.config.set(LEASE_HOURS, "-3");
        assert_eq!(settings.lease_hours(), 8.0);
    }

    #[test]
    fn wip_limit_defaults_to_two_for_doing_and_never_limits_todo() {
        let dir = tempfile::tempdir().unwrap();
        let settings = load(&board(dir.path()));
        assert_eq!(settings.wip_limit(Column::Todo), None);
        assert_eq!(settings.wip_limit(Column::Doing), Some(2));
    }

    #[test]
    fn a_todo_limit_left_in_the_settings_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.config.set(WIP_TODO, "3");
        settings.save(&board).unwrap();
        assert_eq!(load(&board).wip_limit(Column::Todo), None);
        assert_eq!(
            load(&board).wip_limit(Column::Doing),
            Some(2),
            "still defaults"
        );
    }

    #[test]
    fn wip_limit_only_applies_to_todo_and_doing() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        // Even set by hand, backlog/done have nowhere to store a value read
        // from — Column::Backlog/Done short-circuit to None unconditionally.
        settings.config.set("wip-backlog", "1");
        settings.config.set("wip-done", "1");
        settings.save(&board).unwrap();
        assert_eq!(load(&board).wip_limit(Column::Backlog), None);
        assert_eq!(load(&board).wip_limit(Column::Done), None);
    }

    #[test]
    fn wip_limit_for_doing_treats_unparsable_as_absent_defaulting_to_two() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.config.set(WIP_LIMIT, "not a number");
        assert_eq!(settings.wip_limit(Column::Doing), Some(2));
    }

    #[test]
    fn wip_limit_for_doing_treats_explicit_zero_as_turned_off() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.config.set(WIP_LIMIT, "0");
        assert_eq!(settings.wip_limit(Column::Doing), None);
    }

    #[test]
    fn set_wip_limit_round_trips_through_save() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.set_wip_limit(Column::Doing, Some(4)).unwrap();
        settings.save(&board).unwrap();
        assert_eq!(load(&board).wip_limit(Column::Doing), Some(4));
        assert_eq!(load(&board).wip_limit(Column::Todo), None, "untouched");
    }

    #[test]
    fn set_wip_limit_for_doing_stores_explicit_zero_distinctly_from_clearing() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());

        let mut settings = load(&board);
        settings.set_wip_limit(Column::Doing, Some(0)).unwrap();
        settings.save(&board).unwrap();
        assert_eq!(
            load(&board).wip_limit(Column::Doing),
            None,
            "explicit 0 means turned off"
        );

        let mut settings = load(&board);
        settings.set_wip_limit(Column::Doing, None).unwrap();
        settings.save(&board).unwrap();
        assert_eq!(
            load(&board).wip_limit(Column::Doing),
            Some(2),
            "clearing falls back to the default, not unlimited"
        );
    }

    #[test]
    fn set_wip_limit_refuses_every_column_but_doing() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = load(&board(dir.path()));
        let err = settings.set_wip_limit(Column::Todo, Some(2)).unwrap_err();
        assert!(err.to_string().contains("only doing is limited"), "{err}");
        assert!(settings.set_wip_limit(Column::Backlog, Some(1)).is_err());
        assert!(settings.set_wip_limit(Column::Done, Some(1)).is_err());
    }

    #[test]
    fn auto_commit_defaults_to_on() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(&board(dir.path())).auto_commit());
    }

    #[test]
    fn auto_commit_can_be_turned_off() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.config.set(AUTO_COMMIT, "off");
        settings.save(&board).unwrap();
        assert!(!load(&board).auto_commit());
    }

    #[test]
    fn auto_commit_recognises_0_and_false_too() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.config.set(AUTO_COMMIT, "0");
        assert!(!settings.auto_commit());
        settings.config.set(AUTO_COMMIT, "false");
        assert!(!settings.auto_commit());
        settings.config.set(AUTO_COMMIT, "FALSE");
        assert!(!settings.auto_commit(), "case-insensitive");
    }

    #[test]
    fn auto_commit_ignores_unparsable_values() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.config.set(AUTO_COMMIT, "sure, why not");
        assert!(
            settings.auto_commit(),
            "unparsable falls back to on, not off"
        );
    }

    #[test]
    fn repos_defaults_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(&board(dir.path())).repos().is_empty());
    }

    #[test]
    fn declare_repo_round_trips_and_preserves_order() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.declare_repo("amd-documents").unwrap();
        settings.declare_repo("amd-ui").unwrap();
        settings.save(&board).unwrap();
        assert_eq!(load(&board).repos(), ["amd-documents", "amd-ui"]);
    }

    #[test]
    fn declaring_the_same_repo_twice_is_a_silent_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.declare_repo("amd-ui").unwrap();
        settings.declare_repo("amd-ui").unwrap();
        settings.save(&board).unwrap();
        assert_eq!(load(&board).repos(), ["amd-ui"]);
    }

    #[test]
    fn declare_repo_validates_the_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = load(&board(dir.path()));
        assert!(settings.declare_repo("has a space").is_err());
        assert!(settings.declare_repo("has/a/slash").is_err());
    }

    #[test]
    fn a_slash_is_rejected_even_though_the_other_new_characters_are_allowed() {
        // Unlike '.', ':' and '_', a slash breaks amd init's own path-join
        // and owner/repo logic, not just forge compatibility — see this
        // function's doc comment.
        let err = validate_project_name("agile/md").unwrap_err().to_string();
        assert!(err.contains("only letters"), "{err}");
    }

    // ticket 057: the documents sibling ------------------------------

    /// `<dir>/proj-tickets/tasks` — a board named so sibling derivation has
    /// something to find, distinct from the plain `board()` helper above
    /// (whose tempdir name never ends in `-tickets`, so it always falls
    /// through to "no documents sibling", exactly like every test above
    /// this section already relies on).
    fn tickets_board(dir: &std::path::Path) -> Board {
        Board::at(dir.join("proj-tickets").join("tasks"))
    }

    fn documents_settings_dir(dir: &std::path::Path) -> std::path::PathBuf {
        let settings = dir.join("proj-documents").join(".settings");
        std::fs::create_dir_all(&settings).unwrap();
        settings
    }

    fn user(name: &str, email: &str) -> User {
        User {
            name: name.to_string(),
            email: email.to_string(),
        }
    }

    #[test]
    fn users_survive_a_round_trip_in_join_order() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        assert!(
            settings
                .add_user(&user("Old Dippy", "dippy@example.com"))
                .unwrap()
        );
        assert!(
            settings
                .add_user(&user("Oberi Joe", "joe@example.com"))
                .unwrap()
        );
        settings.save(&board).unwrap();

        assert_eq!(
            load(&board).users(),
            vec![
                user("Old Dippy", "dippy@example.com"),
                user("Oberi Joe", "joe@example.com")
            ]
        );
    }

    fn git_repo(identity: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .status()
                .unwrap()
        };
        git(&["init", "-q"]);
        for (key, value) in identity {
            git(&["config", key, value]);
        }
        dir
    }

    // Repo-local values, so the machine's own global identity can't leak in
    // (a repo's config overrides the global one).
    #[test]
    fn the_git_identity_is_the_user() {
        let repo = git_repo(&[("user.name", "Ada"), ("user.email", "a@example.com")]);
        assert_eq!(
            User::from_git(repo.path()),
            Some(user("Ada", "a@example.com"))
        );
    }

    #[test]
    fn a_name_with_a_comma_still_parses() {
        assert_eq!(
            parse_users("Preece, Tim <t@example.com>, Ada <a@example.com>"),
            vec![
                user("Preece, Tim", "t@example.com"),
                user("Ada", "a@example.com")
            ]
        );
    }

    #[test]
    fn adding_the_same_email_again_changes_nothing_whatever_its_case_or_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = load(&board(dir.path()));
        assert!(settings.add_user(&user("Ada", "Ada@Example.com")).unwrap());
        assert!(
            !settings
                .add_user(&user("Ada L", "ada@example.com"))
                .unwrap()
        );
        assert_eq!(settings.users(), vec![user("Ada", "Ada@Example.com")]);
    }

    #[test]
    fn an_unusable_identity_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = load(&board(dir.path()));
        assert!(settings.add_user(&user("Ada", "")).is_err());
        assert!(
            settings
                .add_user(&user("Ada <x>", "a@example.com"))
                .is_err()
        );
        assert!(settings.users().is_empty());
    }

    fn documents_key(dir: &std::path::Path, key: &str, value: &str) {
        std::fs::write(documents_settings_dir(dir).join(key), format!("{value}\n")).unwrap();
    }

    #[test]
    fn every_setting_lives_in_the_tickets_file_even_beside_documents() {
        let dir = tempfile::tempdir().unwrap();
        let board = tickets_board(dir.path());
        std::fs::create_dir_all(board.repo_root()).unwrap();
        documents_settings_dir(dir.path());
        let mut settings = load(&board);
        settings.set_project("proj").unwrap();
        settings.declare_repo("code").unwrap();
        settings.set_wip_limit(Column::Doing, Some(4)).unwrap();
        settings.set_lease_hours(2.0).unwrap();
        settings.save(&board).unwrap();
        let local = Config::load_from(&path(&board));
        assert_eq!(local.get(PROJECT), Some("proj"));
        assert_eq!(local.get(REPOS), Some("code"));
        assert_eq!(local.get(WIP_LIMIT), Some("4"));
        assert!(
            std::fs::read_dir(dir.path().join("proj-documents/.settings"))
                .unwrap()
                .next()
                .is_none(),
            "nothing is written to documents any more"
        );
    }

    #[test]
    fn migrating_unions_repos_fills_gaps_and_lets_tickets_win() {
        let dir = tempfile::tempdir().unwrap();
        let board = tickets_board(dir.path());
        std::fs::create_dir_all(board.repo_root()).unwrap();
        std::fs::write(path(&board), "repos = TestRepository\nwip-limit = 5\n").unwrap();
        documents_settings_dir(dir.path());
        documents_key(dir.path(), REPOS, "TestRepository,long-one,another");
        documents_key(dir.path(), WIP_LIMIT, "3");
        documents_key(dir.path(), LEASE_HOURS, "4");

        let moved = migrate_from_documents(&board).unwrap();
        assert_eq!(moved, vec!["lease-hours".to_string(), "repos".to_string()]);
        let settings = load(&board);
        assert_eq!(settings.repos(), ["TestRepository", "long-one", "another"]);
        assert_eq!(settings.wip_limit(Column::Doing), Some(5), "tickets wins");
        assert_eq!(settings.lease_hours(), 4.0, "documents fills a gap");

        assert!(
            migrate_from_documents(&board).unwrap().is_empty(),
            "silent once across"
        );
    }

    #[test]
    fn no_documents_sibling_means_nothing_to_migrate() {
        let dir = tempfile::tempdir().unwrap();
        let board = tickets_board(dir.path());
        std::fs::create_dir_all(board.repo_root()).unwrap();
        assert!(migrate_from_documents(&board).unwrap().is_empty());
        assert!(!path(&board).exists(), "nothing written");
    }

    #[test]
    fn a_repo_can_be_undeclared() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = load(&board(dir.path()));
        settings.declare_repo("a").unwrap();
        settings.declare_repo("b").unwrap();
        assert!(settings.undeclare_repo("a"));
        assert!(!settings.undeclare_repo("a"));
        assert_eq!(settings.repos(), ["b"]);
    }

    #[test]
    fn an_assignee_off_the_users_list_is_warned_about_only_when_there_is_one() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        assert_eq!(
            unknown_assignee(&board, "anyone"),
            None,
            "no list, no warning"
        );
        let mut settings = load(&board);
        for (name, email) in [
            ("oldippy", "o@t.co"),
            ("Ann", "a@t.co"),
            ("oldippy", "o2@t.co"),
        ] {
            settings
                .add_user(&User {
                    name: name.into(),
                    email: email.into(),
                })
                .unwrap();
        }
        settings.save(&board).unwrap();
        assert_eq!(load(&board).user_names(), ["Ann", "oldippy"]);
        assert_eq!(unknown_assignee(&board, "oldippy"), None);
        assert_eq!(unknown_assignee(&board, ""), None);
        let warning = unknown_assignee(&board, "stranger").unwrap();
        assert!(
            warning.contains("isn't one of this project's users"),
            "{warning}"
        );
    }
}
