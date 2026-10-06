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
//!   recognised (it stays a documents-shaped key, below) so an old file
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
//!   joining user. Deliberately **not** a documents key: joining has to take
//!   effect on the direct-push path, not wait for a documents PR. The email
//!   is the identity (leases use `user.email`); the name is for display.
//!
//! One repository, one settings file — a repository with more than one
//! board (two different `AMD_DIR` trees) shares this file and its counter
//! between them, rather than each getting its own. Not a real scenario:
//! `AMD_DIR` exists so a repository can pick a non-default location for its
//! one board, not to run several boards side by side.
//!
//! **`project`/`repos`/`wip-todo`/`wip-limit`/`lease-hours` move to
//! `{project}-documents` once one is locatable** (ticket 057) — a
//! `.settings/` *directory* there, one small file per key, so a changed
//! setting reviews as its own legible diff in a repository that's already
//! PR-reviewed, rather than a bare commit straight to this repository's own
//! `main`. `next-id`/`commit` stay here regardless: `next-id` is bumped on
//! every `amd new` and must stay on the fast, direct-to-main path;
//! `commit` is the auto-commit toggle itself. A board with no locatable
//! documents sibling — a single-repo board, or one that predates the
//! split — keeps storing the moved keys in this file exactly as it always
//! has; nothing here requires the split to have happened.

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

/// Keys that move to `{project}-documents`'s `.settings/` directory once
/// one is locatable. Everything else this module knows about (`next-id`,
/// `commit`) — and anything it doesn't recognise at all — stays in this
/// board's own file.
const DOCUMENTS_KEYS: &[&str] = &[
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

/// Where `{project}-documents`'s `.settings/` directory can be found for
/// this board, if at all. Sibling-path derivation first — the standing
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
        .find(&declared)
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

fn write_documents_key(documents_root: &std::path::Path, key: &str, value: &str) -> Result<()> {
    let dir = documents_root.join(".settings");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(key), format!("{value}\n"))?;
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub struct BoardSettings {
    /// A merged, in-memory view — `DOCUMENTS_KEYS` from documents (when
    /// locatable) layered over this board's own file, which every getter
    /// below still just reads from directly. Only `save` needs to know
    /// which key belongs in which place.
    config: Config,
    documents_root: Option<std::path::PathBuf>,
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

    /// Where a documents-shaped setting just written by `save` actually
    /// landed, if anywhere other than this board's own file — so a caller
    /// can tell the user it needs a commit and a PR there, rather than
    /// having already been pushed the way a tickets-repo change is.
    pub fn documents_root(&self) -> Option<&std::path::Path> {
        self.documents_root.as_deref()
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

    /// Persists whatever's in memory to wherever each key actually belongs.
    /// With no locatable documents repository, this is unchanged from
    /// before ticket 057 — everything goes in this board's own file.
    pub fn save(&self, board: &Board) -> Result<()> {
        let Some(root) = &self.documents_root else {
            return self.config.save_to(&path(board));
        };
        // A fresh read, not `self.config` verbatim: `self.config` also
        // holds whatever documents keys were merged in at `load()` time,
        // and those must not leak back into this board's own file — a
        // legacy value already sitting here (pre-migration) is left alone
        // rather than actively deleted, since migration is a deliberate,
        // separate step, not something a save should do as a side effect.
        let mut local = Config::load_from(&path(board));
        for key in [NEXT_ID, AUTO_COMMIT, USERS] {
            if let Some(value) = self.config.get(key) {
                local.set(key, value);
            }
        }
        local.save_to(&path(board))?;
        for key in DOCUMENTS_KEYS {
            if let Some(value) = self.config.get(key) {
                write_documents_key(root, key, value)?;
            }
        }
        Ok(())
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
    let mut config = Config::load_from(&path(board));
    let documents_root = documents_root(board);
    if let Some(root) = &documents_root {
        for key in DOCUMENTS_KEYS {
            if let Some(value) = read_documents_key(root, key) {
                config.set(key, &value);
            }
        }
    }
    BoardSettings {
        config,
        documents_root,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(dir: &std::path::Path) -> Board {
        Board::at(dir.join("tasks"))
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

    #[test]
    fn a_board_named_something_tickets_with_no_sibling_falls_back_to_local() {
        let dir = tempfile::tempdir().unwrap();
        let board = tickets_board(dir.path());
        // No proj-documents directory created at all.
        let mut settings = load(&board);
        settings.set_project("proj").unwrap();
        settings.save(&board).unwrap();

        assert_eq!(load(&board).project(), Some("proj"));
        let local = Config::load_from(&path(&board));
        assert_eq!(
            local.get(PROJECT),
            Some("proj"),
            "no sibling exists, so this is exactly the pre-057 behaviour"
        );
    }

    #[test]
    fn a_documents_sibling_is_found_by_the_tickets_suffix_convention() {
        let dir = tempfile::tempdir().unwrap();
        let board = tickets_board(dir.path());
        let settings_dir = documents_settings_dir(dir.path());
        std::fs::write(settings_dir.join("project"), "proj\n").unwrap();

        assert_eq!(load(&board).project(), Some("proj"));
    }

    #[test]
    fn documents_values_win_over_a_stale_local_copy() {
        let dir = tempfile::tempdir().unwrap();
        let board = tickets_board(dir.path());
        std::fs::create_dir_all(board.repo_root()).unwrap();
        std::fs::write(path(&board), "project = old-value\n").unwrap();
        let settings_dir = documents_settings_dir(dir.path());
        std::fs::write(settings_dir.join("project"), "new-value\n").unwrap();

        assert_eq!(
            load(&board).project(),
            Some("new-value"),
            "the documents repo's own copy takes precedence once one exists"
        );
    }

    #[test]
    fn saving_with_a_documents_sibling_writes_there_not_locally() {
        let dir = tempfile::tempdir().unwrap();
        let board = tickets_board(dir.path());
        documents_settings_dir(dir.path());

        let mut settings = load(&board);
        settings.declare_repo("some-repo").unwrap();
        settings.set_wip_limit(Column::Doing, Some(3)).unwrap();
        settings.save(&board).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.path().join("proj-documents/.settings/repos")).unwrap(),
            "some-repo\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("proj-documents/.settings/wip-limit")).unwrap(),
            "3\n"
        );
        let local = Config::load_from(&path(&board));
        assert_eq!(
            local.get(REPOS),
            None,
            "documents-shaped keys must not also land in the local file"
        );
    }

    #[test]
    fn saving_next_id_with_a_documents_sibling_still_writes_locally() {
        let dir = tempfile::tempdir().unwrap();
        let board = tickets_board(dir.path());
        documents_settings_dir(dir.path());

        let mut settings = load(&board);
        settings.set_next_id(42);
        settings.save(&board).unwrap();

        assert_eq!(load(&board).next_id(), Some(42));
        assert!(
            !dir.path().join("proj-documents/.settings/next-id").exists(),
            "next-id is never the documents repo's concern"
        );
    }

    #[test]
    fn a_pre_migration_local_value_is_left_alone_not_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let board = tickets_board(dir.path());
        std::fs::create_dir_all(board.repo_root()).unwrap();
        std::fs::write(path(&board), "repos = leftover-repo\n").unwrap();
        documents_settings_dir(dir.path());

        let mut settings = load(&board);
        settings.set_next_id(1);
        settings.save(&board).unwrap();

        let local = Config::load_from(&path(&board));
        assert_eq!(
            local.get(REPOS),
            Some("leftover-repo"),
            "migration is a deliberate step, not a side effect of an unrelated save"
        );
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

    #[test]
    fn users_stay_in_the_tickets_file_with_a_documents_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let board = tickets_board(dir.path());
        std::fs::create_dir_all(board.repo_root()).unwrap();
        documents_settings_dir(dir.path());

        let mut settings = load(&board);
        settings.add_user(&user("Ada", "a@example.com")).unwrap();
        settings.save(&board).unwrap();

        assert_eq!(load(&board).users(), vec![user("Ada", "a@example.com")]);
        assert_eq!(
            Config::load_from(&path(&board)).get(USERS),
            Some("Ada <a@example.com>")
        );
        assert!(!dir.path().join("proj-documents/.settings/users").exists());
    }
}
