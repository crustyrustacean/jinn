//! When an attendant fires, and what it does to its context beforehand.

use serde::{Deserialize, Serialize};

/// What an attendant does to its context when it runs.
///
/// The behavior says only what the run *sees* — never whether it happens.
/// Whether an attendant may run at all is a separate fact, held as the
/// session's prep mode; a behavior that could not run would not be a
/// behavior.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttendantBehavior {
    /// Force-exclude every non-pinned entry, so the run sees pins alone.
    #[default]
    Reset,
    /// Append to the existing conversation and keep the prior context.
    ///
    /// Named for what it *preserves*, not for the `c` key that resumes a
    /// session — a different feature that happens to share the old word.
    Preserve,
}

impl AttendantBehavior {
    /// Whether a run in this behavior rebuilds context from the pins alone.
    #[must_use]
    pub fn resets_context(self) -> bool {
        matches!(self, Self::Reset)
    }
}

/// What causes an attendant to run without the user asking.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttendantTrigger {
    /// Runs only when the user re-runs it.
    #[default]
    Manual,
    /// Runs after the parent's turn completes successfully.
    ParentCompleted,
}
