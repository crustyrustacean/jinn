//! Preferences update command - carries a batch of atomic preference diffs.

use serde::{Deserialize, Serialize};

use crate::feat::preferences_actor::UserPreferences;

/// A single atomic preference update.
///
/// Each variant targets exactly one field in [`UserPreferences`].
/// When a new field is added to `UserPreferences`, a corresponding
/// variant must be added here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PreferenceUpdate {
    /// Set the compaction model (provider/model for summarization).
    /// `None` means fall back to the session model.
    SetCompactionModel(Option<String>),
}

impl PreferenceUpdate {
    /// Applies this diff to the given preferences in place.
    pub fn apply(&self, prefs: &mut UserPreferences) {
        match self {
            Self::SetCompactionModel(v) => prefs.compaction.model.clone_from(v),
        }
    }
}

/// Command to update one or more preference fields.
///
/// Carries a batch of [`PreferenceUpdate`] diffs. The `PreferencesActor`
/// loads current prefs, applies all diffs, saves, and emits
/// [`PreferencesUpdated`](super::event::PreferencesUpdated) with the full result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePreferences {
    /// The atomic diffs to apply.
    pub updates: Vec<PreferenceUpdate>,
}

//FIXME: disabled during actor migration
// #[cfg(test)]
#[cfg(any())]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::*;

    #[rstest::rstest]
    fn set_compaction_model_some_applies_to_preferences() {
        // Given default preferences.
        let mut prefs = UserPreferences::default();

        // When applying SetCompactionModel with a model.
        PreferenceUpdate::SetCompactionModel(Some("anthropic/claude-sonnet-4-20250514".to_owned()))
            .apply(&mut prefs);

        // Then the compaction model is set.
        assert_eq!(
            prefs.compaction.model.as_deref(),
            Some("anthropic/claude-sonnet-4-20250514")
        );
    }

    #[rstest::rstest]
    fn set_compaction_model_none_clears_existing() {
        // Given preferences with an existing compaction model.
        let mut prefs = UserPreferences::default();
        prefs.compaction.model = Some("anthropic/claude-sonnet-4-20250514".to_owned());

        // When applying SetCompactionModel(None).
        PreferenceUpdate::SetCompactionModel(None).apply(&mut prefs);

        // Then the compaction model is cleared.
        assert!(prefs.compaction.model.is_none());
    }
}
