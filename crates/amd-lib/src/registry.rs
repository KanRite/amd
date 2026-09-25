//! The registered repositories.
//!
//! Deliberately a list of paths and nothing more. The tickets themselves are
//! never copied into a store: they're markdown in each repo, they change when
//! you check out a branch or pull someone else's work, and an index of them
//! would be wrong the moment it was written. Reading a board is a directory
//! listing and a few small files, which is fast enough that caching it would
//! only buy staleness.
//!
//! Stored one path per line at `$XDG_CONFIG_HOME/agile-md/repos`, so it can be
//! read, edited and version-controlled by hand.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::board::Board;

/// A repository on the list, and the board inside it.
#[derive(Clone, Debug)]
pub struct Entry {
    /// The repository's working-tree root.
    pub root: PathBuf,
    /// What to call it: the directory's own name.
    pub name: String,
}

impl Entry {
    pub fn new(root: PathBuf) -> Entry {
        let name = root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        Entry { root, name }
    }

    pub fn board(&self) -> Board {
        Board::in_repo(&self.root)
    }

    /// Has this repository got a board to show?
    pub fn has_board(&self) -> bool {
        self.board().root.is_dir()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Registry {
    pub entries: Vec<Entry>,
}

impl Registry {
    pub fn load() -> Registry {
        match path() {
            Some(path) => Registry::load_from(&path),
            None => Registry::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = path().ok_or_else(|| anyhow::anyhow!("no config directory"))?;
        self.save_to(&path)
    }

    /// The half that takes a path, so tests need no environment fiddling.
    pub fn load_from(path: &Path) -> Registry {
        let Ok(text) = fs::read_to_string(path) else {
            return Registry::default();
        };
        let entries = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| Entry::new(PathBuf::from(line)))
            .collect();
        Registry { entries }
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut text = String::from("# Repositories agile-md knows about, one path per line.\n");
        for entry in &self.entries {
            text.push_str(&entry.root.display().to_string());
            text.push('\n');
        }
        fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    /// Register a repository. Returns false if it was already on the list.
    pub fn add(&mut self, root: &Path) -> Result<bool> {
        let root = root
            .canonicalize()
            .with_context(|| format!("{} doesn't exist", root.display()))?;
        if !root.join(".git").exists() {
            bail!("{} is not a git repository", root.display());
        }
        if self.entries.iter().any(|entry| entry.root == root) {
            return Ok(false);
        }
        self.entries.push(Entry::new(root));
        self.entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(true)
    }

    /// Unregister a repository by path or by name.
    pub fn remove(&mut self, reference: &str) -> bool {
        let canonical = Path::new(reference).canonicalize().ok();
        let before = self.entries.len();
        self.entries.retain(|entry| {
            let by_path = canonical.as_ref().is_some_and(|path| entry.root == *path);
            let by_name = entry.name == reference;
            !(by_path || by_name)
        });
        self.entries.len() != before
    }

    /// Every registered repository that actually has a board, plus `current`
    /// when it isn't registered — you should always see the board you're
    /// standing in, listed or not.
    pub fn boards(&self, current: Option<&Board>) -> Vec<Entry> {
        let mut entries: Vec<Entry> = self
            .entries
            .iter()
            .filter(|entry| entry.has_board())
            .cloned()
            .collect();
        if let Some(board) = current
            && let Some(root) = board.root.parent()
            && !entries.iter().any(|entry| entry.root == root)
        {
            entries.insert(0, Entry::new(root.to_path_buf()));
        }
        entries
    }
}

/// Remember a repository we've just worked in, so the list fills itself from
/// the boards you actually use rather than from anyone maintaining it. Called
/// on every command that resolves a board, and writes only the first time a
/// repository is seen.
///
/// Best effort: a read-only config directory is no reason for `amd board` to
/// fail. Set `AMD_NO_REGISTER=1` (or the equivalent `no-register` settings
/// key — the env var wins when both are set) to keep the list entirely
/// manual — after which `amd repos remove` sticks, because nothing puts it
/// back.
pub fn remember(board: &Board) -> bool {
    let settings = crate::settings::Config::load();
    if crate::settings::resolved("AMD_NO_REGISTER", &settings, "no-register").as_deref()
        == Some("1")
    {
        return false;
    }
    let Some(path) = path() else {
        return false;
    };
    let Some(root) = board.root.parent() else {
        return false;
    };
    remember_in(&path, root)
}

/// The half that takes a path, so tests need no environment fiddling.
fn remember_in(config: &Path, root: &Path) -> bool {
    let mut registry = Registry::load_from(config);
    match registry.add(root) {
        Ok(true) => registry.save_to(config).is_ok(),
        _ => false,
    }
}

/// `$XDG_CONFIG_HOME/agile-md/repos`, else `~/.config/agile-md/repos`.
fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("agile-md").join("repos"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn repo(parent: &Path, name: &str) -> PathBuf {
        let root = parent.join(name);
        fs::create_dir_all(&root).unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init", "-q"])
            .status()
            .unwrap();
        root
    }

    #[test]
    fn repositories_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let alpha = repo(dir.path(), "alpha");
        let beta = repo(dir.path(), "beta");

        let mut registry = Registry::default();
        assert!(registry.add(&beta).unwrap());
        assert!(registry.add(&alpha).unwrap());
        // Sorted by name, so the list reads the same however it was built.
        assert_eq!(
            registry.entries.iter().map(|e| &e.name).collect::<Vec<_>>(),
            ["alpha", "beta"]
        );

        let path = dir.path().join("config/repos");
        registry.save_to(&path).unwrap();
        let loaded = Registry::load_from(&path);
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].name, "alpha");
    }

    #[test]
    fn adding_the_same_repository_twice_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let alpha = repo(dir.path(), "alpha");
        let mut registry = Registry::default();
        assert!(registry.add(&alpha).unwrap());
        assert!(!registry.add(&alpha).unwrap());
        assert_eq!(registry.entries.len(), 1);
    }

    #[test]
    fn only_git_repositories_can_be_registered() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain");
        fs::create_dir_all(&plain).unwrap();
        let mut registry = Registry::default();
        let err = registry.add(&plain).unwrap_err().to_string();
        assert!(err.contains("not a git repository"), "{err}");
        let err = registry
            .add(&dir.path().join("nowhere"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("doesn't exist"), "{err}");
    }

    #[test]
    fn removal_works_by_name_or_by_path() {
        let dir = tempfile::tempdir().unwrap();
        let alpha = repo(dir.path(), "alpha");
        let beta = repo(dir.path(), "beta");
        let mut registry = Registry::default();
        registry.add(&alpha).unwrap();
        registry.add(&beta).unwrap();

        assert!(registry.remove("alpha"));
        assert!(!registry.remove("alpha"));
        assert!(registry.remove(beta.to_str().unwrap()));
        assert!(registry.entries.is_empty());
    }

    #[test]
    fn working_in_a_repository_remembers_it_once() {
        let dir = tempfile::tempdir().unwrap();
        let alpha = repo(dir.path(), "alpha");
        let config = dir.path().join("config/repos");

        assert!(remember_in(&config, &alpha), "first visit registers");
        assert!(
            !remember_in(&config, &alpha),
            "second visit changes nothing"
        );
        let registry = Registry::load_from(&config);
        assert_eq!(registry.entries.len(), 1);
        assert_eq!(registry.entries[0].name, "alpha");
    }

    #[test]
    fn remembering_leaves_the_rest_of_the_list_alone() {
        let dir = tempfile::tempdir().unwrap();
        let alpha = repo(dir.path(), "alpha");
        let beta = repo(dir.path(), "beta");
        let config = dir.path().join("config/repos");

        let mut registry = Registry::default();
        registry.add(&beta).unwrap();
        registry.save_to(&config).unwrap();

        remember_in(&config, &alpha);
        let names: Vec<String> = Registry::load_from(&config)
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        assert_eq!(names, ["alpha", "beta"]);
    }

    #[test]
    fn a_missing_file_is_an_empty_registry() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            Registry::load_from(&dir.path().join("nothing"))
                .entries
                .is_empty()
        );
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repos");
        fs::write(&path, "# a comment\n\n/tmp/one\n  /tmp/two  \n").unwrap();
        let registry = Registry::load_from(&path);
        assert_eq!(registry.entries.len(), 2);
        assert_eq!(registry.entries[1].name, "two");
    }
}
