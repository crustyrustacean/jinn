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
