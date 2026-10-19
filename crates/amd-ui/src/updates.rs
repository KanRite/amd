//! The update bar's backend (ticket 192): asks GitHub for agile-md's
//! releases, through `agile_md::update`, which decides what to say and
//! remembers "Later". amd-lib has no HTTP client, so the network is here.
//!
//! Silent about failure, as `update::check` is: offline, rate limited or a
//! garbled answer is "nothing to report", never an error on screen.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agile_md::settings::Config;
use agile_md::update::{self, Release, ReleaseSource, Version};
use amd_ui_model::UpdateInfo;

/// agile-md's releases, newest first; 30 is far more than a build is behind.
const RELEASES: &str = "https://api.github.com/repos/agile-md/amd/releases?per_page=30";

/// GitHub, as `update::check`'s source.
struct GitHub;

impl ReleaseSource for GitHub {
    fn releases(&self) -> anyhow::Result<Vec<Release>> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .build()
            .into();
        let body = agent
            .get(RELEASES)
            .header("User-Agent", concat!("amdui/", env!("CARGO_PKG_VERSION")))
            .header("Accept", "application/vnd.github+json")
            .call()?
            .into_body()
            .read_to_string()?;
        update::parse_releases(&body)
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// The release to offer, if any: none when the check is switched off
/// (`update-check = off`, `AMD_UPDATE_CHECK=0`), this build is current, or
/// it was put off with Later in the last day. GitHub is asked at most daily.
pub fn status() -> Option<UpdateInfo> {
    if !update::enabled(&Config::load()) {
        return None;
    }
    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
    let found = update::check(&GitHub, current, now(), &update::state_path()?)?;
    Some(UpdateInfo {
        version: found.version,
        notes_url: found.notes_url,
        behind: found.behind,
        current: current.to_string(),
    })
}

/// "Later": hides `version` for a day, across restarts. A newer release
/// still shows at once.
pub fn later(version: &str) -> anyhow::Result<()> {
    let path = update::state_path().ok_or_else(|| anyhow::anyhow!("no home folder"))?;
    update::snooze(&path, version, now())
}
