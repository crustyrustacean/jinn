//! App-level intent validators.
//!
//! Validators for quit, toggle-which-key, and normal-escape intents.
//! All are infallible — they always succeed.

use nullslop_component::AppState;

/// Validates the Quit intent.
///
/// Quit can always proceed — it has no preconditions.
pub fn validate_quit(_state: &AppState) {}

/// Validates the ToggleWhichkey intent.
///
/// Toggling the which-key popup can always proceed.
pub fn validate_toggle_whichkey(_state: &AppState) {}

/// Validates the NormalEscape intent.
///
/// Escape in Normal mode can always proceed.
pub fn validate_normal_escape(_state: &AppState) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn validate_quit_always_succeeds() {
        // Given any app state.
        let state = AppState::default();

        // When validating quit.
        validate_quit(&state);

        // Then it succeeds (returns unit without panicking).
    }

    #[rstest::rstest]
    fn validate_toggle_whichkey_always_succeeds() {
        // Given any app state.
        let state = AppState::default();

        // When validating toggle which-key.
        validate_toggle_whichkey(&state);

        // Then it succeeds (returns unit without panicking).
    }

    #[rstest::rstest]
    fn validate_normal_escape_always_succeeds() {
        // Given any app state.
        let state = AppState::default();

        // When validating normal escape.
        validate_normal_escape(&state);

        // Then it succeeds (returns unit without panicking).
    }
}
