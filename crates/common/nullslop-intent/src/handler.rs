// Copyright (C) 2026 Jayson Lennon
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! The [`IntentHandler`] — a single decision point for all user input.
//!
//! Processes every [`Intent`] variant: call the validator, then act.
//! On validation failure, the handler does nothing (no-op). On success,
//! it mutates [`AppState`] directly, optionally sets TUI signals, and
//! returns [`IntentResult`] carrying commands for the actor system.

#![allow(
    clippy::missing_docs_in_private_items,
    reason = "Phase 2 transitional — Phase 4 refactors handler into per-intent modules"
)]
#![allow(
    clippy::doc_markdown,
    reason = "auto-idents like IntentHandler, AppState, PickerKind are meaningful names"
)]

use nullslop_component::AppState;
use nullslop_domain::PinPosition;

use crate::Intent;

use nullslop_domain::IntentResult;

/// Processes user intents — the single decision point for all user input.
///
/// For each [`Intent`] variant: call the validator, then act.
/// On validation failure, the handler does nothing (no-op).
///
/// Some intents set "TUI signals" on `state.frontend.tui_signals` — flags that the
/// outer platform layer reads after `handle()` returns and acts upon
/// (e.g., opening an external editor, toggling a popup).
pub struct IntentHandler;

impl IntentHandler {
    /// Process an intent against the current application state.
    ///
    /// Clears TUI signals from the previous call, then processes the intent.
    /// Mutates `state` directly for UI operations.
    /// Returns commands and events for the actor system.
    #[expect(
        clippy::too_many_lines,
        reason = "exhaustive match on all Intent variants"
    )]
    pub fn handle(intent: &Intent, state: &mut AppState) -> IntentResult {
        state.frontend.tui_signals.clear();

        match intent {
            // --- Chat Input ---
            Intent::InsertChar { ch } => {
                nullslop_domain::chat_input_box::intent::handle_insert_char(*ch, state)
            }
            Intent::DeleteGrapheme => nullslop_domain::chat_input_box::intent::handle_delete_grapheme(state),
            Intent::DeleteGraphemeForward => {
                nullslop_domain::chat_input_box::intent::handle_delete_grapheme_forward(state)
            }
            Intent::SubmitMessage => nullslop_domain::chat_input_box::intent::handle_submit_message(state),
            Intent::AutocompleteConfirm => {
                nullslop_domain::chat_input_box::intent::handle_autocomplete_confirm(state)
            }
            Intent::MoveCursorLeft => {
                nullslop_domain::chat_input_box::intent::handle_move_cursor_left(state)
            }
            Intent::MoveCursorRight => {
                nullslop_domain::chat_input_box::intent::handle_move_cursor_right(state)
            }
            Intent::MoveCursorToStart => {
                nullslop_domain::chat_input_box::intent::handle_move_cursor_to_start(state)
            }
            Intent::MoveCursorToEnd => {
                nullslop_domain::chat_input_box::intent::handle_move_cursor_to_end(state)
            }
            Intent::MoveCursorWordLeft => {
                nullslop_domain::chat_input_box::intent::handle_move_cursor_word_left(state)
            }
            Intent::MoveCursorWordRight => {
                nullslop_domain::chat_input_box::intent::handle_move_cursor_word_right(state)
            }
            Intent::MoveCursorUp => nullslop_domain::chat_input_box::intent::handle_move_cursor_up(state),
            Intent::MoveCursorDown => {
                nullslop_domain::chat_input_box::intent::handle_move_cursor_down(state)
            }

            // --- Navigation ---
            Intent::ScrollUp => nullslop_domain::navigation::intent::handle_scroll_up(state),
            Intent::ScrollDown => nullslop_domain::navigation::intent::handle_scroll_down(state),
            Intent::MouseScrollUp => nullslop_domain::navigation::intent::handle_mouse_scroll_up(state),
            Intent::MouseScrollDown => nullslop_domain::navigation::intent::handle_mouse_scroll_down(state),
            Intent::ScrollToTop => nullslop_domain::navigation::intent::handle_scroll_to_top(state),
            Intent::ScrollToBottom => nullslop_domain::navigation::intent::handle_scroll_to_bottom(state),
            Intent::SwitchTab { direction } => {
                nullslop_domain::navigation::intent::handle_switch_tab(state, *direction)
            }
            Intent::EditInput => nullslop_domain::navigation::intent::handle_edit_input(state),

            // --- Mode & App ---
            Intent::Quit => nullslop_domain::global::intent::handle_quit(state),
            Intent::Interrupt { session_id } => {
                nullslop_domain::global::intent::handle_interrupt(state, session_id.as_ref())
            }
            Intent::EnterInsertMode => {
                nullslop_domain::chat_input_box::intent::handle_enter_insert_mode(state)
            }
            Intent::EnterNormalMode => {
                nullslop_domain::chat_input_box::intent::handle_enter_normal_mode(state)
            }
            Intent::ToggleWhichkey => nullslop_domain::global::intent::handle_toggle_whichkey(state),
            Intent::NormalEscape => nullslop_domain::chat_input_box::intent::handle_normal_escape(state),

            // --- Picker ---
            Intent::OpenPicker { kind } => nullslop_domain::picker::intent::handle_open_picker(state, *kind),
            Intent::PickerInsertChar { ch } => {
                nullslop_domain::picker::intent::handle_insert_char(state, *ch)
            }
            Intent::PickerBackspace => nullslop_domain::picker::intent::handle_backspace(state),
            Intent::PickerConfirm => {
                let (result, maybe_intent) = nullslop_domain::picker::intent::handle_picker_confirm(state);
                if let Some(intent) = maybe_intent {
                    let redispatch = IntentHandler::handle(&intent, state);
                    IntentResult::with_commands([result.commands, redispatch.commands].concat())
                } else {
                    result
                }
            }
            Intent::PickerMoveUp => nullslop_domain::picker::intent::handle_move_up(state),
            Intent::PickerMoveDown => nullslop_domain::picker::intent::handle_move_down(state),
            Intent::PickerMoveCursorLeft => nullslop_domain::picker::intent::handle_move_cursor_left(state),
            Intent::PickerMoveCursorRight => {
                nullslop_domain::picker::intent::handle_move_cursor_right(state)
            }
            Intent::ToggleKeymapScopeFilter => {
                nullslop_domain::picker::intent::handle_toggle_keymap_scope_filter(state)
            }
            Intent::SessionNew => nullslop_domain::session::intent::handle_session_new(state),
            Intent::RefreshModels => {
                nullslop_domain::session::intent::handle_refresh_models(state)
            }
            Intent::RescanPromptTemplates => {
                nullslop_domain::session::intent::handle_rescan_prompt_templates(state)
            }

            // --- Dashboard ---
            Intent::DashboardSelectDown => nullslop_domain::dashboard::intent::handle_select_down(state),
            Intent::DashboardSelectUp => nullslop_domain::dashboard::intent::handle_select_up(state),
            Intent::DashboardSelectFirst => nullslop_domain::dashboard::intent::handle_select_first(state),
            Intent::DashboardSelectLast => nullslop_domain::dashboard::intent::handle_select_last(state),

            // --- Pinned Panel ---
            Intent::PinnedPanelToggle => nullslop_domain::pinned_panel::intent::handle_toggle(state),
            Intent::PinnedPanelOpen => nullslop_domain::pinned_panel::intent::handle_open(state),
            Intent::PinnedPanelClose => nullslop_domain::pinned_panel::intent::handle_close(state),
            Intent::PinnedPanelSelectDown => {
                nullslop_domain::pinned_panel::intent::handle_select_down(state)
            }
            Intent::PinnedPanelSelectUp => nullslop_domain::pinned_panel::intent::handle_select_up(state),
            Intent::PinnedPanelUnpin => {
                nullslop_domain::pinned_panel::intent::handle_pinned_panel_unpin(state)
            }
            Intent::PinnedPanelPinTop => {
                nullslop_domain::pinned_panel::intent::handle_pinned_panel_pin(state, PinPosition::Top)
            }
            Intent::PinnedPanelPinBottom => {
                nullslop_domain::pinned_panel::intent::handle_pinned_panel_pin(state, PinPosition::Bottom)
            }
            Intent::PinnedPanelPinRelative => {
                nullslop_domain::pinned_panel::intent::handle_pinned_panel_pin(state, PinPosition::Relative)
            }
            Intent::PinnedPanelPinCycle => {
                nullslop_domain::pinned_panel::intent::handle_pinned_panel_pin_cycle(state)
            }

            // --- Chat Entry Selection ---
            Intent::ChatEntrySelectNext => {
                nullslop_domain::chat_entry_selection::intent::handle_select_next(state)
            }
            Intent::ChatEntrySelectPrev => {
                nullslop_domain::chat_entry_selection::intent::handle_select_prev(state)
            }
            Intent::ChatEntryPinSelected => {
                nullslop_domain::chat_entry_selection::intent::handle_pin_selected(state)
            }
        }
    }
}

#[cfg(test)]
#[path = "handler_tests.rs"]
mod tests;
