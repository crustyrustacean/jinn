//! When an attendant fires and what it does to its context beforehand.

use serde::{Deserialize, Serialize};

/// How an attendant's context is prepared when it runs.
///
/// `Seed` is the state an attendant is *composed* in: the user is still writing
/// its instructions, so submissions are pinned into context and nothing
/// dispatches. A trigger cannot fire in this mode — an attendant that fires
/// against half-written pins is the exact failure this design exists to prevent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttendantActivation {
    /// Being composed. Submissions are pinned; no turn dispatches.
    #[default]
    Seed,
    /// Rebuild context from pins alone before each run.
    Reset,
    /// Append to the existing conversation and keep the prior context.
    ///
    /// Named for what it *preserves*, not for the `c` key that resumes a
    /// session — a different feature that happens to share the old word.
    Preserve,
}

impl AttendantActivation {
    /// Whether a run in this mode may dispatch at all.
    #[must_use]
    pub fn is_dispatchable(self) -> bool {
        !matches!(self, Self::Seed)
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
