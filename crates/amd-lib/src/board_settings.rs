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
//! - `wip-todo`: the most tickets `todo/` may hold at once. Absent,
//!   unparsable or `0` all mean unlimited — board-local for the same reason
//!   as `lease-hours` (a team decision, not a personal one), but unlike it
//!   there is a programmatic setter (`set_wip_limit`) — `amdui`'s Settings
//!   modal writes it, not just a hand edit.
//! - `wip-limit`: `doing/`'s own WIP limit, deliberately asymmetric with
//!   `wip-todo` rather than a `wip-doing` twin of it. Absent or unparsable
//!   defaults to **1**, not unlimited — a queue of more than one ticket
//!   "in progress" at once is treated as the mistake, not the norm.
//!   Explicit `0` is what turns it off, and is stored literally rather than
//!   collapsed into "absent" the way `wip-todo`'s `0` is, precisely so the
//!   two states (nobody set it; somebody deliberately disabled it) stay
//!   distinguishable. `amd wip-limit <n>` is the CLI setter.
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
//!
//! One repository, one settings file — a repository with more than one
//! board (two different `AMD_DIR` trees) shares this file and its counter
//! between them, rather than each getting its own. Not a real scenario:
//! `AMD_DIR` exists so a repository can pick a non-default location for its
//! one board, not to run several boards side by side.
//!
//! **`project`/`repos`/`wip-todo`/`wip-limit`/`lease-hours` move to
//! `{project}-governance` once one is locatable** (ticket 057) — a
//! `.settings/` *directory* there, one small file per key, so a changed
//! setting reviews as its own legible diff in a repository that's already
//! PR-reviewed, rather than a bare commit straight to this repository's own
//! `main`. `next-id`/`commit` stay here regardless: `next-id` is bumped on
//! every `amd new` and must stay on the fast, direct-to-main path;
//! `commit` is the auto-commit toggle itself. A board with no locatable
//! governance sibling — a single-repo board, or one that predates the
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
const REPOS: &str = "repos";
const AUTO_COMMIT: &str = "commit";

/// Keys that move to `{project}-governance`'s `.settings/` directory once
/// one is locatable. Everything else this module knows about (`next-id`,
/// `commit`) — and anything it doesn't recognise at all — stays in this
/// board's own file.
const GOVERNANCE_KEYS: &[&str] = &[PROJECT, LEASE_HOURS, WIP_TODO, WIP_LIMIT, REPOS];

/// `<repo-root>/.settings` — a sibling of the board directory, not inside it.
pub fn path(board: &Board) -> std::path::PathBuf {
    board.repo_root().join(".settings")
}

/// Where `{project}-governance`'s `.settings/` directory can be found for
/// this board, if at all. Sibling-path derivation first — the standing
/// `rfc/0003` layout, `{project}-tickets` next to `{project}-governance` —
/// so this works even on a machine that's never run `amd register`. Falls
/// back to a **project-scoped** registry lookup, `{project}-governance`,
/// using this board's own (possibly stale, pre-migration) locally recorded
/// `project` value — deliberately *not* the machine-wide `governance`
/// alias: a machine can have more than one project registered at once, and
/// resolving to "whichever repo happens to be aliased governance right
/// now" would silently leak a *different* project's settings into this
/// one — confirmed live, the hard way, as a real `project_trust` violation
/// during this ticket's own testing. `None` means no governance repository
/// is locatable at all — not an error, just "keep using the local file",
/// which is also exactly what every board that predates the governance
/// split needs.
fn governance_root(board: &Board) -> Option<std::path::PathBuf> {
    let repo = board.repo_root();
    if let Some(name) = repo.file_name().and_then(|n| n.to_str())
        && let Some(prefix) = name.strip_suffix("-tickets")
        && let Some(parent) = repo.parent()
    {
        let sibling = parent.join(format!("{prefix}-governance"));
        if sibling.join(".settings").is_dir() {
            return Some(sibling);
        }
    }
    let local = Config::load_from(&path(board));
    let project = local.get(PROJECT)?;
    let declared = format!("{project}-governance");
    crate::registry::Registry::load()
        .find(&declared)
        .map(|entry| entry.root.clone())
}

/// One key, one file, its entire trimmed content the value — simpler than
/// reusing `Config`'s `key = value` parser for a file that only ever holds
/// one key, and it reviews as an even smaller diff.
fn read_governance_key(governance_root: &std::path::Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(governance_root.join(".settings").join(key)).ok()?;
    let value = text.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn write_governance_key(governance_root: &std::path::Path, key: &str, value: &str) -> Result<()> {
    let dir = governance_root.join(".settings");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(key), format!("{value}\n"))?;
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub struct BoardSettings {
    /// A merged, in-memory view — `GOVERNANCE_KEYS` from governance (when
    /// locatable) layered over this board's own file, which every getter
    /// below still just reads from directly. Only `save` needs to know
    /// which key belongs in which place.
    config: Config,
    governance_root: Option<std::path::PathBuf>,
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

    /// The most tickets `column` may hold at once — `AMD_WIP_TODO`/
    /// `AMD_WIP_LIMIT`, then this board's own `.settings`, then a
    /// column-specific default. Only `Column::Todo`/`Column::Doing` can
    /// carry a limit; anything else always reads as unlimited, matching the
    /// ticket's own scope.
    ///
    /// **Todo and Doing resolve an absent/unparsable value differently, on
    /// purpose.** Todo falls back to unlimited, same as it always has.
    /// Doing falls back to **1** — deliberately asymmetric, per the ticket
    /// that added this: a queue of more than one "in progress" ticket is
    /// the thing worth naming a default against, todo isn't. Explicit `0`
    /// still means "off" for both, but only Doing needs to store it
    /// literally to tell "somebody set it to 0" apart from "nobody set it" —
    /// Todo's two absent-like states (missing, or `0`) mean the same thing
    /// anyway, so they don't need to stay distinguishable.
    pub fn wip_limit(&self, column: Column) -> Option<u32> {
        match column {
            Column::Todo => crate::settings::resolved("AMD_WIP_TODO", &self.config, WIP_TODO)
                .and_then(|value| value.trim().parse::<u32>().ok())
                .filter(|limit| *limit > 0),
            Column::Doing => {
                let parsed = crate::settings::resolved("AMD_WIP_LIMIT", &self.config, WIP_LIMIT)
                    .and_then(|value| value.trim().parse::<u32>().ok());
                match parsed {
                    Some(0) => None,
                    Some(limit) => Some(limit),
                    None => Some(1),
                }
            }
            Column::Backlog | Column::Done => None,
        }
    }

    /// Sets or clears `column`'s WIP limit in this board's own `.settings`.
    /// This never touches `AMD_WIP_TODO`/`AMD_WIP_LIMIT` — an env var set at
    /// all still wins over whatever this writes, same precedence
    /// `wip_limit` already documents.
    ///
    /// For **Todo**, `None` or a limit of `0` both clear the key back to an
    /// empty string, which `wip_limit`'s own parse chain already reads as
    /// absent (unlimited) — no separate "unset" mechanism needed.
    ///
    /// For **Doing**, `0` is stored *literally* rather than collapsed into
    /// clearing: `wip_limit` now treats an absent/unparsable value as `1`,
    /// not unlimited, so `Some(0)` and `None` have to reach the file as
    /// different states for the read side to tell them apart.
    pub fn set_wip_limit(&mut self, column: Column, limit: Option<u32>) -> Result<()> {
        match column {
            Column::Todo => {
                let value = limit.filter(|l| *l > 0).map(|l| l.to_string());
                self.config.set(WIP_TODO, value.as_deref().unwrap_or(""));
                Ok(())
            }
            Column::Doing => {
                let value = limit.map(|l| l.to_string());
                self.config.set(WIP_LIMIT, value.as_deref().unwrap_or(""));
                Ok(())
            }
            Column::Backlog | Column::Done => {
                bail!("{} has no WIP limit", column.as_str())
            }
        }
    }

    /// Where a governance-shaped setting just written by `save` actually
    /// landed, if anywhere other than this board's own file — so a caller
    /// can tell the user it needs a commit and a PR there, rather than
    /// having already been pushed the way a tickets-repo change is.
    pub fn governance_root(&self) -> Option<&std::path::Path> {
        self.governance_root.as_deref()
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

    /// Persists whatever's in memory to wherever each key actually belongs.
    /// With no locatable governance repository, this is unchanged from
    /// before ticket 057 — everything goes in this board's own file.
    pub fn save(&self, board: &Board) -> Result<()> {
        let Some(root) = &self.governance_root else {
            return self.config.save_to(&path(board));
        };
        // A fresh read, not `self.config` verbatim: `self.config` also
        // holds whatever governance keys were merged in at `load()` time,
        // and those must not leak back into this board's own file — a
        // legacy value already sitting here (pre-migration) is left alone
        // rather than actively deleted, since migration is a deliberate,
        // separate step, not something a save should do as a side effect.
        let mut local = Config::load_from(&path(board));
        for key in [NEXT_ID, AUTO_COMMIT] {
            if let Some(value) = self.config.get(key) {
                local.set(key, value);
            }
        }
        local.save_to(&path(board))?;
        for key in GOVERNANCE_KEYS {
            if let Some(value) = self.config.get(key) {
                write_governance_key(root, key, value)?;
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
    let governance_root = governance_root(board);
    if let Some(root) = &governance_root {
        for key in GOVERNANCE_KEYS {
            if let Some(value) = read_governance_key(root, key) {
                config.set(key, &value);
            }
        }
    }
    BoardSettings {
        config,
        governance_root,
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
    fn wip_limit_defaults_to_unlimited_for_todo_and_one_for_doing() {
        let dir = tempfile::tempdir().unwrap();
        let settings = load(&board(dir.path()));
        assert_eq!(settings.wip_limit(Column::Todo), None);
        assert_eq!(settings.wip_limit(Column::Doing), Some(1));
    }

    #[test]
    fn wip_limit_reads_the_board_setting_per_column() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.config.set(WIP_TODO, "3");
        settings.save(&board).unwrap();
        assert_eq!(load(&board).wip_limit(Column::Todo), Some(3));
        assert_eq!(
            load(&board).wip_limit(Column::Doing),
            Some(1),
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
    fn wip_limit_ignores_zero_and_unparsable_values_for_todo() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.config.set(WIP_TODO, "0");
        assert_eq!(settings.wip_limit(Column::Todo), None);
        settings.config.set(WIP_TODO, "not a number");
        assert_eq!(settings.wip_limit(Column::Todo), None);
    }

    #[test]
    fn wip_limit_for_doing_treats_unparsable_as_absent_defaulting_to_one() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.config.set(WIP_LIMIT, "not a number");
        assert_eq!(settings.wip_limit(Column::Doing), Some(1));
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
            Some(1),
            "clearing falls back to the default, not unlimited"
        );
    }

    #[test]
    fn set_wip_limit_none_clears_it_back_to_unlimited() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.set_wip_limit(Column::Todo, Some(2)).unwrap();
        settings.save(&board).unwrap();
        assert_eq!(load(&board).wip_limit(Column::Todo), Some(2));

        let mut settings = load(&board);
        settings.set_wip_limit(Column::Todo, None).unwrap();
        settings.save(&board).unwrap();
        assert_eq!(load(&board).wip_limit(Column::Todo), None);
    }

    #[test]
    fn set_wip_limit_refuses_backlog_and_done() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = load(&board(dir.path()));
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

    // ticket 057: the governance sibling ------------------------------

    /// `<dir>/proj-tickets/tasks` — a board named so sibling derivation has
    /// something to find, distinct from the plain `board()` helper above
    /// (whose tempdir name never ends in `-tickets`, so it always falls
    /// through to "no governance sibling", exactly like every test above
    /// this section already relies on).
    fn tickets_board(dir: &std::path::Path) -> Board {
        Board::at(dir.join("proj-tickets").join("tasks"))
    }

    fn governance_settings_dir(dir: &std::path::Path) -> std::path::PathBuf {
        let settings = dir.join("proj-governance").join(".settings");
        std::fs::create_dir_all(&settings).unwrap();
        settings
    }

    #[test]
    fn a_board_named_something_tickets_with_no_sibling_falls_back_to_local() {
        let dir = tempfile::tempdir().unwrap();
        let board = tickets_board(dir.path());
        // No proj-governance directory created at all.
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
    fn a_governance_sibling_is_found_by_the_tickets_suffix_convention() {
        let dir = tempfile::tempdir().unwrap();
        let board = tickets_board(dir.path());
        let settings_dir = governance_settings_dir(dir.path());
        std::fs::write(settings_dir.join("project"), "proj\n").unwrap();

        assert_eq!(load(&board).project(), Some("proj"));
    }

    #[test]
    fn governance_values_win_over_a_stale_local_copy() {
        let dir = tempfile::tempdir().unwrap();
        let board = tickets_board(dir.path());
        std::fs::create_dir_all(board.repo_root()).unwrap();
        std::fs::write(path(&board), "project = old-value\n").unwrap();
        let settings_dir = governance_settings_dir(dir.path());
        std::fs::write(settings_dir.join("project"), "new-value\n").unwrap();

        assert_eq!(
            load(&board).project(),
            Some("new-value"),
            "governance's own copy takes precedence once one exists"
        );
    }

    #[test]
    fn saving_with_a_governance_sibling_writes_there_not_locally() {
        let dir = tempfile::tempdir().unwrap();
        let board = tickets_board(dir.path());
        governance_settings_dir(dir.path());

        let mut settings = load(&board);
        settings.declare_repo("some-repo").unwrap();
        settings.set_wip_limit(Column::Doing, Some(3)).unwrap();
        settings.save(&board).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.path().join("proj-governance/.settings/repos")).unwrap(),
            "some-repo\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("proj-governance/.settings/wip-limit"))
                .unwrap(),
            "3\n"
        );
        let local = Config::load_from(&path(&board));
        assert_eq!(
            local.get(REPOS),
            None,
            "governance-shaped keys must not also land in the local file"
        );
    }

    #[test]
    fn saving_next_id_with_a_governance_sibling_still_writes_locally() {
        let dir = tempfile::tempdir().unwrap();
        let board = tickets_board(dir.path());
        governance_settings_dir(dir.path());

        let mut settings = load(&board);
        settings.set_next_id(42);
        settings.save(&board).unwrap();

        assert_eq!(load(&board).next_id(), Some(42));
        assert!(
            !dir.path()
                .join("proj-governance/.settings/next-id")
                .exists(),
            "next-id is never governance's concern"
        );
    }

    #[test]
    fn a_pre_migration_local_value_is_left_alone_not_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let board = tickets_board(dir.path());
        std::fs::create_dir_all(board.repo_root()).unwrap();
        std::fs::write(path(&board), "repos = leftover-repo\n").unwrap();
        governance_settings_dir(dir.path());

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
}
