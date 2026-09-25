//! Branch protection on `main`, via `gh api` — the same shell-out philosophy
//! `git.rs` already states for git itself, extended to the forge (see
//! `rfc/0004-reducing-forge-specific-dependencies.md` in `agile-md/amd-engineering`).
//!
//! Protection needs the branch to exist first, so this is only ever called
//! once a repository has its first commit pushed.

use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

/// Sets `main`'s branch protection on `full_name` (`owner/repo`).
///
/// Both tiers block a force-push or a deletion of `main`, and both run with
/// `enforce_admins: true` — without it, the repository's own owner (who is
/// also a GitHub admin on it) could simply bypass whichever tier is set,
/// which would make either one symbolic rather than real.
///
/// `require_review` selects the tier: `false` (the tickets repository)
/// leaves `required_pull_request_reviews` unset entirely, so an ordinary
/// direct push is completely unaffected — only history-rewriting and
/// deletion are blocked. `true` (the documents repository) additionally
/// requires one approving review. `require_last_push_approval` is
/// deliberately never set: that combination is what previously made a solo
/// pusher's own PRs permanently unmergeable elsewhere in this org — a solo
/// human (or an agent acting for them) self-approving is still a real PR
/// with a real diff, just not a second person's review.
pub fn protect_main(full_name: &str, require_review: bool) -> Result<()> {
    let body = if require_review {
        r#"{
            "required_status_checks": null,
            "enforce_admins": true,
            "required_pull_request_reviews": {
                "required_approving_review_count": 1
            },
            "restrictions": null,
            "allow_force_pushes": false,
            "allow_deletions": false
        }"#
    } else {
        r#"{
            "required_status_checks": null,
            "enforce_admins": true,
            "required_pull_request_reviews": null,
            "restrictions": null,
            "allow_force_pushes": false,
            "allow_deletions": false
        }"#
    };

    let mut child = Command::new("gh")
        .args([
            "api",
            "--method",
            "PUT",
            &format!("repos/{full_name}/branches/main/protection"),
            "--input",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("running gh api — is the gh CLI installed and authenticated?")?;

    use std::io::Write;
    child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(body.as_bytes())
        .context("writing branch protection payload to gh api")?;

    let out = child.wait_with_output().context("waiting for gh api")?;
    if !out.status.success() {
        bail!(
            "gh api branch protection for {full_name} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}
