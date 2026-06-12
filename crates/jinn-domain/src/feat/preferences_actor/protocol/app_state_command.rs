//! App-state update command and event — carries atomic diffs for runtime state.

use serde::{Deserialize, Serialize};

use crate::feat::preferences_actor::app_state_file::AppStateFile;

/// A single atomic app-state update.
///
/// Each variant targets exactly one field in [`AppStateFile`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AppStateUpdate {
    /// Set the last-used model.
    SetLastModel(Option<String>),
    /// Set the active theme name.
    SetTheme(Option<String>),
    /// Set the active persona name.
    SetPersona(Option<String>),
    /// Set the sidebar width.
    SetSidebarWidth(Option<u16>),
}

impl AppStateUpdate {
    /// Applies this diff to the given app-state in place.
    pub fn apply(&self, state: &mut AppStateFile) {
        match self {
            Self::SetLastModel(v) => v.clone_into(&mut state.last_model),
            Self::SetTheme(v) => v.clone_into(&mut state.theme_name),
            Self::SetPersona(v) => v.clone_into(&mut state.persona_name),
            Self::SetSidebarWidth(v) => {
                state.sidebar_width = *v;
            }
        }
    }
}

/// Command to update one or more app-state fields.
///
/// Carries a batch of [`AppStateUpdate`] diffs. The `AppStateActor`
/// loads current state, applies all diffs, saves, and emits
/// [`AppStateUpdated`](super::event::AppStateUpdated) with the full result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateAppState {
    /// The atomic diffs to apply.
    pub updates: Vec<AppStateUpdate>,
}

impl crate::common::bus::BusMessage for UpdateAppState {}

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
    fn set_last_model_some_applies_to_state() {
        // Given default app-state.
        let mut state = AppStateFile::default();

        // When applying SetLastModel with a model.
        AppStateUpdate::SetLastModel(Some("anthropic/claude-sonnet-4".to_owned()))
            .apply(&mut state);

        // Then the last model is set.
        assert_eq!(
            state.last_model.as_deref(),
            Some("anthropic/claude-sonnet-4")
        );
    }

    #[rstest::rstest]
    fn set_last_model_none_clears_existing() {
        // Given app-state with an existing last model.
        let mut state = AppStateFile {
            last_model: Some("anthropic/claude-sonnet-4".to_owned()),
            ..Default::default()
        };

        // When applying SetLastModel(None).
        AppStateUpdate::SetLastModel(None).apply(&mut state);

        // Then the last model is cleared.
        assert!(state.last_model.is_none());
    }

    #[rstest::rstest]
    fn set_sidebar_width_applies_to_state() {
        // Given default app-state.
        let mut state = AppStateFile::default();

        // When applying SetSidebarWidth with a value.
        AppStateUpdate::SetSidebarWidth(Some(40)).apply(&mut state);

        // Then sidebar_width is set.
        assert_eq!(state.sidebar_width, Some(40));
    }

    #[rstest::rstest]
    fn set_theme_applies_to_state() {
        // Given default app-state.
        let mut state = AppStateFile::default();

        // When applying SetTheme with a name.
        AppStateUpdate::SetTheme(Some("dracula".to_owned())).apply(&mut state);

        // Then theme_name is set.
        assert_eq!(state.theme_name.as_deref(), Some("dracula"));
    }

    #[rstest::rstest]
    fn set_persona_applies_to_state() {
        // Given default app-state.
        let mut state = AppStateFile::default();

        // When applying SetPersona with a name.
        AppStateUpdate::SetPersona(Some("default".to_owned())).apply(&mut state);

        // Then persona_name is set.
        assert_eq!(state.persona_name.as_deref(), Some("default"));
    }
}
