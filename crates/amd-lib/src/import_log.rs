//! `import.log` (ticket 173): what an import left out, and why, appended
//! in the folder the import ran from, a dated section per run. It's never
//! committed: the file name goes into the repository's own
//! `.git/info/exclude`, so it isn't a change, needs no `.gitignore` commit,
//! and auto-commit never sweeps it in.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::git;

/// The log's file name.
pub const FILE: &str = "import.log";

/// Appends a section to `dir/import.log`: when, `command`, then one line
/// per thing left out. Nothing is written when `lines` is empty. Returns
/// the log's path when it wrote one.
pub fn append(dir: &Path, command: &str, lines: &[String]) -> Result<Option<PathBuf>> {
    if lines.is_empty() {
        return Ok(None);
    }
    let path = dir.join(FILE);
    let (_, when) = crate::templates::today();
    let mut text = format!("## {when}  {command}\n\n");
    for line in lines {
        text.push_str(&format!("- {line}\n"));
    }
    text.push('\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("writing {}", path.display()))?;
    file.write_all(text.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    exclude(dir);
    Ok(Some(path))
}

/// Puts `import.log` in the repository's local exclude list, once. Best
/// effort: a folder outside git, or an exclude file that can't be written,
/// only means the log shows as untracked.
fn exclude(dir: &Path) {
    let Ok(rel) = git::checked(dir, &["rev-parse", "--git-path", "info/exclude"]) else {
        return;
    };
    let exclude = if Path::new(&rel).is_absolute() {
        PathBuf::from(rel)
    } else {
        dir.join(rel)
    };
    let current = crate::eol::read(&exclude).unwrap_or_default();
    if current.lines().any(|l| l.trim() == FILE) {
        return;
    }
    if let Some(parent) = exclude.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let sep = if current.is_empty() || current.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let _ = fs::write(
        &exclude,
        format!("{current}{sep}# amd's import notes (ticket 173)\n{FILE}\n"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["init", "-q"])
            .status()
            .unwrap()
            .success();
        assert!(ok);
        dir
    }

    #[test]
    fn nothing_left_out_writes_nothing() {
        let dir = repo();
        assert_eq!(append(dir.path(), "amd import x", &[]).unwrap(), None);
        assert!(!dir.path().join(FILE).exists());
    }

    #[test]
    fn sections_are_appended_and_the_log_is_excluded_once() {
        let dir = repo();
        let sub = dir.path().join("docs");
        fs::create_dir_all(&sub).unwrap();
        append(
            &sub,
            "amd import documents a",
            &["a.md: already here".into()],
        )
        .unwrap();
        append(
            &sub,
            "amd import documents b",
            &["b.md: already here".into()],
        )
        .unwrap();
        let log = crate::eol::read(sub.join(FILE)).unwrap();
        assert!(
            log.contains("amd import documents a\n\n- a.md: already here\n"),
            "{log}"
        );
        assert!(log.contains("- b.md: already here"), "{log}");
        let exclude = crate::eol::read(dir.path().join(".git/info/exclude")).unwrap();
        assert_eq!(
            exclude.lines().filter(|l| *l == FILE).count(),
            1,
            "{exclude}"
        );
        // git doesn't see it.
        let status = git::checked(dir.path(), &["status", "--porcelain"]).unwrap();
        assert!(status.is_empty(), "{status}");
    }
}
