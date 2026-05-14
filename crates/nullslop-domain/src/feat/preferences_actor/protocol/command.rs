//! Preferences update command — carries a batch of atomic preference diffs.

use serde::{Deserialize, Serialize};

use crate::feat::preferences_actor::UserPreferences;
use crate::protocol::CommandMsg;

/// A single atomic preference update.
///
/// Each variant targets exactly one field in [`UserPreferences`].
/// When a new field is added to `UserPreferences`, a corresponding
/// variant must be added here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PreferenceUpdate {
    /// Set the last model preference.
    SetLastModel(Option<String>),
    /// Set the last strategy preference.
    SetLastStrategy(Option<String>),
}

impl PreferenceUpdate {
    /// Applies this diff to the given preferences in place.
    pub fn apply(&self, prefs: &mut UserPreferences) {
        match self {
            Self::SetLastModel(v) => prefs.last_model.clone_from(v),
            Self::SetLastStrategy(v) => prefs.last_strategy.clone_from(v),
        }
    }
}

/// Command to update one or more preference fields.
///
/// Carries a batch of [`PreferenceUpdate`] diffs. The `PreferencesActor`
/// loads current prefs, applies all diffs, saves, and emits
/// [`PreferencesUpdated`](super::event::PreferencesUpdated) with the full result.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("preferences")]
pub struct UpdatePreferences {
    /// The atomic diffs to apply.
    pub updates: Vec<PreferenceUpdate>,
}
