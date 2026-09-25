//! Branch types, and the branch names they produce.
//!
//! There is one kind of ticket. What makes it a piece of code work is that it
//! carries a **branch type** — `feature`, `bugfix`, `hotfix`, `release`,
//! `chore` — from which the branch name is worked out: a `bugfix` ticket
//! titled "Crash on save" is worked on `bugfix/crash-on-save`.
//!
//! The type is **optional and empty by default**. A ticket without one is
//! still a ticket — a rota, an approval, a renewal — it just has nothing to
//! check out, so entering `todo` moves it and leaves the working tree alone.
//!
//! The list matches the branch types mrdoodles/conventional-validator accepts,
//! so a branch made from a ticket passes its check.

use anyhow::{Result, bail};

use crate::settings::{self, Config};
use crate::task;

/// The branch types a ticket may carry. Override the list with
/// `AMD_BRANCH_TYPES="feature,bugfix,spike"`, or the equivalent
/// `branch-types` key in the settings file — the env var wins when both are
/// set (`docs/adr/0005-settings-state-and-precedence.md`).
pub const DEFAULT_BRANCH_TYPES: [&str; 5] = ["feature", "bugfix", "hotfix", "release", "chore"];

/// What an empty branch type looks like in a picker.
pub const NONE: &str = "(none)";

/// The branch types this board accepts.
pub fn branch_types() -> Vec<String> {
    branch_types_from(&Config::load())
}

/// The half that takes a config, so tests need no environment fiddling.
fn branch_types_from(config: &Config) -> Vec<String> {
    match settings::resolved("AMD_BRANCH_TYPES", config, "branch-types") {
        Some(raw) if !raw.trim().is_empty() => {
            let types: Vec<String> = raw
                .split(',')
                .map(|kind| kind.trim().to_string())
                .filter(|kind| !kind.is_empty())
                .collect();
            if types.is_empty() { defaults() } else { types }
        }
        _ => defaults(),
    }
}

fn defaults() -> Vec<String> {
    DEFAULT_BRANCH_TYPES
        .iter()
        .map(|kind| (*kind).to_string())
        .collect()
}

/// The choices a form offers: no branch, then each branch type.
pub fn choices() -> Vec<String> {
    let mut all = vec![NONE.to_string()];
    all.extend(branch_types());
    all
}

/// Check a branch type against the accepted list. Empty is always fine — that's
/// a ticket with no branch.
pub fn validate_branch_type(kind: &str) -> Result<()> {
    if kind.is_empty() || kind == NONE {
        return Ok(());
    }
    let types = branch_types();
    if types.iter().any(|known| known == kind) {
        return Ok(());
    }
    bail!(
        "unknown branch type '{kind}' (expected one of: {}; set AMD_BRANCH_TYPES to change the list)",
        types.join(", ")
    )
}

/// `(none)` and an empty answer mean the same thing: no branch.
pub fn normalise(kind: &str) -> String {
    match kind.trim() {
        NONE => String::new(),
        kind => kind.to_string(),
    }
}

/// `("bugfix", "Crash on save")` -> `bugfix/crash-on-save`. An empty branch
/// type gives an empty name: the ticket simply has no branch.
pub fn for_title(kind: &str, title: &str) -> Result<String> {
    let kind = normalise(kind);
    if kind.is_empty() {
        return Ok(String::new());
    }
    let slug = task::slugify(title);
    if slug.is_empty() {
        bail!(
            "title '{title}' has no letters or numbers to build a branch name from \
             (it becomes the branch {kind}/<title>)"
        );
    }
    let name = format!("{kind}/{slug}");
    validate(&name)?;
    Ok(name)
}

/// Reject anything `git check-ref-format --branch` would.
///
/// Worth doing properly: a branch name can also arrive from a ticket's
/// `branch-name` frontmatter or from `--branch`, which are arbitrary text.
pub fn validate(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("branch name is empty");
    }
    if name == "@" {
        bail!("'@' is not a valid branch name");
    }
    if name.starts_with('-') {
        bail!("branch name '{name}' cannot start with '-'");
    }
    if name.starts_with('/') || name.ends_with('/') {
        bail!("branch name '{name}' cannot start or end with '/'");
    }
    if name.starts_with('.') || name.ends_with('.') {
        bail!("branch name '{name}' cannot start or end with '.'");
    }
    if name.ends_with(".lock") {
        bail!("branch name '{name}' cannot end with '.lock'");
    }
    for forbidden in ["..", "//", "@{", "\\"] {
        if name.contains(forbidden) {
            bail!("branch name '{name}' cannot contain '{forbidden}'");
        }
    }
    for ch in name.chars() {
        if ch.is_control() || ch.is_whitespace() {
            bail!("branch name '{name}' cannot contain whitespace or control characters");
        }
        if "~^:?*[".contains(ch) {
            bail!("branch name '{name}' cannot contain '{ch}'");
        }
    }
    Ok(())
}

/// Check a title at the prompt: it has to make a filename, and for a ticket
/// with a branch type that slug is also the branch.
pub fn validate_sluggable(title: &str) -> Result<()> {
    if title.trim().is_empty() {
        bail!("a title is required");
    }
    if task::slugify(title).is_empty() {
        bail!("title '{title}' has no letters or numbers to build a filename from");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_types_are_the_conventional_branch_ones() {
        assert_eq!(branch_types_from(&Config::default()), DEFAULT_BRANCH_TYPES);
        assert_eq!(choices()[0], NONE);
        assert!(validate_branch_type("bugfix").is_ok());
        let err = validate_branch_type("feat").unwrap_err().to_string();
        assert!(err.contains("unknown branch type 'feat'"), "{err}");
        assert!(err.contains("AMD_BRANCH_TYPES"), "{err}");
    }

    // Precedence itself (env beats file) is proven once, hermetically, by
    // `settings::resolved`'s own tests with a fabricated env var name —
    // deliberately not re-proven here by mutating the real AMD_BRANCH_TYPES,
    // which every other test in this file relies on being unset and which
    // Rust's default parallel test execution would race against.
    #[test]
    fn a_config_file_can_set_the_list_when_the_env_var_is_absent() {
        let mut config = Config::default();
        config.set("branch-types", "spike, docs");
        assert_eq!(branch_types_from(&config), vec!["spike", "docs"]);
    }

    #[test]
    fn no_branch_type_means_no_branch() {
        assert!(validate_branch_type("").is_ok());
        assert!(validate_branch_type(NONE).is_ok());
        assert_eq!(for_title("", "Renew the certs").unwrap(), "");
        assert_eq!(for_title(NONE, "Renew the certs").unwrap(), "");
        assert_eq!(normalise(NONE), "");
        assert_eq!(normalise(" bugfix "), "bugfix");
    }

    #[test]
    fn a_branch_type_and_a_title_make_the_branch_name() {
        assert_eq!(
            for_title("bugfix", "Crash on SAVE!").unwrap(),
            "bugfix/crash-on-save"
        );
        assert_eq!(
            for_title("chore", "Ticket fields").unwrap(),
            "chore/ticket-fields"
        );
    }

    #[test]
    fn a_title_with_nothing_sluggable_is_rejected() {
        let err = for_title("feature", "***").unwrap_err().to_string();
        assert!(err.contains("no letters or numbers"), "{err}");
        assert!(validate_sluggable("   ").is_err());
        assert!(validate_sluggable("***").is_err());
        // With no branch type there's still a filename to make.
        assert!(for_title("", "***").is_ok());
    }

    #[test]
    fn git_ref_rules_are_enforced() {
        for bad in [
            "",
            "@",
            "-lead",
            "/lead",
            "trail/",
            ".lead",
            "trail.",
            "branch.lock",
            "a..b",
            "a//b",
            "a@{b",
            "a\\b",
            "a b",
            "a~b",
            "a^b",
            "a:b",
            "a?b",
            "a*b",
            "a[b",
        ] {
            assert!(validate(bad).is_err(), "{bad:?} should be rejected");
        }
        for good in ["feature/add-login", "hotfix/001-fix", "chore/deps"] {
            assert!(validate(good).is_ok(), "{good:?} should be accepted");
        }
    }
}
