//! Applying an update (ticket 191): replacing the installed `rite` and
//! `riteui` with a newer release's, and nothing else.
//!
//! **User data is never touched.** The only files written are the two
//! binaries, in the folder they're installed in, their temporary copies
//! beside them, and the previous versions kept as `rite.previous` and
//! `riteui.previous` for [`rollback`] (on Windows `rite.exe`, `riteui.exe`
//! and `rite.exe.previous`: [`file_name`]). No ticket, document, idea, setting,
//! lock or git repository is opened, let alone changed: releases carry no
//! breaking changes (ticket 193), so there is nothing to migrate.
//!
//! - **Only an installer's release install updates itself.** The installer
//!   writes `amd-uninstall` beside `rite` and records in it how it installed
//!   (`# agile-md install: release` or `source`). Anything else — a source
//!   build, a package manager's install, an install from before the marker
//!   — is told how to update and left alone ([`install_dir`]).
//! - **Verified before anything is replaced:** each release file's SHA-256
//!   must match the one GitHub publishes for it.
//! - **One release at a time, in order** ([`apply`]): each new `rite` (and
//!   `riteui`) is run once (`--version`) before it's installed, so a failure
//!   at release N leaves release N-1 in place, working.
//! - **Atomic:** each binary is written to a temporary file in the same
//!   folder and renamed over the old one, so an interrupted download, a
//!   full disk or a bad file leaves the old binaries as they were.
//!
//! rite-lib has no HTTP client: the download is a [`Fetch`] the front end
//! supplies (`ureq`), and the tests use a fake.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};

use crate::update::{Release, Version};

/// The binaries a release zip carries, `rite` first: it must be there;
/// `riteui` is installed when it is.
pub const BINARIES: [&str; 2] = ["rite", "riteui"];

/// What the installer writes beside `rite`, and where it says how.
pub const MARKER: &str = "amd-uninstall";

/// The line in [`MARKER`] that a release install has.
pub const RELEASE_INSTALL: &str = "# agile-md install: release";

/// How to install (or update by hand): the installer, run again.
pub const INSTALL_COMMAND: &str =
    "curl -fsSL https://raw.githubusercontent.com/agile-md/amd/v4/install.sh | bash";

/// Downloads a URL's bytes. The front end supplies the network.
pub trait Fetch {
    fn get(&self, url: &str) -> Result<Vec<u8>>;
}

/// The release build for this system, as the release workflow names it,
/// or `None` where there's no build (FreeBSD, say).
pub fn target() -> Option<&'static str> {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("x86_64", "linux") => Some("x86_64-unknown-linux-gnu"),
        ("aarch64", "linux") => Some("aarch64-unknown-linux-gnu"),
        ("x86_64", "macos") => Some("x86_64-apple-darwin"),
        ("aarch64", "macos") => Some("aarch64-apple-darwin"),
        ("x86_64", "windows") => Some("x86_64-pc-windows-msvc"),
        ("aarch64", "windows") => Some("aarch64-pc-windows-msvc"),
        _ => None,
    }
}

/// The release file for a build target.
pub fn asset_name(target: &str) -> String {
    format!("amd-{target}.zip")
}

/// A binary's file name here, in the zip and installed: `rite.exe` on
/// Windows (ticket 281), `rite` everywhere else.
pub fn file_name(bin: &str) -> String {
    format!("{bin}{}", std::env::consts::EXE_SUFFIX)
}

/// Whether the binaries in a folder may be replaced.
#[derive(Debug, PartialEq, Eq)]
pub enum Install {
    /// An installer's release install, in this folder.
    Managed(PathBuf),
    /// Not one rite replaces: what to tell the person, and whether running
    /// the installer ([`INSTALL_COMMAND`]) is the way to update it.
    Unmanaged { why: String, reinstall: bool },
}

/// Judges the install that `exe` (the running `rite` or `riteui`) belongs to.
pub fn install_dir(exe: &Path) -> Install {
    let Some(dir) = exe.parent() else {
        return Install::Unmanaged {
            why: "couldn't tell where rite is installed".into(),
            reinstall: true,
        };
    };
    let shown = dir.display();
    match crate::eol::read(dir.join(MARKER)) {
        Err(_) => Install::Unmanaged {
            why: format!(
                "rite in {shown} wasn't put there by agile-md's installer, so it isn't replaced: \
                 update it the way you installed it (a package manager, say), or use the installer"
            ),
            reinstall: true,
        },
        Ok(text) if text.lines().any(|l| l.trim() == RELEASE_INSTALL) => {
            Install::Managed(dir.to_path_buf())
        }
        Ok(text)
            if text
                .lines()
                .any(|l| l.trim() == "# agile-md install: source") =>
        {
            Install::Unmanaged {
                why: format!(
                    "rite in {shown} was built from source, so it isn't replaced with a release: \
                     update your clone and run ./install.sh there again"
                ),
                reinstall: false,
            }
        }
        Ok(_) => Install::Unmanaged {
            why: format!(
                "rite in {shown} was installed before it could update itself: run the installer \
                 once more, and later updates happen here"
            ),
            reinstall: true,
        },
    }
}

/// The releases to apply to go from `current` to the newest, oldest first.
pub fn steps(current: Version, releases: &[Release]) -> Vec<&Release> {
    let mut newer: Vec<&Release> = releases.iter().filter(|r| r.version > current).collect();
    newer.sort_by_key(|r| r.version);
    newer.dedup_by_key(|r| r.version);
    newer
}

/// Where an update has got to, for a front end to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Downloading(Version),
    Installing(Version),
    Installed(Version),
}

/// What [`apply`] did: the release it reached (`None`: none applied), and
/// the one that failed, with why.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub reached: Option<Version>,
    pub failed: Option<(Version, String)>,
}

/// Updates the install in `dir` from `current` to the newest of `releases`,
/// one release at a time ([`steps`]), stopping at the first that fails.
pub fn apply(
    dir: &Path,
    current: Version,
    releases: &[Release],
    fetch: &dyn Fetch,
    progress: &mut dyn FnMut(Step),
) -> Applied {
    let mut applied = Applied::default();
    for release in steps(current, releases) {
        match apply_one(dir, release, fetch, progress) {
            Ok(()) => applied.reached = Some(release.version),
            Err(err) => {
                applied.failed = Some((release.version, format!("{err:#}")));
                break;
            }
        }
    }
    applied
}

/// One release: download, verify, try, then put in place.
fn apply_one(
    dir: &Path,
    release: &Release,
    fetch: &dyn Fetch,
    progress: &mut dyn FnMut(Step),
) -> Result<()> {
    let version = release.version;
    let target = target().ok_or_else(|| anyhow!("there's no release build for this system"))?;
    let name = asset_name(target);
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == name)
        .ok_or_else(|| anyhow!("{version} has no build for {target}"))?;
    if asset.sha256.is_empty() {
        bail!("{version} has no checksum on GitHub, so it isn't installed");
    }
    progress(Step::Downloading(version));
    let bytes = fetch
        .get(&asset.url)
        .with_context(|| format!("downloading {version}"))?;
    let got = sha256_hex(&bytes);
    if got != asset.sha256 {
        bail!(
            "{version}'s download doesn't match its checksum (damaged, or not what was published); nothing was replaced"
        );
    }
    let files = unzip(&bytes).with_context(|| format!("reading {version}'s download"))?;
    if !files.iter().any(|(n, _)| *n == "rite") {
        bail!("{version}'s download has no rite in it");
    }

    progress(Step::Installing(version));
    // Written beside the live ones (same file system, so the rename is
    // atomic) and each run once before anything is replaced.
    let mut ready = Vec::new();
    let cleanup = |ready: &[(String, PathBuf)]| {
        for (_, tmp) in ready {
            let _ = fs::remove_file(tmp);
        }
    };
    for (bin, data) in &files {
        // Ending `.exe` on Windows, which runs nothing else.
        let tmp = dir.join(format!(
            ".{bin}.new-{}{}",
            std::process::id(),
            std::env::consts::EXE_SUFFIX
        ));
        if let Err(err) = write_executable(&tmp, data) {
            cleanup(&ready);
            let _ = fs::remove_file(&tmp);
            return Err(err).with_context(|| format!("writing {}", tmp.display()));
        }
        ready.push((bin.to_string(), tmp));
    }
    for (bin, tmp) in &ready {
        if let Err(err) = runs_as(tmp, version) {
            cleanup(&ready);
            return Err(err).with_context(|| format!("{version}'s {bin} doesn't run here"));
        }
    }
    for (bin, tmp) in &ready {
        let live = dir.join(file_name(bin));
        let previous = dir.join(format!("{}.previous", file_name(bin)));
        if live.exists() {
            fs::rename(&live, &previous)
                .with_context(|| format!("keeping {} as {}", live.display(), previous.display()))?;
        }
        if let Err(err) = fs::rename(tmp, &live) {
            // Put the old one back: a failure here must leave it working.
            let _ = fs::rename(&previous, &live);
            cleanup(&ready);
            return Err(err).with_context(|| format!("installing {}", live.display()));
        }
    }
    progress(Step::Installed(version));
    Ok(())
}

/// BYTES' SHA-256 as lowercase hex, the form GitHub's `digest` gives. sha2
/// 0.11's digest no longer formats with `{:x}` (ticket 319).
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The binaries in a release zip, by name, `rite` first.
fn unzip(bytes: &[u8]) -> Result<Vec<(&'static str, Vec<u8>)>> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    let mut found = Vec::new();
    for bin in BINARIES {
        let Ok(mut file) = archive.by_name(&file_name(bin)) else {
            continue;
        };
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;
        found.push((bin, data));
    }
    Ok(found)
}

fn write_executable(path: &Path, data: &[u8]) -> Result<()> {
    fs::write(path, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Runs `bin --version` and checks it names `version`.
fn runs_as(bin: &Path, version: Version) -> Result<()> {
    let out = version_output(bin).with_context(|| format!("running {}", bin.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() || !text.contains(&version.to_string()) {
        bail!(
            "`--version` said {:?}{}",
            text.trim(),
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .next()
                .map(|l| format!(" ({l})"))
                .unwrap_or_default()
        );
    }
    Ok(())
}

/// `bin --version`'s output, run as soon as it can be (ticket 265). A file
/// just written can't be run while anything still holds it open for
/// writing, and a process another thread forks in that moment holds the
/// write handle until it execs: Linux says "Text file busy" (ETXTBSY). It
/// clears within milliseconds, so the run is tried again, a few times,
/// as Go's standard library does.
fn version_output(bin: &Path) -> std::io::Result<Output> {
    let mut tries = 0;
    loop {
        match Command::new(bin).arg("--version").output() {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 20 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(10 * tries));
            }
            result => return result,
        }
    }
}

/// Puts the previous `rite` and `riteui` back (`rite update --rollback`),
/// returning which. The rolled-back-from ones are removed.
pub fn rollback(dir: &Path) -> Result<Vec<String>> {
    let mut restored = Vec::new();
    for bin in BINARIES {
        let previous = dir.join(format!("{}.previous", file_name(bin)));
        if previous.exists() {
            fs::rename(&previous, dir.join(file_name(bin)))
                .with_context(|| format!("restoring {}", previous.display()))?;
            restored.push(bin.to_string());
        }
    }
    if restored.is_empty() {
        bail!(
            "there's no previous version in {} to go back to",
            dir.display()
        );
    }
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::Asset;
    use std::cell::RefCell;
    use std::collections::{BTreeMap, HashMap};
    use std::io::Write;

    /// A release zip whose `rite` and `riteui` are scripts saying `version`.
    fn release_zip(version: &str) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut out);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755);
        for bin in BINARIES {
            zip.start_file(bin, options).unwrap();
            write!(zip, "#!/bin/sh\necho \"{bin} {version}\"\n").unwrap();
        }
        zip.finish().unwrap();
        out.into_inner()
    }

    /// Serves URLs from a map, counting what it was asked for.
    struct Fake {
        files: HashMap<String, Vec<u8>>,
        asked: RefCell<Vec<String>>,
    }

    impl Fetch for Fake {
        fn get(&self, url: &str) -> Result<Vec<u8>> {
            self.asked.borrow_mut().push(url.to_string());
            self.files
                .get(url)
                .cloned()
                .ok_or_else(|| anyhow!("no such file {url}"))
        }
    }

    /// Releases for these versions, with `bad` published under a wrong
    /// checksum, and the server holding them.
    fn published(versions: &[&str], bad: Option<&str>) -> (Vec<Release>, Fake) {
        let target = target().expect("tests run on a release target");
        let mut files = HashMap::new();
        let mut releases = Vec::new();
        for version in versions {
            let bytes = release_zip(version);
            let url = format!("https://example.test/{version}/{}", asset_name(target));
            let sha256 = if Some(*version) == bad {
                "0".repeat(64)
            } else {
                sha256_hex(&bytes)
            };
            files.insert(url.clone(), bytes);
            releases.push(Release {
                version: Version::parse(version).unwrap(),
                notes_url: String::new(),
                assets: vec![Asset {
                    name: asset_name(target),
                    url,
                    sha256,
                }],
            });
        }
        let fake = Fake {
            files,
            asked: RefCell::new(Vec::new()),
        };
        (releases, fake)
    }

    /// An installer's release install of `version`.
    fn installed(version: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for bin in BINARIES {
            write_executable(
                &dir.path().join(bin),
                format!("#!/bin/sh\necho \"{bin} {version}\"\n").as_bytes(),
            )
            .unwrap();
        }
        fs::write(
            dir.path().join(MARKER),
            format!("#!/usr/bin/env bash\n{RELEASE_INSTALL}\n"),
        )
        .unwrap();
        dir
    }

    /// Ticket 319: the checksum is the lowercase hex GitHub gives, checked
    /// against SHA-256's published answer for "abc".
    #[test]
    fn the_checksum_is_lowercase_hex() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// Ticket 265: a program still open for writing somewhere can't be
    /// run ("Text file busy"); the run waits it out rather than failing.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_program_still_open_for_writing_is_run_once_it_is_closed() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("rite");
        let mut file = fs::File::create(&bin).unwrap();
        file.write_all(b"#!/bin/sh\necho rite 9.9.9\n").unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        // Still open for writing: running it now would be refused.
        assert_eq!(
            Command::new(&bin)
                .arg("--version")
                .output()
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::ExecutableFileBusy
        );
        let closer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            drop(file);
        });
        let out = version_output(&bin).unwrap();
        closer.join().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "rite 9.9.9");
    }

    fn version_of(dir: &Path, bin: &str) -> String {
        let out = version_output(&dir.join(bin)).unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Every file under `root`, with its bytes.
    fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut all = BTreeMap::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    all.insert(path.clone(), fs::read(&path).unwrap());
                }
            }
        }
        all
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "the pretend binaries are shell scripts; self-update on Windows is ticket 282"
    )]
    fn an_update_goes_one_release_at_a_time_and_touches_only_the_binaries() {
        let dir = installed("4.5.0");
        // A project beside it, with a board, documents and a git repository.
        let project = tempfile::tempdir().unwrap();
        let tickets = project.path().join("acme-tickets");
        fs::create_dir_all(tickets.join("tasks/backlog")).unwrap();
        fs::write(
            tickets.join("tasks/backlog/001-x.md"),
            "---\nid: \"001\"\n---\n",
        )
        .unwrap();
        fs::write(tickets.join(".settings"), "commit = on\n").unwrap();
        Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(&tickets)
            .status()
            .unwrap();
        let before = snapshot(project.path());

        let (releases, fake) = published(&["4.6.0", "4.5.0", "4.7.0"], None);
        let mut seen = Vec::new();
        let applied = apply(
            dir.path(),
            Version::parse("4.5.0").unwrap(),
            &releases,
            &fake,
            &mut |step| seen.push(step),
        );
        assert_eq!(applied.reached, Version::parse("4.7.0"));
        assert_eq!(applied.failed, None);
        // 4.6.0 before 4.7.0, never 4.5.0 again.
        let asked = fake.asked.borrow();
        assert_eq!(asked.len(), 2, "{asked:?}");
        assert!(
            asked[0].contains("/4.6.0/") && asked[1].contains("/4.7.0/"),
            "{asked:?}"
        );
        assert_eq!(seen.last(), Some(&Step::Installed(Version(4, 7, 0))));
        assert_eq!(version_of(dir.path(), "rite"), "rite 4.7.0");
        assert_eq!(version_of(dir.path(), "riteui"), "riteui 4.7.0");
        assert_eq!(version_of(dir.path(), "rite.previous"), "rite 4.6.0");
        // Only the binaries changed, and nothing of the project's.
        let mut names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "amd-uninstall",
                "rite",
                "rite.previous",
                "riteui",
                "riteui.previous"
            ]
        );
        assert_eq!(
            snapshot(project.path()),
            before,
            "the project must be byte-identical"
        );
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "the pretend binaries are shell scripts; self-update on Windows is ticket 282"
    )]
    fn a_failure_part_way_leaves_the_last_good_release_working() {
        let dir = installed("4.5.0");
        let (releases, fake) = published(&["4.6.0", "4.7.0", "4.8.0"], Some("4.7.0"));
        let applied = apply(
            dir.path(),
            Version::parse("4.5.0").unwrap(),
            &releases,
            &fake,
            &mut |_| {},
        );
        assert_eq!(applied.reached, Version::parse("4.6.0"));
        let (version, why) = applied.failed.unwrap();
        assert_eq!(version, Version(4, 7, 0));
        assert!(why.contains("doesn't match its checksum"), "{why}");
        assert_eq!(version_of(dir.path(), "rite"), "rite 4.6.0");
        // 4.8.0 wasn't even fetched, and no temporary files are left.
        assert_eq!(fake.asked.borrow().len(), 2);
        assert!(
            fs::read_dir(dir.path()).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with('.'))
        );
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "the pretend binaries are shell scripts; self-update on Windows is ticket 282"
    )]
    fn a_failed_download_or_a_binary_that_wont_run_changes_nothing() {
        let dir = installed("4.5.0");
        let (mut releases, fake) = published(&["4.6.0"], None);
        // The file isn't there: a failed download.
        releases[0].assets[0].url = "https://example.test/gone.zip".into();
        let applied = apply(dir.path(), Version(4, 5, 0), &releases, &fake, &mut |_| {});
        assert_eq!(applied.reached, None);
        assert!(applied.failed.unwrap().1.contains("downloading 4.6.0"));
        assert_eq!(version_of(dir.path(), "rite"), "rite 4.5.0");

        // A download that says the wrong version when run.
        let (releases, mut fake) = published(&["4.6.0"], None);
        let url = releases[0].assets[0].url.clone();
        let wrong = release_zip("1.0.0");
        let mut releases = releases;
        releases[0].assets[0].sha256 = sha256_hex(&wrong);
        fake.files.insert(url, wrong);
        let applied = apply(dir.path(), Version(4, 5, 0), &releases, &fake, &mut |_| {});
        assert!(applied.failed.unwrap().1.contains("doesn't run here"));
        assert_eq!(version_of(dir.path(), "rite"), "rite 4.5.0");
        assert!(!dir.path().join("rite.previous").exists());
    }

    #[test]
    fn only_an_installers_release_install_is_replaced() {
        let dir = installed("4.5.0");
        let exe = dir.path().join("rite");
        assert_eq!(
            install_dir(&exe),
            Install::Managed(dir.path().to_path_buf())
        );
        fs::write(
            dir.path().join(MARKER),
            "#!/usr/bin/env bash\n# agile-md install: source\n",
        )
        .unwrap();
        assert!(matches!(
            install_dir(&exe),
            Install::Unmanaged { why, reinstall: false } if why.contains("built from source")
        ));
        fs::write(dir.path().join(MARKER), "#!/usr/bin/env bash\n").unwrap();
        assert!(matches!(
            install_dir(&exe),
            Install::Unmanaged { why, reinstall: true } if why.contains("run the installer")
        ));
        fs::remove_file(dir.path().join(MARKER)).unwrap();
        assert!(matches!(
            install_dir(&exe),
            Install::Unmanaged { why, reinstall: true } if why.contains("wasn't put there")
        ));
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "the pretend binaries are shell scripts; self-update on Windows is ticket 282"
    )]
    fn rollback_puts_the_previous_binaries_back() {
        let dir = installed("4.5.0");
        let (releases, fake) = published(&["4.6.0"], None);
        apply(dir.path(), Version(4, 5, 0), &releases, &fake, &mut |_| {});
        assert_eq!(version_of(dir.path(), "rite"), "rite 4.6.0");
        assert_eq!(rollback(dir.path()).unwrap(), ["rite", "riteui"]);
        assert_eq!(version_of(dir.path(), "rite"), "rite 4.5.0");
        assert!(rollback(dir.path()).is_err(), "nothing left to go back to");
    }

    /// Ticket 281: every system CI builds and tests on has a release
    /// build, Windows included, and its binaries' names.
    #[test]
    fn this_system_has_a_release_build() {
        let target = target().expect("a release build for this system");
        assert!(asset_name(target).starts_with("amd-"));
        let rite = if cfg!(windows) { "rite.exe" } else { "rite" };
        assert_eq!(file_name("rite"), rite);
    }

    #[test]
    fn release_files_carry_githubs_checksum() {
        let json = r#"[{"tag_name":"v4.6.0","html_url":"https://x","draft":false,"prerelease":false,
            "assets":[{"name":"amd-x86_64-unknown-linux-gnu.zip",
                       "browser_download_url":"https://x/rite.zip","digest":"sha256:ABC123"}]}]"#;
        let releases = crate::update::parse_releases(json).unwrap();
        assert_eq!(releases[0].assets[0].sha256, "abc123");
        assert_eq!(releases[0].assets[0].url, "https://x/rite.zip");
    }
}
