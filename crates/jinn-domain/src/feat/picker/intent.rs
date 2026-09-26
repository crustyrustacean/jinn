//! Picker intent handlers - navigation, filtering, confirmation, and scope toggling.
//!
//! Handles all picker intents: open, insert char, backspace, confirm, move up/down,
//! and cursor movement. Confirmation is owned entirely by the active spec's
//! confirm hook — there is no intent re-dispatch, so no handler here returns a
//! follow-up intent.

use crate::common::app_state::AppState;
use jinn_slices::FocusScope;

use crate::protocol::{IntentResult, PickerKind};

use super::geometry::active_viewport;
use super::validator;

/// Opens a picker of the given kind. Sets mode to Picker and optionally
/// requests picker entries from the actor system.
pub fn handle_open_picker(
    state: &mut AppState,
    kind: PickerKind,
    pickers: &jinn_picker::PickerRegistry,
) -> IntentResult {
    if validator::validate_open_picker(state, &kind).is_err() {
        return IntentResult::empty();
    }

    // Slice-owned pickers push their own dynamic scope. The kernel resolves
    // the id to a registered slice scope and does nothing else: the slice
    // renders and acts on its own state, so the kernel never branches on
    // which picker it was.
    if let Some(scope) = slice_owned_picker_scope(kind) {
        state.frontend.scope_push(FocusScope::Dynamic(scope));
        return IntentResult::empty();
    }

    state.frontend.scope_push(FocusScope::Picker { kind });

    // Every kind is spec-driven: the open hook owns open-time preparation.
    // An empty registry (test seams) falls through with nothing to prepare.
    if jinn_picker::spec_id_for_kind(&kind).is_some_and(|id| pickers.get(id).is_some()) {
        return crate::feat::picker::action::run_active_hook(
            state,
            pickers,
            crate::feat::picker::action::Hook::Open,
        );
    }
    IntentResult::empty()
}

/// The dynamic scope of a picker that its owning slice has taken over.
///
/// A `None` here means the picker is still kernel-driven (a legacy
/// `FocusScope::Picker` plus a spec). Each entry is a slice-owned scope the
/// slice registered at activation; the kernel only relays the identity.
///
/// Slice-owned pickers no longer appear here at all: their own opener rows
/// push their scope directly, so the kernel names no picker. The match is kept
/// as a total function over the kinds it still handles, and will collapse to
/// `None` entirely once the last kernel-driven picker is migrated.
fn slice_owned_picker_scope(_kind: PickerKind) -> Option<jinn_slices::SliceScopeId> {
    None
}

/// Resets the preview scroll offset when the active picker's spec opts in
/// (`PreviewSpec::reset_scroll_on_selection_change`).
fn reset_preview_scroll(state: &mut AppState, registry: &jinn_picker::PickerRegistry) {
    let Some(kind) = state.frontend.picker_kind() else {
        return;
    };
    let Some(spec) = jinn_picker::spec_id_for_kind(&kind).and_then(|id| registry.get(id)) else {
        return;
    };
    if spec.resets_scroll_on_selection_change() {
        state
            .frontend
            .pickers
            .pickers_scrolls
            .reset(jinn_picker::PickerId::new(spec.id().as_str()));
    }
}

pub fn handle_insert_char(state: &mut AppState, ch: char) -> IntentResult {
    validator::validate_picker_insert_char(state, ch);
    if let Some(picker) = super::host_impl::active_picker_ops(state) {
        picker.insert_char(ch);
    }
    IntentResult::empty()
}

/// Handles `PasteText` in picker scope - bulk inserts pasted text into the filter.
///
/// Newlines are stripped by the picker's `insert_text` method since the filter
/// is a single-line input.
pub fn handle_picker_paste(state: &mut AppState, text: &str) -> IntentResult {
    if let Some(picker) = super::host_impl::active_picker_ops(state) {
        picker.insert_text(text);
    }
    IntentResult::empty()
}

/// Removes the last character from the active picker's filter.
pub fn handle_backspace(state: &mut AppState) -> IntentResult {
    validator::validate_picker_backspace(state);
    if let Some(picker) = super::host_impl::active_picker_ops(state) {
        picker.backspace();
    }
    IntentResult::empty()
}

/// Confirms the active picker selection.
///
/// Every kind is spec-driven: the spec's confirm hook owns confirm behavior.
pub fn handle_picker_confirm(
    state: &mut AppState,
    pickers: &jinn_picker::PickerRegistry,
) -> IntentResult {
    if validator::validate_picker_confirm(state).is_err() {
        return IntentResult::empty();
    }

    // An empty registry (test seams) falls through with nothing to do.
    if state
        .frontend
        .picker_kind()
        .as_ref()
        .and_then(jinn_picker::spec_id_for_kind)
        .is_some_and(|id| pickers.get(id).is_some())
    {
        return crate::feat::picker::action::run_active_hook(
            state,
            pickers,
            crate::feat::picker::action::Hook::Confirm,
        );
    }
    IntentResult::empty()
}

/// Moves the selection up in the active picker.
pub fn handle_move_up(state: &mut AppState, pickers: &jinn_picker::PickerRegistry) -> IntentResult {
    validator::validate_picker_move_up(state);
    let viewport = active_viewport(state);
    if let Some(picker) = super::host_impl::active_picker_ops(state) {
        picker.move_up(viewport);
    }
    reset_preview_scroll(state, pickers);
    crate::feat::picker::action::run_selection_change(state, pickers);
    IntentResult::empty()
}

/// Moves the selection down in the active picker.
pub fn handle_move_down(
    state: &mut AppState,
    pickers: &jinn_picker::PickerRegistry,
) -> IntentResult {
    validator::validate_picker_move_down(state);
    let viewport = active_viewport(state);
    if let Some(picker) = super::host_impl::active_picker_ops(state) {
        picker.move_down(viewport);
    }
    reset_preview_scroll(state, pickers);
    crate::feat::picker::action::run_selection_change(state, pickers);
    IntentResult::empty()
}

/// Pages the selection up by half the visible window in the active picker.
pub fn handle_page_up(state: &mut AppState, pickers: &jinn_picker::PickerRegistry) -> IntentResult {
    validator::validate_picker_page_up(state);
    let viewport = active_viewport(state);
    if let Some(picker) = super::host_impl::active_picker_ops(state) {
        picker.page_up(viewport);
    }
    reset_preview_scroll(state, pickers);
    crate::feat::picker::action::run_selection_change(state, pickers);
    IntentResult::empty()
}

/// Pages the selection down by half the visible window in the active picker.
pub fn handle_page_down(
    state: &mut AppState,
    pickers: &jinn_picker::PickerRegistry,
) -> IntentResult {
    validator::validate_picker_page_down(state);
    let viewport = active_viewport(state);
    if let Some(picker) = super::host_impl::active_picker_ops(state) {
        picker.page_down(viewport);
    }
    reset_preview_scroll(state, pickers);
    crate::feat::picker::action::run_selection_change(state, pickers);
    IntentResult::empty()
}

/// Moves the filter cursor left in the active picker.
pub fn handle_move_cursor_left(state: &mut AppState) -> IntentResult {
    validator::validate_picker_move_cursor_left(state);
    if let Some(picker) = super::host_impl::active_picker_ops(state) {
        picker.move_cursor_left();
    }
    IntentResult::empty()
}

/// Moves the filter cursor right in the active picker.
pub fn handle_move_cursor_right(state: &mut AppState) -> IntentResult {
    validator::validate_picker_move_cursor_right(state);
    if let Some(picker) = super::host_impl::active_picker_ops(state) {
        picker.move_cursor_right();
    }
    IntentResult::empty()
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
}
