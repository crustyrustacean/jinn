//! When an attendant fires, what it does to its context beforehand, and
//! whether its model is its own.

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

/// Whether an attendant's model is its own or merely the copy it inherited.
///
/// The session always holds a concrete model selection — an attendant runs,
/// and running needs a model — so this is the only record of whether that
/// value was *chosen* for this attendant or arrived with the parent.
/// Without it a save cannot tell a hand-written inheriting entry from a
/// pinned one, and writes a `model` key over the former.
///
/// A two-variant enum rather than a bare `bool`: this value is persisted into
/// a session blob, and `true`/`false` in that file is a fact about a bool,
/// not about an attendant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttendantModelSetting {
    /// The model is the parent's, as copied when this attendant was created.
    ///
    /// The default, because an entry that carries no `model` key says
    /// exactly this and the reading has always been "inherit".
    ///
    /// Not the other way round: nothing about the absence of a `model` key
    /// promises the model still *is* the parent's. A user who edits the key
    /// by hand, or changes their default model between two runs of the same
    /// entry, gets an attendant that holds the older concrete value. What
    /// is absent is a claim to have chosen it.
    #[default]
    Inherit,
    /// The model belongs to this attendant, and is written to its entry.
    Fixed,
}

impl AttendantModelSetting {
    /// Whether this setting stores a model into a saved entry.
    ///
    /// The single reading both the save path and the restore path use, so a
    /// row cannot mean one thing when it writes and another when it reads.
    #[must_use]
    pub fn is_fixed(self) -> bool {
        matches!(self, Self::Fixed)
    }

    /// The setting an entry that carries `configured` models restore as.
    ///
    /// The mirror of what save writes: an entry with a `model` key came from
    /// a Fixed attendant, and one without it from an inheriting one.
    #[must_use]
    pub fn of_configured_entry(configured: bool) -> Self {
        if configured {
            Self::Fixed
        } else {
            Self::Inherit
        }
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
