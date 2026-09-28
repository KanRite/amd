//! Local, per-machine repository aliases: a short name a team member can
//! type instead of a repository's full declared name — `tickets` instead
//! of `agile-md-tickets`, say — resolving to the same registered entry.
//!
//! Deliberately separate from `registry.rs`'s own file: that one is "a list
//! of paths and nothing more" by design, and an alias is a second, purely
//! local concern layered on top, not a property of a path
//! (`rfc/0003-ticket-repository-topology-and-team-collaboration.md`'s
//! "friendly display names are a separate, purely cosmetic concern" note
//! names exactly this feature). Every team member picks their own aliases
//! independently; nothing here is shared or git-tracked, and nothing here
//! ever touches a project's declared `repos` list.
//!
//! Stored at `$XDG_CONFIG_HOME/agile-md/aliases`, reusing
//! `settings::Config`'s `key = value` format rather than inventing a
//! second parser for the same shape.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::registry::Registry;
use crate::settings::Config;

#[derive(Clone, Debug, Default)]
pub struct Aliases {
    config: Config,
}

impl Aliases {
    pub fn load() -> Aliases {
        match path() {
            Some(path) => Aliases::load_from(&path),
            None => Aliases::default(),
        }
    }

    /// The half that takes a path, so tests need no environment fiddling.
    pub fn load_from(path: &Path) -> Aliases {
        Aliases {
            config: Config::load_from(path),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = path().ok_or_else(|| anyhow::anyhow!("no config directory"))?;
        self.save_to(&path)
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        self.config.save_to(path)
    }

    /// Assigns `alias` to `target`, a repository already registered on this
    /// machine. Refuses if `target` isn't registered (an alias to nothing
    /// is never useful), and refuses if `alias` is already a *different*
    /// repository's own registered name — an alias and a real name must
    /// never be ambiguous with each other.
    pub fn set(&mut self, registry: &Registry, target: &str, alias: &str) -> Result<()> {
        if registry.find(target).is_none() {
            bail!("{target} is not registered on this machine — run `amd register` first");
        }
        if let Some(entry) = registry.find(alias)
            && entry.name != target
        {
            bail!(
                "{alias} is already the name of a registered repository ({}) — \
                 choose a different alias",
                entry.name
            );
        }
        self.config.set(alias, target);
        Ok(())
    }

    /// `name` resolves to its target if it's a known alias; otherwise `name`
    /// unchanged — so a call site can always resolve first, whether or not
    /// the input was ever an alias.
    pub fn resolve(&self, name: &str) -> String {
        self.config
            .get(name)
            .map(str::to_string)
            .unwrap_or_else(|| name.to_string())
    }
}

/// `$XDG_CONFIG_HOME/agile-md/aliases`, else `~/.config/agile-md/aliases`.
fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("agile-md").join("aliases"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn repo(parent: &Path, name: &str) -> PathBuf {
        let root = parent.join(name);
        std::fs::create_dir_all(&root).unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init", "-q"])
            .status()
            .unwrap();
        root
    }

    fn registered(dir: &Path, name: &str) -> Registry {
        let mut registry = Registry::default();
        registry.add(&repo(dir, name)).unwrap();
        registry
    }

    #[test]
    fn resolve_is_a_no_op_for_an_unknown_name() {
        let aliases = Aliases::default();
        assert_eq!(aliases.resolve("agile-md-tickets"), "agile-md-tickets");
    }

    #[test]
    fn an_alias_resolves_to_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registered(dir.path(), "agile-md-tickets");
        let mut aliases = Aliases::default();
        aliases
            .set(&registry, "agile-md-tickets", "tickets")
            .unwrap();
        assert_eq!(aliases.resolve("tickets"), "agile-md-tickets");
    }

    #[test]
    fn setting_an_alias_to_an_unregistered_repo_is_refused() {
        let registry = Registry::default();
        let mut aliases = Aliases::default();
        let err = aliases.set(&registry, "nope", "n").unwrap_err().to_string();
        assert!(err.contains("not registered"), "{err}");
    }

    #[test]
    fn an_alias_cannot_collide_with_a_different_repos_real_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registered(dir.path(), "agile-md-tickets");
        registry.add(&repo(dir.path(), "tickets")).unwrap();
        let mut aliases = Aliases::default();
        let err = aliases
            .set(&registry, "agile-md-tickets", "tickets")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("already the name of a registered repository"),
            "{err}"
        );
    }

    #[test]
    fn re_setting_the_same_alias_to_the_same_target_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registered(dir.path(), "agile-md-tickets");
        let mut aliases = Aliases::default();
        aliases
            .set(&registry, "agile-md-tickets", "tickets")
            .unwrap();
        aliases
            .set(&registry, "agile-md-tickets", "tickets")
            .unwrap();
        assert_eq!(aliases.resolve("tickets"), "agile-md-tickets");
    }

    #[test]
    fn aliases_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registered(dir.path(), "agile-md-tickets");
        let mut aliases = Aliases::default();
        aliases
            .set(&registry, "agile-md-tickets", "tickets")
            .unwrap();

        let path = dir.path().join("config/aliases");
        aliases.save_to(&path).unwrap();
        let reloaded = Aliases::load_from(&path);
        assert_eq!(reloaded.resolve("tickets"), "agile-md-tickets");
    }

    #[test]
    fn a_missing_file_is_an_empty_alias_set() {
        let dir = tempfile::tempdir().unwrap();
        let aliases = Aliases::load_from(&dir.path().join("nothing"));
        assert_eq!(aliases.resolve("tickets"), "tickets");
    }
}
