//! Noticing that riteui's own program was replaced while it runs (ticket
//! 208): by `rite update` in a terminal, the installer run again, or a
//! package manager. The running process carries on as the old version
//! until it's restarted (the page is inside the binary, so a webview reload
//! isn't enough), so the page offers a Restart.
//!
//! Recorded once at start: the path (afterwards `current_exe` can name a
//! deleted file), and the file's inode, modified time and size. A check is
//! one `stat`. When the file differs, it has to stay the same for
//! [`SETTLE`] before it's believed (an install mid-copy is never offered),
//! and then its `--version` must name a different version: a `touch`, a
//! backup restore or a local rebuild of the same version only becomes the
//! new baseline, with no bar. Linux and macOS alike.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// How long a changed file must stay unchanged before it's believed.
pub const SETTLE: Duration = Duration::from_secs(2);

/// A file's identity, as far as noticing a replacement goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub inode: u64,
    pub modified: Option<std::time::SystemTime>,
    pub len: u64,
}

impl Stamp {
    /// The file at `path` now, or `None` while it isn't there (mid-swap).
    pub fn of(path: &Path) -> Option<Stamp> {
        let meta = std::fs::metadata(path).ok()?;
        #[cfg(unix)]
        let inode = std::os::unix::fs::MetadataExt::ino(&meta);
        #[cfg(not(unix))]
        let inode = 0;
        Some(Stamp {
            inode,
            modified: meta.modified().ok(),
            len: meta.len(),
        })
    }
}

/// What the watch has seen.
#[derive(Debug)]
pub struct Watch {
    path: PathBuf,
    running: String,
    baseline: Option<Stamp>,
    /// A change seen, and when, waiting to settle.
    pending: Option<(Stamp, Instant)>,
    /// The new version, once found: said on every check after.
    found: Option<String>,
}

impl Watch {
    /// Watches `path`, the program running as `running` (its version).
    pub fn new(path: PathBuf, running: &str) -> Watch {
        let baseline = Stamp::of(&path);
        Watch {
            path,
            running: running.to_string(),
            baseline,
            pending: None,
            found: None,
        }
    }

    /// The version the program on disk is now, when it's settled and isn't
    /// the one running. `stamp` is the file now, `version_of` asks it.
    pub fn step(
        &mut self,
        stamp: Option<Stamp>,
        now: Instant,
        version_of: impl FnOnce() -> Option<String>,
    ) -> Option<String> {
        if self.found.is_some() {
            return self.found.clone();
        }
        let stamp = stamp?;
        if Some(stamp) == self.baseline {
            self.pending = None;
            return None;
        }
        match self.pending {
            Some((seen, since)) if seen == stamp => {
                if now.duration_since(since) < SETTLE {
                    return None;
                }
            }
            // A new change, or one still being written: start waiting.
            _ => {
                self.pending = Some((stamp, now));
                return None;
            }
        }
        self.pending = None;
        match version_of() {
            Some(version) if version != self.running => {
                self.found = Some(version);
                self.found.clone()
            }
            // The same version (a touch, a rebuild), or it won't say: the
            // file as it is now is the new baseline.
            _ => {
                self.baseline = Some(stamp);
                None
            }
        }
    }

    /// [`step`](Self::step) against the real file and clock.
    pub fn check(&mut self) -> Option<String> {
        let path = self.path.clone();
        self.step(Stamp::of(&path), Instant::now(), || version_of(&path))
    }
}

/// `riteui --version` is `riteui 4.6.0`: the version, or `None` if it
/// doesn't run or says something else.
fn version_of(path: &Path) -> Option<String> {
    let out = Command::new(path).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .nth(1)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn stamp(inode: u64, len: u64) -> Option<Stamp> {
        Some(Stamp {
            inode,
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(inode)),
            len,
        })
    }

    fn watch() -> Watch {
        let mut watch = Watch::new(PathBuf::from("/nonexistent/riteui"), "4.5.0");
        watch.baseline = stamp(1, 100);
        watch
    }

    #[test]
    fn an_unchanged_file_says_nothing_and_asks_nothing() {
        let mut w = watch();
        let t = Instant::now();
        assert_eq!(w.step(stamp(1, 100), t, || panic!("not asked")), None);
        // Gone for a moment (mid-swap): nothing either.
        assert_eq!(w.step(None, t, || panic!("not asked")), None);
    }

    #[test]
    fn a_replaced_file_is_offered_once_it_has_settled() {
        let mut w = watch();
        let t = Instant::now();
        assert_eq!(w.step(stamp(2, 50), t, || panic!("too soon")), None);
        // Still being written: the wait starts again.
        assert_eq!(
            w.step(stamp(2, 90), t + SETTLE, || panic!("still changing")),
            None
        );
        assert_eq!(
            w.step(stamp(2, 90), t + SETTLE, || panic!("too soon")),
            None
        );
        let later = t + SETTLE + SETTLE;
        assert_eq!(
            w.step(stamp(2, 90), later, || Some("4.6.0".into())),
            Some("4.6.0".into())
        );
        // Said again on every check, without asking again.
        assert_eq!(
            w.step(stamp(2, 90), later, || panic!("asked twice")),
            Some("4.6.0".into())
        );
    }

    #[test]
    fn a_touch_or_rebuild_of_the_same_version_becomes_the_baseline() {
        let mut w = watch();
        let t = Instant::now();
        w.step(stamp(3, 100), t, || None);
        assert_eq!(
            w.step(stamp(3, 100), t + SETTLE, || Some("4.5.0".into())),
            None
        );
        // Not asked again for the same file.
        assert_eq!(
            w.step(stamp(3, 100), t + SETTLE * 3, || panic!("asked again")),
            None
        );
        // A file that won't say its version is no update either.
        w.step(stamp(4, 10), t, || None);
        assert_eq!(w.step(stamp(4, 10), t + SETTLE, || None), None);
    }

    /// A shell script stands in for riteui, so Unix only: Windows can't run
    /// one, and there the real check runs `riteui.exe` (ticket 280).
    #[cfg(unix)]
    #[test]
    fn it_reads_the_version_a_real_program_reports() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("riteui");
        std::fs::write(&bin, "#!/bin/sh\necho \"riteui 4.6.0\"\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(version_of(&bin).as_deref(), Some("4.6.0"));
        assert_eq!(version_of(&dir.path().join("gone")), None);
    }
}
