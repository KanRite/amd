//! The rename of `amd` to `rite` and `amdui` to `riteui` (ticket 309).
//!
//! Through the 5.x releases the old names keep working: `amd` and `amdui`
//! are shims (a symlink, or on Windows a copy) that run the new programs,
//! which say once a day that they have a new name ([`notice`]). The
//! environment variables follow: `RITE_*` is read first, then the `AMD_*`
//! of the same name ([`var`]), so a script or CI job written for 4.x
//! behaves the same.

use std::env::VarError;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

/// How often a shim says it is one: once a day.
const NOTICE_EVERY: i64 = 24 * 60 * 60;

/// An environment variable by its new name (`RITE_COMMIT`), else by the
/// old one (`AMD_COMMIT`). Names without the `RITE_` prefix are read as
/// they are.
pub fn var(name: &str) -> Result<String, VarError> {
    match std::env::var(name) {
        Err(VarError::NotPresent) => match name.strip_prefix("RITE_") {
            Some(rest) => std::env::var(format!("AMD_{rest}")),
            None => Err(VarError::NotPresent),
        },
        found => found,
    }
}

/// The old and new name, when the program was started by its old name:
/// `argv0` is how it was run (`/usr/local/bin/amd`, `amd.exe`, a symlink).
pub fn old_name(argv0: &OsStr) -> Option<(&'static str, &'static str)> {
    // Either separator, so a Windows path reads the same everywhere.
    let name = argv0.to_str()?.rsplit(['/', '\\']).next()?;
    let stem = name
        .strip_suffix(".exe")
        .or_else(|| name.strip_suffix(".EXE"))
        .unwrap_or(name);
    match stem {
        "amd" => Some(("amd", "rite")),
        "amdui" => Some(("amdui", "riteui")),
        _ => None,
    }
}

/// What to print on stderr when started by an old name, at most once a day
/// (remembered in `state`), or `None`: started by the new name, or already
/// said today. Silent about a state file it can't read or write: the
/// worst case is saying it again.
pub fn notice(argv0: &OsStr, state: Option<&Path>, now: i64) -> Option<String> {
    let (old, new) = old_name(argv0)?;
    if let Some(state) = state {
        let last = fs::read_to_string(state)
            .ok()
            .and_then(|text| text.trim().parse::<i64>().ok());
        if last.is_some_and(|last| (0..NOTICE_EVERY).contains(&(now - last))) {
            return None;
        }
        if let Some(dir) = state.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(state, now.to_string());
    }
    Some(format!(
        "{old} is now {new}: the {old} command keeps working through the 5.x releases, \
         but use {new} from now on."
    ))
}

/// What `rite` and `riteui` do first, when run by an old name: finish an
/// update from 4.x ([`finish_update_from_4x`]), then print [`notice`] on
/// stderr when it has something to say.
pub fn when_run_by_old_name() {
    finish_update_from_4x();
    let Some(argv0) = std::env::args_os().next() else {
        return;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    if let Some(text) = notice(&argv0, notice_state().as_deref(), now) {
        eprintln!("{text}");
    }
}

/// A 4.x install that has just updated itself to 5.x has the new program
/// under the old name, `amd`, and no `rite` (ticket 311): this puts in the
/// new names, once, saying so. Only for an installer's release install, and
/// only when this program is the real file by the old name (a symlink
/// resolves to `rite`, so a finished install is never touched again).
pub fn finish_update_from_4x() {
    use crate::update_apply::{self, Install};
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some((old, _)) = old_name(exe.as_os_str()) else {
        return;
    };
    let Install::Managed(dir) = update_apply::install_dir(&exe) else {
        return;
    };
    match update_apply::adopt_new_names(&dir) {
        Ok(names) if !names.is_empty() => eprintln!(
            "finished updating from {old}: {} now in {}, and {old} still works",
            names.join(" and "),
            dir.display()
        ),
        Ok(_) => {}
        Err(err) => eprintln!("couldn't finish the update to rite: {err:#}"),
    }
}

/// Where [`notice`] remembers when it last spoke, beside the update check's
/// state.
pub fn notice_state() -> Option<PathBuf> {
    crate::update::state_path().map(|path| path.with_file_name("renamed-notice"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_old_names_are_recognised_however_they_were_run() {
        for argv0 in ["amd", "/usr/local/bin/amd", r"C:\bin\amd.exe"] {
            assert_eq!(old_name(OsStr::new(argv0)), Some(("amd", "rite")));
        }
        assert_eq!(old_name(OsStr::new("amdui")), Some(("amdui", "riteui")));
        for argv0 in ["rite", "riteui", "/x/rite.exe", "amd-uninstall"] {
            assert_eq!(old_name(OsStr::new(argv0)), None, "{argv0}");
        }
    }

    #[test]
    fn a_shim_says_so_once_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state/renamed-notice");
        let amd = OsStr::new("amd");
        let first = notice(amd, Some(&state), 1_000_000).unwrap();
        assert!(first.starts_with("amd is now rite"), "{first}");
        assert_eq!(notice(amd, Some(&state), 1_000_000 + 60), None);
        assert!(notice(amd, Some(&state), 1_000_000 + NOTICE_EVERY).is_some());
        assert_eq!(notice(OsStr::new("rite"), Some(&state), 9_000_000), None);
    }

    #[test]
    fn rite_variables_win_and_amd_ones_still_count() {
        // Names no other test uses: the environment is the whole process's.
        unsafe { std::env::set_var("AMD_RENAMED_TEST_A", "old") };
        assert_eq!(var("RITE_RENAMED_TEST_A").as_deref(), Ok("old"));
        unsafe { std::env::set_var("RITE_RENAMED_TEST_A", "new") };
        assert_eq!(var("RITE_RENAMED_TEST_A").as_deref(), Ok("new"));
        assert_eq!(var("RITE_RENAMED_TEST_B"), Err(VarError::NotPresent));
        unsafe {
            std::env::remove_var("AMD_RENAMED_TEST_A");
            std::env::remove_var("RITE_RENAMED_TEST_A");
        }
    }
}
