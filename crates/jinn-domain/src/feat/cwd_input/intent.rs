//! Cwd input intent handlers - open, confirm, and leave.
//!
//! Skeleton stubs; filled in Phase 4.

use crate::common::app_state::AppState;
use crate::protocol::IntentResult;

/// Opens the cwd input popup.
///
/// Pushes `FocusScope::CwdInput` and seeds an empty [`CwdInputState`].
pub fn handle_cwd_input_enter(state: &mut AppState) -> IntentResult {
    // Phase 4: push scope + seed default state.
    let _ = state;
    IntentResult::empty()
}

/// Confirms the cwd input.
///
/// Resolves the typed path against the active session cwd; on success sets the
/// session cwd and rescans context files inline, then pops the scope and clears
/// state. On failure (not a dir / empty) stays open with the inline error.
pub fn handle_cwd_input_confirm(state: &mut AppState) -> IntentResult {
    // Phase 4: resolve, validate, set_cwd + rescan, pop + clear (or stay open).
    let _ = state;
    IntentResult::empty()
}

/// Cancels the cwd input popup.
///
/// Pops the scope and discards the input state.
pub fn handle_cwd_input_leave(state: &mut AppState) -> IntentResult {
    // Phase 4: pop scope + clear state.
    let _ = state;
    IntentResult::empty()
}
