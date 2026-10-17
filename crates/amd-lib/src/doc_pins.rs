//! Your pinned documents (ticket 232): the documents you're writing or
//! working from, first in your Documents list. Pins are per person and
//! follow you across machines, so they're kept in the project's own
//! documents repository, one file each: `.agile-md-pins/<your email>`,
//! one `<repository>/<path>` a line. A file of its own per person means
//! two people pinning never touch the same lines. "You" is git's
//! `user.email` in that clone. A pin can name the shared documents
//! repository too; one whose document has gone is still listed, so it can
//! be unpinned.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::documents::{Restored, commit_change, ensure_clean};
use crate::git;

/// The folder holding everyone's pins, at the top of the project's own
/// documents repository. A dot-folder, so the Documents list skips it.
pub const FOLDER: &str = ".agile-md-pins";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    /// The documents repository's name.
    pub source: String,
    /// Relative to that repository.
    pub path: String,
}

impl Pin {
    fn line(&self) -> String {
        format!("{}/{}", self.source, self.path)
    }

    fn parse(line: &str) -> Option<Pin> {
        let (source, path) = line.trim().split_once('/')?;
        (!source.is_empty() && !path.is_empty()).then(|| Pin {
            source: source.to_string(),
            path: path.to_string(),
        })
    }
}

/// Who "you" are in the documents clone `root`: git's `user.email`.
pub fn me(root: &Path) -> Result<String> {
    git::config(root, "user.email")
        .map(|e| e.trim().to_lowercase())
        .filter(|e| !e.is_empty())
        .ok_or_else(|| anyhow::anyhow!("set git's user.email to pin documents"))
}

/// Your pins file, relative to the repository: the email with anything but
/// letters, digits and `@._-` made a `-`.
fn file_of(email: &str) -> String {
    let key: String = email
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "@._-".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("{FOLDER}/{key}")
}

fn read_file(path: &Path) -> Vec<Pin> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(Pin::parse)
        .collect()
}

fn render(pins: &[Pin]) -> String {
    pins.iter().map(|p| p.line() + "\n").collect()
}

/// `email`'s pins, in the order they were pinned.
pub fn pins(root: &Path, email: &str) -> Vec<Pin> {
    read_file(&root.join(file_of(email)))
}

/// Pins (or with `on` false, unpins) `source`/`path` for `email`, committed
/// to the documents repository `root` and pushed (ADR 0007).
pub fn set(root: &Path, email: &str, source: &str, path: &str, on: bool) -> Result<Restored> {
    let file = file_of(email);
    let pin = Pin {
        source: source.to_string(),
        path: path.to_string(),
    };
    if Pin::parse(&pin.line()).as_ref() != Some(&pin) || path.contains('\n') {
        bail!("{source}/{path} can't be pinned");
    }
    let had = pins(root, email).contains(&pin);
    if on && had {
        bail!("{} is pinned already", pin.line());
    }
    if !on && !had {
        bail!("{} isn't pinned", pin.line());
    }
    ensure_clean(root, &[&file])?;
    let message = format!("{}: {}", if on { "pin" } else { "unpin" }, pin.line());
    commit_change(root, &message, || {
        let full = root.join(&file);
        // Read again: catching up may have brought pins from another machine.
        let mut list = read_file(&full);
        list.retain(|p| *p != pin);
        if on {
            list.push(pin.clone());
        }
        if list.is_empty() {
            if full.exists() {
                fs::remove_file(&full).with_context(|| format!("removing {file}"))?;
            }
            // Gone when nobody has pins left; fine if someone still does.
            let _ = fs::remove_dir(root.join(FOLDER));
        } else {
            fs::create_dir_all(root.join(FOLDER)).with_context(|| format!("creating {FOLDER}"))?;
            fs::write(&full, render(&list)).with_context(|| format!("writing {file}"))?;
        }
        Ok(vec![PathBuf::from(&file)])
    })
}

/// Everyone's pins of `source`/`from`, pointed at `to` instead: each pins
/// file that changes and its new text, for a move to write (relative to
/// `root`, the repository holding the pins).
pub fn moved(root: &Path, source: &str, from: &str, to: &str) -> Vec<(String, String)> {
    let Ok(entries) = fs::read_dir(root.join(FOLDER)) else {
        return Vec::new();
    };
    let mut changes = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let mut list = read_file(&path);
        let mut changed = false;
        for pin in &mut list {
            if pin.source == source && pin.path == from {
                pin.path = to.to_string();
                changed = true;
            }
        }
        if changed {
            let name = entry.file_name().to_string_lossy().into_owned();
            changes.push((format!("{FOLDER}/{name}"), render(&list)));
        }
    }
    changes.sort();
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("acme-documents");
        fs::create_dir_all(root.join("docs")).unwrap();
        git::checked(&root, &["init", "-q", "-b", "main"]).unwrap();
        for (k, v) in [
            ("user.email", "Sam@Acme.io"),
            ("user.name", "Sam Lee"),
            ("commit.gpgsign", "false"),
        ] {
            git::checked(&root, &["config", k, v]).unwrap();
        }
        fs::write(root.join("docs/guide.md"), "# Guide\n").unwrap();
        git::checked(&root, &["add", "-A"]).unwrap();
        git::checked(&root, &["commit", "-q", "-m", "seed"]).unwrap();
        (dir, root)
    }

    #[test]
    fn pins_are_yours_kept_in_order_and_committed() {
        let (_dir, root) = repo();
        let me = me(&root).unwrap();
        assert_eq!(me, "sam@acme.io");
        set(&root, &me, "acme-documents", "docs/guide.md", true).unwrap();
        set(&root, &me, "acme-handbook", "people/leave.md", true).unwrap();
        let list = pins(&root, &me);
        assert_eq!(
            list.iter().map(Pin::line).collect::<Vec<_>>(),
            [
                "acme-documents/docs/guide.md",
                "acme-handbook/people/leave.md"
            ]
        );
        assert!(pins(&root, "priya@acme.io").is_empty(), "nobody else's");
        assert!(set(&root, &me, "acme-documents", "docs/guide.md", true).is_err());
        let status = git::checked(&root, &["status", "--porcelain"]).unwrap();
        assert!(status.trim().is_empty(), "committed: {status}");
        let log = git::checked(&root, &["log", "-1", "--format=%s"]).unwrap();
        assert_eq!(log.trim(), "pin: acme-handbook/people/leave.md");
    }

    #[test]
    fn unpinning_the_last_removes_the_file() {
        let (_dir, root) = repo();
        set(
            &root,
            "sam@acme.io",
            "acme-documents",
            "docs/guide.md",
            true,
        )
        .unwrap();
        set(
            &root,
            "sam@acme.io",
            "acme-documents",
            "docs/guide.md",
            false,
        )
        .unwrap();
        assert!(!root.join(file_of("sam@acme.io")).exists());
        assert!(
            set(
                &root,
                "sam@acme.io",
                "acme-documents",
                "docs/guide.md",
                false
            )
            .is_err()
        );
    }

    #[test]
    fn a_move_repoints_everyones_pins_of_that_document_only() {
        let (_dir, root) = repo();
        set(
            &root,
            "sam@acme.io",
            "acme-documents",
            "docs/guide.md",
            true,
        )
        .unwrap();
        set(
            &root,
            "priya@acme.io",
            "acme-documents",
            "docs/guide.md",
            true,
        )
        .unwrap();
        set(
            &root,
            "priya@acme.io",
            "acme-handbook",
            "docs/guide.md",
            true,
        )
        .unwrap();
        let changes = moved(&root, "acme-documents", "docs/guide.md", "guide.md");
        assert_eq!(changes.len(), 2);
        let priya = &changes.iter().find(|(f, _)| f.contains("priya")).unwrap().1;
        assert_eq!(
            priya, "acme-documents/guide.md\nacme-handbook/docs/guide.md\n",
            "another repository's document of the same path stays"
        );
    }

    #[test]
    fn odd_emails_make_plain_file_names() {
        assert_eq!(file_of("A.B+x@Acme.io"), ".agile-md-pins/a.b-x@acme.io");
    }
}
