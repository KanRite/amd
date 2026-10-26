//! The work-in-progress limit: how many tickets doing may hold at once, so
//! work is finished before more is started. Only doing is limited — a limit
//! on todo as well could leave work stuck with nowhere to go.
//!
//! The limit is a team decision, so it lives with the tickets, in the
//! repository's `.settings`, under the key `rite` uses: `wip-limit`. Unset or
//! unreadable means [`DEFAULT_LIMIT`]; 0 turns it off. It only gates entry:
//! moving a ticket out of doing is never blocked, however full it is.

use serde::{Deserialize, Serialize};

use crate::Column;

/// Doing's limit when nobody has set one.
pub const DEFAULT_LIMIT: u32 = 2;

/// The board's WIP limit on doing; 0 means no limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WipLimits {
    pub doing: u32,
}

impl Default for WipLimits {
    fn default() -> Self {
        WipLimits {
            doing: DEFAULT_LIMIT,
        }
    }
}

impl WipLimits {
    /// The limit on `column`, if it has one: only doing ever does.
    pub fn of(self, column: Column) -> Option<u32> {
        (column == Column::Doing && self.doing > 0).then_some(self.doing)
    }

    /// Whether `column`, holding `count` tickets, can take no more.
    pub fn is_full(self, column: Column, count: usize) -> bool {
        self.of(column).is_some_and(|limit| count >= limit as usize)
    }

    /// Whether `ticket` may move into `column`, which already holds `count`
    /// tickets (not counting this one). The refusal names the limit and
    /// the count.
    pub fn admit(self, column: Column, count: usize, ticket: &str) -> Result<(), String> {
        admit(self.of(column), "Doing", count, ticket)
    }
}

/// Whether `ticket` may move into a column called `label` with `limit`
/// (none is no limit), already holding `count`: any doing-kind column has
/// one of its own (ticket 224).
pub fn admit(limit: Option<u32>, label: &str, count: usize, ticket: &str) -> Result<(), String> {
    match limit {
        Some(limit) if count >= limit as usize => Err(format!(
            "{label} is full: its limit is {limit} and it holds {count}. \
             Move a ticket on first, or move {ticket} anyway."
        )),
        _ => Ok(()),
    }
}

impl WipLimits {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_column_refuses_naming_the_limit_and_count() {
        let limits = WipLimits::default();
        assert!(limits.admit(Column::Doing, 1, "026").is_ok());
        let err = limits.admit(Column::Doing, 2, "026").unwrap_err();
        assert_eq!(
            err,
            "Doing is full: its limit is 2 and it holds 2. Move a ticket on first, or move 026 anyway."
        );
        assert!(limits.is_full(Column::Doing, 3) && !limits.is_full(Column::Doing, 1));
        assert!(limits.admit(Column::Todo, 50, "026").is_ok());
        let off = WipLimits { doing: 0 };
        assert!(off.admit(Column::Doing, 9, "026").is_ok() && !off.is_full(Column::Doing, 9));
    }
}
