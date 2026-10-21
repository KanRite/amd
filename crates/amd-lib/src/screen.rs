//! What amdui is showing, for an AI assistant to read (ticket 186).
//!
//! amdui writes a small note whenever its view changes: which project,
//! which screen, the open ticket, document or idea, and the board's
//! filters. `amd mcp`'s `screen` tool reads it, so "move this to doing"
//! knows what "this" is. Names and paths only: the content is fetched
//! through the other tools, each of which the AI tool shows.
//!
//! One note per project, keyed by the project's own board repository, in
//! `$XDG_STATE_HOME/agile-md/screen/`: machine-local, never committed. With
//! two windows on one project the latest write wins; a note whose window
//! has closed (its `pid` isn't running) is ignored.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// What's on screen in one amdui window.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// The amdui process that wrote it.
    pub pid: u32,
    /// When, as RFC 3339.
    pub updated: String,
    pub project: String,
    #[serde(flatten)]
    pub view: View,
}

/// The part of the note the page knows: everything but who wrote it and
/// when.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    /// `home`, `board`, `documents`, `ideas` or `roadmaps`.
    pub screen: String,
    /// The ticket open in the editor.
    pub ticket: Option<Place>,
    /// The document open in Documents.
    pub document: Option<Place>,
    /// The idea open in Ideas.
    pub idea: Option<Place>,
    /// The epic or sprint opened as a column beside Backlogs.
    pub epic: Option<Place>,
    /// The board's search box, when it has text.
    pub search: String,
    /// The repository filter: tickets about this repository only.
    pub about: Option<String>,
    /// The assignee filter: a name, `(nobody)`, or empty for everyone.
    pub assignee: String,
}

/// Something in a repository: a ticket's file name, a document's path, an
/// idea's path or an epic's name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    pub repo: String,
    pub path: String,
}

/// Where the notes live.
fn dir() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .map(|state| state.join("agile-md/screen"))
}

/// The note for the project whose own board is in `repo`: its name, plus a
/// hash of the path, since two clones can share a name.
pub fn path(repo: &Path) -> Option<PathBuf> {
    let name = repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Some(dir()?.join(format!("{name}-{:016x}.json", fnv1a(repo))))
}

/// FNV-1a over the path's bytes: stable across builds, unlike the standard
/// hasher, so amdui and `amd` agree whichever was built when.
fn fnv1a(path: &Path) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.as_os_str().as_encoded_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Records what this process shows for the project in `repo`.
pub fn write(repo: &Path, project: &str, view: View) -> Result<()> {
    let Some(path) = path(repo) else {
        return Ok(());
    };
    let note = Note {
        pid: std::process::id(),
        updated: time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
        project: project.to_string(),
        view,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("making {}", parent.display()))?;
    }
    // Written beside it and renamed, so a reader never sees half a note.
    let part = path.with_extension("json.part");
    std::fs::write(&part, serde_json::to_vec_pretty(&note)?)
        .with_context(|| format!("writing {}", part.display()))?;
    std::fs::rename(&part, &path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Takes this process's note for `repo` away, when its window closes.
pub fn clear(repo: &Path) {
    if let Some(path) = path(repo)
        && let Some(note) = read_file(&path)
        && note.pid == std::process::id()
    {
        let _ = std::fs::remove_file(path);
    }
}

/// What amdui shows for the project in `repo`, if a window that's still
/// open says.
pub fn read(repo: &Path) -> Option<Note> {
    read_file(&path(repo)?).filter(|note| running(note.pid))
}

fn read_file(path: &Path) -> Option<Note> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Whether a process is running: `kill -0` asks without sending anything,
/// on Linux and macOS alike.
#[cfg(unix)]
fn running(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The same on Windows (ticket 279), which has no `kill`: `tasklist`
/// filtered to the one id lists it, quoted, only while it runs (otherwise
/// it says there are no tasks, and still succeeds).
#[cfg(windows)]
fn running(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .stderr(std::process::Stdio::null())
        .output()
        .is_ok_and(|out| listed(&String::from_utf8_lossy(&out.stdout), pid))
}

/// Whether `tasklist /FO CSV` output lists `pid`: its second field.
#[cfg_attr(not(windows), allow(dead_code))]
fn listed(csv: &str, pid: u32) -> bool {
    let quoted = format!("\"{pid}\"");
    csv.lines()
        .any(|line| line.split(',').nth(1) == Some(quoted.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasklist_output_lists_a_process_only_by_its_own_id() {
        let out = "\"amdui.exe\",\"4242\",\"Console\",\"1\",\"80,512 K\"\r\n";
        assert!(listed(out, 4242));
        assert!(!listed(out, 424), "not a prefix match");
        assert!(!listed(
            "INFO: No tasks are running which match the specified criteria.\r\n",
            4242
        ));
    }

    #[test]
    fn a_note_round_trips_and_is_keyed_by_the_repository() {
        let state = tempfile::tempdir().unwrap();
        // Safety: this test alone sets XDG_STATE_HOME, before reading it.
        unsafe { std::env::set_var("XDG_STATE_HOME", state.path()) };
        let (a, b) = (Path::new("/x/acme-tickets"), Path::new("/y/acme-tickets"));
        assert_ne!(path(a), path(b), "two clones with one name");
        assert!(
            path(a)
                .unwrap()
                .starts_with(state.path().join("agile-md/screen"))
        );
        assert_eq!(read(a), None);

        let view = View {
            screen: "board".into(),
            ticket: Some(Place {
                repo: "acme-tickets".into(),
                path: "007-checkout.md".into(),
            }),
            ..View::default()
        };
        write(a, "acme", view.clone()).unwrap();
        let note = read(a).unwrap();
        assert_eq!((note.project.as_str(), &note.view), ("acme", &view));
        assert_eq!(note.pid, std::process::id());
        assert_eq!(read(b), None);

        // A window that has gone leaves a note nobody should trust.
        let mut stale = note.clone();
        stale.pid = u32::MAX - 1;
        std::fs::write(path(a).unwrap(), serde_json::to_vec(&stale).unwrap()).unwrap();
        assert_eq!(read(a), None);

        write(a, "acme", view).unwrap();
        clear(a);
        assert!(!path(a).unwrap().exists());
    }
}
