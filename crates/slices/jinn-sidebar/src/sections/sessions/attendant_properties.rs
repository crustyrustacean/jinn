//! Attendant properties popup — opening, closing, and cell seeding.
//!
//! The popup lives on its own dynamic scope with the seven controls
//! (trigger, behavior, prep mode, tool set, skill set, model, seed template)
//! as a single form. This module is the sidebar-side glue: `P` seeds the
//! popup's cell from the highlighted attendant and pushes the scope; the
//! popup's own rows (in `jinn-attendant`) handle editing, applying, and
//! leaving.
//!
//! `P` answers for two sections, because there are two places a user stands
//! when they decide an attendant needs editing. The sessions section lists
//! the whole tree and reaches the attendant through its own row; the
//! attendants section lists an attendant's siblings directly, which is the
//! screen a user is already on when the trigger or the frozen sets are what
//! they came to change. Only the *row resolution* differs — each section owns
//! its own cursor over its own list — so the split is drawn at the id: this
//! module resolves a highlighted attendant from whichever section is focused,
//! and everything after that is section-agnostic.
//!
//! The apply half commits straight to `AppState` and publishes
//! `PersistSession`; it cannot live in `jinn-attendant-msg` (no state
//! access) and stays near the sidebar because that is where the user is.

use jinn_attendant_msg::{
    AttendantPropertiesState, OriginalValues, attendant_properties_scope, attendant_properties_slot,
};
use jinn_kernel::common::app_state::AppState;
use jinn_kernel::protocol::IntentResult;
use jinn_sidebar_msg::SidebarSectionId;
use jinn_slices::ScopeSignal;

/// Opens the attendant properties popup for the highlighted attendant,
/// from whichever sidebar section is focused.
pub fn handle_open_attendant_properties(state: &mut AppState) -> IntentResult {
    let Some(id) = highlighted_attendant_id(state) else {
        return IntentResult::empty();
    };
    open_properties_for(state, &id)
}

/// The session id of the highlighted attendant, or `None` when the focused
/// section has no attendant under its cursor.
///
/// Both sections keep their own cursor and clear the one they are leaving,
/// so exactly one of them is ever set — which is what makes reading the
/// focused section's cursor the right answer rather than a guess. Both
/// cursors are session ids, so this is a choice between two identities rather
/// than between two row positions: the two sections list different sessions
/// over different orders, and there is no index to resolve against the wrong
/// one.
fn highlighted_attendant_id(state: &AppState) -> Option<jinn_core_types::SessionId> {
    let focused = state.frontend.sidebar_section()?;
    state.frontend.with_sections(
        |s| match focused {
            SidebarSectionId::Attendant => s.attendant.selected_id.clone(),
            _ => s.sessions.selected_id.clone(),
        },
        || None,
    )
}

/// Opens the properties popup over one specific attendant.
///
/// Validates that the session *is* an attendant — the popup has nothing to
/// edit on a user session, fork, or subagent — seeds the popup's cell with
/// the session's current values (as both the pending edits and the open-time
/// snapshot that leaving restores), and pushes the popup scope.
fn open_properties_for(state: &mut AppState, id: &jinn_core_types::SessionId) -> IntentResult {
    let Some(session) = state.session.get(id) else {
        return IntentResult::empty();
    };
    if !session.is_attendant() {
        return IntentResult::empty();
    }

    let template = session.seed_template().to_owned();
    let cursor_pos = template.len();
    let prep_mode = session.attendant_is_prepping();
    // What the set rows say is a reading of the attendant's own filters:
    // only a present allow-mode filter pins a set, so a blocklist — the shape
    // the pickers write — still opens as Live.
    let tool_set = session.tool_filter().cloned();
    let skill_set = session.skill_filter().cloned();
    let model_setting = session.attendant_model_setting();
    let popup = AttendantPropertiesState {
        session_id: Some(id.clone()),
        seed_template: jinn_slices::LineInput {
            input: template.clone(),
            cursor_pos,
        },
        pending_behavior: session.attendant_behavior(),
        pending_trigger: session.attendant_trigger(),
        pending_prep_mode: prep_mode,
        pending_tool_set: OriginalValues::mode_of(tool_set.as_ref()),
        frozen_tools: OriginalValues::names_of(tool_set.as_ref()),
        pending_skill_set: OriginalValues::mode_of(skill_set.as_ref()),
        frozen_skills: OriginalValues::names_of(skill_set.as_ref()),
        pending_model_setting: model_setting,
        original: Some(OriginalValues {
            trigger: session.attendant_trigger(),
            behavior: session.attendant_behavior(),
            prep_mode,
            tool_set,
            skill_set,
            model_setting,
            template,
        }),
        // A composing attendant opens on the prep row: the two rows above
        // it do not apply, so a cursor there would be on an inert control.
        // A running one opens on the trigger, the first field.
        focus: jinn_attendant_msg::PropertyField::opening_focus(prep_mode),
        ..AttendantPropertiesState::default()
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
