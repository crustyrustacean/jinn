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
    /// How a run in this mode prepares the session's context beforehand.
    ///
    /// This is the *only* question the mode answers, and it answers it
    /// whether the run was asked for or fired by a trigger. Which modes may
    /// run at all is a separate question, asked of the whole configuration
    /// in [`AttendantTrigger::is_enabled_for`] — a mode that cannot run is
    /// not a mode, it is a composition state.
    #[must_use]
    pub fn context_policy(self) -> AttendantContextPolicy {
        match self {
            Self::Seed => AttendantContextPolicy::Pin,
            Self::Reset => AttendantContextPolicy::Reset,
            Self::Preserve => AttendantContextPolicy::Preserve,
        }
    }
}

/// What a mode does to a session's context when it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttendantContextPolicy {
    /// Pin every submitted entry into context and dispatch nothing —
    /// the user is still composing the attendant.
    Pin,
    /// Force-exclude every non-pinned entry, so the run sees pins alone.
    Reset,
    /// Leave the existing context alone and run on it as it stands.
    Preserve,
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

impl AttendantTrigger {
    /// Whether a fire from this trigger is configured to run at all.
    ///
    /// Asked of the *configuration*, not of the mode: a trigger that fires
    /// with nothing to send is a silent no-op, which is the worst kind of
    /// failure for a feature whose whole promise is "it just runs". Whether
    /// the mode permits a dispatch is a separate question, and belongs to
    /// whichever code path is actually about to dispatch.
    #[must_use]
    pub fn is_enabled_for(self, mode: AttendantActivation) -> bool {
        match self {
            Self::Manual => false,
            // Every mode that can dispatch, whatever it does to context.
            // `Preserve` is as runnable as `Reset`; it just carries the
            // existing context instead of rebuilding it.
            Self::ParentCompleted => !matches!(mode, AttendantActivation::Seed),
        }
    }
}
