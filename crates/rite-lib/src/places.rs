//! The folders amd keeps per-machine files in (ticket 310): config
//! (`~/.config/kanrite`: settings, the registry, project trust), state
//! (`~/.local/state/kanrite`: the update check, screen notes, recovered
//! tickets) and data (`~/.local/share/kanrite`: speech models and Piper).
//! `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and `XDG_DATA_HOME` move them, when
//! they're absolute paths, as the XDG spec says.
//!
//! Before 5.0 each was called `agile-md`. [`move_old_folders`], run first
//! thing by `rite` and `riteui`, renames an `agile-md` folder to `kanrite`
//! and leaves a symlink at the old name, so a 4.x build on the same machine
//! still finds its config. Renamed, never copied: the speech models are
//! large. Until it has run (or where it couldn't), [`config`], [`state`]
//! and [`data`] use the old folder when it's the only one there, so nothing
//! is ever lost track of.

use std::fs;
use std::path::{Path, PathBuf};

/// The folders' name since 5.0.
pub const NAME: &str = "kanrite";
/// Their name before (ticket 310).
pub const OLD_NAME: &str = "agile-md";

/// `$XDG_CONFIG_HOME/kanrite`, else `~/.config/kanrite`.
pub fn config() -> Option<PathBuf> {
    base("XDG_CONFIG_HOME", ".config").map(|base| within(&base))
}

/// `$XDG_STATE_HOME/kanrite`, else `~/.local/state/kanrite`.
pub fn state() -> Option<PathBuf> {
    base("XDG_STATE_HOME", ".local/state").map(|base| within(&base))
}

/// `$XDG_DATA_HOME/kanrite`, else `~/.local/share/kanrite`.
pub fn data() -> Option<PathBuf> {
    base("XDG_DATA_HOME", ".local/share").map(|base| within(&base))
}

/// The folder an XDG variable names, if absolute, else `~/<fallback>`.
fn base(var: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| crate::home::dir().map(|home| home.join(fallback)))
}

/// `base/kanrite`, or `base/agile-md` while that's the only one there.
fn within(base: &Path) -> PathBuf {
    let new = base.join(NAME);
    let old = base.join(OLD_NAME);
    if !new.exists() && old.is_dir() {
        old
    } else {
        new
    }
}

/// Renames each `agile-md` folder that has no `kanrite` beside it, leaving
/// a symlink at the old name (on Windows, which had no 4.x release, just
/// the rename). Returns the folders moved, as `(old, new)`. A failure
/// leaves that folder where it was, still used ([`within`]).
pub fn move_old_folders() -> Vec<(PathBuf, PathBuf)> {
    let mut moved = Vec::new();
    for (var, fallback) in [
        ("XDG_CONFIG_HOME", ".config"),
        ("XDG_STATE_HOME", ".local/state"),
        ("XDG_DATA_HOME", ".local/share"),
    ] {
        if let Some(base) = base(var, fallback)
            && let Some(pair) = move_in(&base)
        {
            moved.push(pair);
        }
    }
    moved
}

/// [`move_old_folders`], saying on stderr what moved. `rite` and `riteui`
/// call it first thing.
pub fn move_old_folders_and_say() {
    for (old, new) in move_old_folders() {
        eprintln!(
            "moved {} to {} (the old name stays as a link, for older versions)",
            old.display(),
            new.display()
        );
    }
}

/// [`move_old_folders`] for one base folder.
fn move_in(base: &Path) -> Option<(PathBuf, PathBuf)> {
    let old = base.join(OLD_NAME);
    let new = base.join(NAME);
    let is_real_dir = fs::symlink_metadata(&old).is_ok_and(|m| m.is_dir());
    if !is_real_dir || fs::symlink_metadata(&new).is_ok() {
        return None;
    }
    fs::rename(&old, &new).ok()?;
    #[cfg(unix)]
    if std::os::unix::fs::symlink(NAME, &old).is_err() {
        // Without the link a 4.x build would start afresh: put it back.
        let _ = fs::rename(&new, &old);
        return None;
    }
    Some((old, new))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_folder_is_used_until_it_is_moved_then_linked() {
        let base = tempfile::tempdir().unwrap();
        let old = base.path().join(OLD_NAME);
        let new = base.path().join(NAME);
        // Nothing yet: the new name.
        assert_eq!(within(base.path()), new);
        // Only the old one: used as it is.
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("config"), "zoom = 120\n").unwrap();
        assert_eq!(within(base.path()), old);
        // Moved: the new name, with the old one a link to it.
        assert_eq!(move_in(base.path()), Some((old.clone(), new.clone())));
        assert_eq!(within(base.path()), new);
        assert_eq!(
            fs::read_to_string(new.join("config")).unwrap(),
            "zoom = 120\n"
        );
        #[cfg(unix)]
        {
            assert!(fs::symlink_metadata(&old).unwrap().is_symlink());
            assert_eq!(
                fs::read_to_string(old.join("config")).unwrap(),
                "zoom = 120\n"
            );
        }
        // Once only.
        assert_eq!(move_in(base.path()), None);
    }

    #[test]
    fn both_folders_there_are_left_alone() {
        let base = tempfile::tempdir().unwrap();
        fs::create_dir_all(base.path().join(OLD_NAME)).unwrap();
        fs::create_dir_all(base.path().join(NAME)).unwrap();
        assert_eq!(move_in(base.path()), None);
        assert_eq!(within(base.path()), base.path().join(NAME));
        assert!(base.path().join(OLD_NAME).is_dir());
    }
}
