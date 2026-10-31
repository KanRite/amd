//! Comparing project and repository names without case (ticket 338).
//!
//! Whether a folder name is case-sensitive depends on the disk, not on the
//! operating system: NTFS can turn it on per folder, APFS and HFS+ volumes
//! can be formatted either way, ext4 has casefold folders, and an SMB or USB
//! drive mounted anywhere can go either way. GitHub's repository names are
//! case-insensitive too. So rite treats `KanRite` and `kanrite` as one name
//! on every disk, rather than leaving it to whatever the file system says:
//! [`same`] is the one comparison, [`folder_named`] finds a folder by it, and
//! two folders that differ only in case are never quietly mixed up.
//!
//! Names stay as they were typed or stored: nothing is lowercased on disk,
//! and what people read is the display name (ticket 326).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

/// A name with case taken out, for comparing.
pub fn key(name: &str) -> String {
    name.to_lowercase()
}

/// Whether two project or repository names are the same name.
pub fn same(a: &str, b: &str) -> bool {
    a == b || key(a) == key(b)
}

/// The folders directly in `dir` named `name`, ignoring case, exact match
/// first. Never reads inside them.
fn matching(dir: &Path, name: &str) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| same(&entry.file_name().to_string_lossy(), name))
        .map(|entry| entry.path())
        .collect();
    found.sort_by_key(|path| path.file_name().is_none_or(|n| *n != *name));
    found
}

/// The folder in `dir` called `name`: the exact name if it's there, else one
/// that differs only in case. `Ok(None)` when there is none. Two or more
/// folders that differ only in case (possible on a case-sensitive disk) are
/// an error naming them, never a guess.
pub fn folder_named(dir: &Path, name: &str) -> Result<Option<PathBuf>> {
    let found = matching(dir, name);
    match found.as_slice() {
        [] => Ok(None),
        [only] => Ok(Some(only.clone())),
        many => bail!(
            "{} differ only in case: keep one of them, so a project name can't mean either",
            many.iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(" and ")
        ),
    }
}

/// [`folder_named`] where an error counts as none: for a lookup that only
/// wants to know whether a clone is there.
pub fn find_folder(dir: &Path, name: &str) -> Option<PathBuf> {
    folder_named(dir, name).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_the_same_whatever_their_case() {
        assert!(same("KanRite", "kanrite"));
        assert!(same("kanrite", "kanrite"));
        assert!(same("KANRITE-tickets", "kanrite-Tickets"));
        assert!(!same("kanrite", "kanrite2"));
        assert!(!same("kan-rite", "kanrite"));
        // Not only ASCII.
        assert!(same("Ärger", "ärger"));
    }

    #[test]
    fn a_folder_is_found_by_its_name_in_any_case() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(folder_named(dir.path(), "acme-tickets").unwrap(), None);
        fs::create_dir(dir.path().join("Acme-Tickets")).unwrap();
        let found = folder_named(dir.path(), "acme-tickets").unwrap().unwrap();
        assert_eq!(found.file_name().unwrap(), "Acme-Tickets");
        // Exact name given: the same folder.
        let found = folder_named(dir.path(), "Acme-Tickets").unwrap().unwrap();
        assert_eq!(found.file_name().unwrap(), "Acme-Tickets");
        assert_eq!(find_folder(dir.path(), "other"), None);
    }

    /// Two folders that differ only in case can exist only on a
    /// case-sensitive disk; where the disk ignores case this can't be set up
    /// and the test has nothing to check (Linux CI is where it runs).
    #[test]
    fn two_folders_that_differ_only_in_case_are_refused_not_picked() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("acme-tickets")).unwrap();
        // A disk that ignores case says the second one is there already.
        if fs::create_dir(dir.path().join("Acme-Tickets")).is_err() {
            return;
        }
        let err = folder_named(dir.path(), "ACME-TICKETS")
            .unwrap_err()
            .to_string();
        assert!(err.contains("differ only in case"), "{err}");
        assert_eq!(find_folder(dir.path(), "acme-tickets"), None);
    }
}
