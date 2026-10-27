//! The config file the front ends share.
//!
//! One `key = value` per line: greppable, hand-editable, no parser dependency.
//! Anything unrecognised is kept as it was rather than dropped, so one writer
//! cannot delete another's keys — the whole reason this is a shared type
//! rather than a `fs::write` in each front end. The library owns the file;
//! each front end owns the meaning of the keys in its own namespace.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

/// The file as a list of key/value pairs, in the order they were read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    entries: Vec<(String, String)>,
}

impl Config {
    pub fn load() -> Config {
        match path() {
            Some(path) => Config::load_from(&path),
            None => Config::default(),
        }
    }

    /// A missing or unreadable file is an empty config, never an error: a
    /// broken config should not stop you seeing the board.
    pub fn load_from(path: &Path) -> Config {
        let Ok(text) = crate::eol::read(path) else {
            return Config::default();
        };
        let mut entries = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                entries.push((key.trim().to_string(), value.trim().to_string()));
            }
        }
        Config { entries }
    }

    /// Every key set, in file order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(k, _)| k.as_str())
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    /// Set a key, keeping its position if it was already there so the file
    /// doesn't reshuffle every time it's written.
    pub fn set(&mut self, key: &str, value: &str) {
        match self.entries.iter_mut().find(|(name, _)| name == key) {
            Some((_, existing)) => *existing = value.to_string(),
            None => self.entries.push((key.to_string(), value.to_string())),
        }
    }

    /// Removes a key. Returns whether it was there.
    pub fn remove(&mut self, key: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(name, _)| name != key);
        self.entries.len() != before
    }

    /// Renames a key in place (its line keeps its position), for a setting
    /// whose name has changed. If both names are present, the old one is
    /// dropped and the new one's value kept.
    pub fn rename_key(&mut self, old: &str, new: &str) {
        if self.get(new).is_some() {
            self.entries.retain(|(name, _)| name != old);
        } else if let Some((name, _)) = self.entries.iter_mut().find(|(name, _)| name == old) {
            *name = new.to_string();
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = path().ok_or_else(|| anyhow::anyhow!("no config directory"))?;
        self.save_to(&path)
    }

    /// Writes via a temp file and a rename, so a reader never sees a partial
    /// file and two concurrent writers can't interleave — the file this
    /// backs may be read and written by more than one process (the CLI and
    /// the GUI, or two clones of a git-tracked board-local file).
    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut text = String::new();
        for (key, value) in &self.entries {
            text.push_str(&format!("{key} = {value}\n"));
        }
        let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
        tmp_name.push(format!(".tmp.{}", std::process::id()));
        let tmp = path.with_file_name(tmp_name);
        fs::write(&tmp, text)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// `config` in the config folder ([`crate::places::config`]).
pub fn path() -> Option<PathBuf> {
    crate::places::config().map(|dir| dir.join("config"))
}

/// The app-tier precedence for a knob that has no CLI flag of its own:
/// environment variable, then the settings file, then whatever built-in
/// default the caller falls back to. (`docs/adr/0005-settings-state-and-precedence.md`
/// settles the full order, `flag > env > board-local settings > global
/// settings > built-in default`; this covers the env/file half of it — a
/// flag, where one exists, is read by the caller before ever calling this.)
pub fn resolved(env_var: &str, config: &Config, config_key: &str) -> Option<String> {
    crate::renamed::var(env_var)
        .ok()
        .or_else(|| config.get(config_key).map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_an_empty_config() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load_from(&dir.path().join("nope"));
        assert_eq!(config, Config::default());
        assert_eq!(config.get("anything"), None);
    }

    #[test]
    fn writing_one_key_keeps_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        fs::write(&path, "theme = nord\ngui.font = large\n").unwrap();

        // One front end saving its theme must not delete another's settings.
        let mut config = Config::load_from(&path);
        config.set("theme", "gruvbox");
        config.save_to(&path).unwrap();

        let reread = Config::load_from(&path);
        assert_eq!(reread.get("theme"), Some("gruvbox"));
        assert_eq!(
            reread.get("gui.font"),
            Some("large"),
            "the GUI key survived"
        );
    }

    #[test]
    fn comments_and_nonsense_are_ignored_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        fs::write(&path, "# a note\nnonsense\ngui.theme = dark\n").unwrap();
        assert_eq!(Config::load_from(&path).get("gui.theme"), Some("dark"));
    }

    #[test]
    fn a_new_key_appends_and_survives_a_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kanrite/config");
        let mut config = Config::default();
        config.set("gui.font", "medium");
        config.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path).get("gui.font"), Some("medium"));
    }

    #[test]
    fn saving_leaves_no_stray_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        let mut config = Config::default();
        config.set("theme", "nord");
        config.save_to(&path).unwrap();

        let entries: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("config")]);
    }

    #[test]
    fn resolved_prefers_the_environment_over_the_file() {
        let mut config = Config::default();
        config.set("editor", "vim");
        // SAFETY: single-threaded test, no other thread reads this var.
        unsafe { std::env::set_var("RITE_TEST_RESOLVED_EDITOR", "nano") };
        assert_eq!(
            resolved("RITE_TEST_RESOLVED_EDITOR", &config, "editor"),
            Some("nano".to_string())
        );
        unsafe { std::env::remove_var("RITE_TEST_RESOLVED_EDITOR") };
    }

    #[test]
    fn resolved_falls_back_to_the_file_then_to_nothing() {
        let mut config = Config::default();
        config.set("editor", "vim");
        unsafe { std::env::remove_var("RITE_TEST_RESOLVED_EDITOR_2") };
        assert_eq!(
            resolved("RITE_TEST_RESOLVED_EDITOR_2", &config, "editor"),
            Some("vim".to_string())
        );
        assert_eq!(
            resolved("RITE_TEST_RESOLVED_EDITOR_2", &config, "nothing"),
            None
        );
    }
}
