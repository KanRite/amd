//! The user's home folder, for the places rite keeps per-machine files
//! (`~/.config/agile-md`, `~/.local/state/agile-md`, …).
//!
//! `HOME` is what Unix sets, and what Git Bash sets on Windows. A plain
//! Windows shell (PowerShell, cmd) sets only `USERPROFILE`, so a lookup of
//! `HOME` alone found no config directory there: `rite register` failed and
//! the registry read as empty (ticket 324).

use std::ffi::OsString;
use std::path::PathBuf;

/// `$HOME`, else `$USERPROFILE`; an empty value counts as unset.
pub fn dir() -> Option<PathBuf> {
    pick(std::env::var_os("HOME"), std::env::var_os("USERPROFILE"))
}

fn pick(home: Option<OsString>, profile: Option<OsString>) -> Option<PathBuf> {
    home.filter(|v| !v.is_empty())
        .or(profile.filter(|v| !v.is_empty()))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(s: &str) -> Option<OsString> {
        Some(OsString::from(s))
    }

    #[test]
    fn home_wins_when_both_are_set() {
        assert_eq!(pick(os("/h"), os("C:\\u")), Some(PathBuf::from("/h")));
    }

    #[test]
    fn userprofile_is_used_when_home_is_missing_or_empty() {
        assert_eq!(pick(None, os("C:\\u")), Some(PathBuf::from("C:\\u")));
        assert_eq!(pick(os(""), os("C:\\u")), Some(PathBuf::from("C:\\u")));
    }

    #[test]
    fn neither_set_is_none() {
        assert_eq!(pick(None, None), None);
        assert_eq!(pick(os(""), os("")), None);
    }
}
