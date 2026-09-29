//! Attendant properties popup — opening, closing, and cell seeding.
//!
//! The popup lives on its own dynamic scope with the three controls
//! (trigger, activation, seed template) as a single form. This module is
//! the sidebar-side glue: `P` in the sessions scope seeds the popup's cell
//! from the highlighted attendant and pushes the scope; the popup's own
//! rows (in `jinn-attendant`) handle editing, applying, and leaving.
//!
//! The apply half commits straight to `AppState` and publishes
//! `PersistSession`; it cannot live in `jinn-attendant-msg` (no state
//! access) and stays near the sidebar because that is where the user is.

use jinn_attendant_msg::{
    AttendantPropertiesState, attendant_properties_scope, attendant_properties_slot,
};
use jinn_kernel::common::app_state::AppState;
use jinn_kernel::protocol::IntentResult;
use jinn_slices::ScopeSignal;

use super::sorted_open_sessions;

/// Opens the attendant properties popup for the highlighted session.
///
/// Validates that the highlighted session *is* an attendant — the popup has
/// nothing to edit on a user session, fork, or subagent — seeds the popup's
/// cell with the session's current values, and pushes the popup scope.
pub fn handle_open_attendant_properties(state: &mut AppState) -> IntentResult {
    let Some(index) = state
        .frontend
        .with_sections(|s| s.sessions.selected_index, || None)
    else {
        return IntentResult::empty();
    };
    let sessions = sorted_open_sessions(state);
    let Some(entry) = sessions.get(index) else {
        return IntentResult::empty();
    };
    let Some(session) = state.session.get(&entry.id) else {
        return IntentResult::empty();
    };
    if !session.is_attendant() {
        return IntentResult::empty();
    }

    let template = session.seed_template().to_owned();
    let cursor_pos = template.len();
    let popup = AttendantPropertiesState {
        session_id: Some(entry.id.clone()),
        seed_template: jinn_slices::LineInput {
            input: template,
            cursor_pos,
        },
        trigger_focus: false,
        activation_focus: false,
        current_activation: session.attendant_activation(),
        current_trigger: session.attendant_trigger(),
    };
    // The popup state rides its cell (registered by the cell catalog); the
    // synchronous write here is the sanctioned carve-out — the same one the
    // rename input uses for its text. A missing cell means the attendant
    // slice never activated; the popup then simply does not open.
    if let Some(cell) =
        state.frontend.scope_focus.get().and_then(|slices| {
            slices.reader::<AttendantPropertiesState>(&attendant_properties_slot())
        })
    {
        cell.update(|s| *s = popup);
    }
    IntentResult::empty().with_scope_signal(ScopeSignal::Push(attendant_properties_scope()))
}
