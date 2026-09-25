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
//!   hand-edited, the same as `branch-types`/`editor`/`no-register` in the
//!   *global* settings file, just board-local rather than per-machine
//!   because a lease policy is a team decision, not a personal preference.
//! - `wip-todo`/`wip-doing`: the most tickets `todo/`/`doing/` may hold at
//!   once. Absent (the default) means unlimited — same hand-edited,
//!   board-local shape as `lease-hours`, for the same reason.
//! - `commit`: whether a board-changing command also commits and pushes its
//!   own change (`crates/amd-cli/src/autocommit.rs`). Defaults **off** —
//!   unlike the three settings above, this one is deliberately *not*
//!   uniformly on, because it's only right for one of two real shapes a
//!   board can be in (ticket
//!   `012-have-amd-commit-board-and-document-changes`'s own "real dividing
//!   line"): a dedicated, board-only repository (RFC 0003's
//!   `{project}-tickets`) barely functions without it, since nothing else in
//!   the repository ever changes for a move to ride alongside — but a board
//!   sharing a tree with code would have `amd` committing across whatever
//!   the user already has staged, which is the opposite of helpful.
//!   `amd init <project>` sets `commit = true` explicitly when it scaffolds
//!   a tickets repository, since it knows which shape that one is; every
//!   other board (including one created by hand, or by `--local-only`)
//!   stays off until someone opts it in on purpose.
//!
//! One repository, one settings file — a repository with more than one
//! board (two different `AMD_DIR` trees) shares this file and its counter
//! between them, rather than each getting its own. Not a real scenario:
//! `AMD_DIR` exists so a repository can pick a non-default location for its
//! one board, not to run several boards side by side.

use anyhow::{Result, bail};

use crate::board::{Board, Column};
use crate::settings::Config;

const PROJECT: &str = "project";
const NEXT_ID: &str = "next-id";
const LEASE_HOURS: &str = "lease-hours";
const DEFAULT_LEASE_HOURS: f64 = 8.0;
const WIP_TODO: &str = "wip-todo";
const WIP_DOING: &str = "wip-doing";
const COMMIT: &str = "commit";

/// `<repo-root>/.settings` — a sibling of the board directory, not inside it.
pub fn path(board: &Board) -> std::path::PathBuf {
    board.repo_root().join(".settings")
}

#[derive(Clone, Debug, Default)]
pub struct BoardSettings {
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

    /// The most tickets `column` may hold at once — `AMD_WIP_TODO`/
    /// `AMD_WIP_DOING`, then this board's own `.settings`, then unlimited
    /// (`None`). Only `Column::Todo`/`Column::Doing` can carry a limit;
    /// anything else always reads as unlimited, matching the ticket's own
    /// scope.
    pub fn wip_limit(&self, column: Column) -> Option<u32> {
        let (env_var, key) = match column {
            Column::Todo => ("AMD_WIP_TODO", WIP_TODO),
            Column::Doing => ("AMD_WIP_DOING", WIP_DOING),
            Column::Backlog | Column::Done => return None,
        };
        crate::settings::resolved(env_var, &self.config, key)
            .and_then(|value| value.trim().parse::<u32>().ok())
            .filter(|limit| *limit > 0)
    }

    /// Whether a board-changing command also commits and pushes its own
    /// change — `AMD_COMMIT`, then this board's own `.settings`, then **off**
    /// by default. `1`/`true`/`yes` (case-insensitive) turn it on; anything
    /// else, including an unset or unparsable value, leaves it off. Off is
    /// the right default for a board sharing a tree with code — `amd`
    /// committing there would cut across whatever the user already has
    /// staged. `amd init <project>` sets this explicitly for the tickets
    /// repository it scaffolds, which is the one shape this is actually for
    /// (ticket `[[012-have-amd-commit-board-and-document-changes]]`).
    pub fn commit_enabled(&self) -> bool {
        crate::settings::resolved("AMD_COMMIT", &self.config, COMMIT).is_some_and(|value| {
            matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
        })
    }

    /// Sets `commit` explicitly — used by `amd init <project>` when it
    /// scaffolds a tickets repository, the one board shape where
    /// auto-commit is the right default.
    pub fn set_commit(&mut self, enabled: bool) {
        self.config
            .set(COMMIT, if enabled { "true" } else { "false" });
    }

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
    fn wip_limit_defaults_to_unlimited() {
        let dir = tempfile::tempdir().unwrap();
        let settings = load(&board(dir.path()));
        assert_eq!(settings.wip_limit(Column::Todo), None);
        assert_eq!(settings.wip_limit(Column::Doing), None);
    }

    #[test]
    fn wip_limit_reads_the_board_setting_per_column() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.config.set(WIP_TODO, "3");
        settings.save(&board).unwrap();
        assert_eq!(load(&board).wip_limit(Column::Todo), Some(3));
        assert_eq!(load(&board).wip_limit(Column::Doing), None);
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
    fn wip_limit_ignores_zero_and_unparsable_values() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.config.set(WIP_TODO, "0");
        assert_eq!(settings.wip_limit(Column::Todo), None);
        settings.config.set(WIP_TODO, "not a number");
        assert_eq!(settings.wip_limit(Column::Todo), None);
    }

    #[test]
    fn commit_enabled_defaults_to_off() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!load(&board(dir.path())).commit_enabled());
    }

    #[test]
    fn commit_enabled_reads_the_board_setting_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.config.set(COMMIT, "true");
        settings.save(&board).unwrap();
        assert!(load(&board).commit_enabled());
    }

    #[test]
    fn commit_enabled_is_case_insensitive_and_tolerates_unparsable_values() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.config.set(COMMIT, "TRUE");
        assert!(settings.commit_enabled());
        settings.config.set(COMMIT, "nonsense");
        assert!(!settings.commit_enabled(), "an unparsable value stays off");
    }

    #[test]
    fn set_commit_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let board = board(dir.path());
        let mut settings = load(&board);
        settings.set_commit(true);
        settings.save(&board).unwrap();
        assert!(load(&board).commit_enabled());
    }

    #[test]
    fn a_slash_is_rejected_even_though_the_other_new_characters_are_allowed() {
        // Unlike '.', ':' and '_', a slash breaks amd init's own path-join
        // and owner/repo logic, not just forge compatibility — see this
        // function's doc comment.
        let err = validate_project_name("agile/md").unwrap_err().to_string();
        assert!(err.contains("only letters"), "{err}");
    }
}
