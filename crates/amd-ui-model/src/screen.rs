//! What the window shows, as the page tells amd-ui (ticket 186). The same
//! shape as `agile_md::screen::View`, which amd-ui writes for an AI
//! assistant's `screen` tool; this crate can't depend on amd-lib.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenView {
    /// `home`, `board`, `documents` or `ideas`.
    pub screen: String,
    pub ticket: Option<ScreenPlace>,
    pub document: Option<ScreenPlace>,
    pub idea: Option<ScreenPlace>,
    pub epic: Option<ScreenPlace>,
    pub search: String,
    pub about: Option<String>,
    pub assignee: String,
}

/// Something in a repository: a ticket's file, a document's or idea's
/// path, an epic's name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenPlace {
    pub repo: String,
    pub path: String,
}

impl ScreenPlace {
    pub fn new(repo: &str, path: &str) -> ScreenPlace {
        ScreenPlace {
            repo: repo.to_string(),
            path: path.to_string(),
        }
    }
}
