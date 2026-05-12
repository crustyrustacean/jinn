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

use crate::AppState;
use crate::protocol::PinPosition;

use crate::Intent;

use crate::IntentResult;

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
                crate::feat::chat_input::intent::handle_insert_char(*ch, state)
            }
            Intent::DeleteGrapheme => {
                crate::feat::chat_input::intent::handle_delete_grapheme(state)
            }
            Intent::DeleteGraphemeForward => {
                crate::feat::chat_input::intent::handle_delete_grapheme_forward(state)
            }
            Intent::SubmitMessage => crate::feat::chat_input::intent::handle_submit_message(state),
            Intent::AutocompleteConfirm => {
                crate::feat::chat_input::intent::handle_autocomplete_confirm(state)
            }
            Intent::MoveCursorLeft => {
                crate::feat::chat_input::intent::handle_move_cursor_left(state)
            }
            Intent::MoveCursorRight => {
                crate::feat::chat_input::intent::handle_move_cursor_right(state)
            }
            Intent::MoveCursorToStart => {
                crate::feat::chat_input::intent::handle_move_cursor_to_start(state)
            }
            Intent::MoveCursorToEnd => {
                crate::feat::chat_input::intent::handle_move_cursor_to_end(state)
            }
            Intent::MoveCursorWordLeft => {
                crate::feat::chat_input::intent::handle_move_cursor_word_left(state)
            }
            Intent::MoveCursorWordRight => {
                crate::feat::chat_input::intent::handle_move_cursor_word_right(state)
            }
            Intent::MoveCursorUp => crate::feat::chat_input::intent::handle_move_cursor_up(state),
            Intent::MoveCursorDown => {
                crate::feat::chat_input::intent::handle_move_cursor_down(state)
            }

            // --- Navigation ---
            Intent::ScrollUp => crate::feat::navigation::intent::handle_scroll_up(state),
            Intent::ScrollDown => crate::feat::navigation::intent::handle_scroll_down(state),
            Intent::MouseScrollUp => crate::feat::navigation::intent::handle_mouse_scroll_up(state),
            Intent::MouseScrollDown => {
                crate::feat::navigation::intent::handle_mouse_scroll_down(state)
            }
            Intent::ScrollToTop => crate::feat::navigation::intent::handle_scroll_to_top(state),
            Intent::ScrollToBottom => {
                crate::feat::navigation::intent::handle_scroll_to_bottom(state)
            }
            Intent::SwitchTab { direction } => {
                crate::feat::navigation::intent::handle_switch_tab(state, *direction)
            }
            Intent::EditInput => crate::feat::navigation::intent::handle_edit_input(state),

            // --- Mode & App ---
            Intent::Quit => crate::feat::global::intent::handle_quit(state),
            Intent::Interrupt { session_id } => {
                crate::feat::global::intent::handle_interrupt(state, session_id.as_ref())
            }
            Intent::EnterInsertMode => {
                crate::feat::chat_input::intent::handle_enter_insert_mode(state)
            }
            Intent::EnterNormalMode => {
                crate::feat::chat_input::intent::handle_enter_normal_mode(state)
            }
            Intent::ToggleWhichkey => crate::feat::global::intent::handle_toggle_whichkey(state),
            Intent::NormalEscape => crate::feat::chat_input::intent::handle_normal_escape(state),

            // --- Picker ---
            Intent::OpenPicker { kind } => {
                crate::feat::picker::intent::handle_open_picker(state, *kind)
            }
            Intent::PickerInsertChar { ch } => {
                crate::feat::picker::intent::handle_insert_char(state, *ch)
            }
            Intent::PickerBackspace => crate::feat::picker::intent::handle_backspace(state),
            Intent::PickerConfirm => {
                let (result, maybe_intent) =
                    crate::feat::picker::intent::handle_picker_confirm(state);
                if let Some(intent) = maybe_intent {
                    let redispatch = IntentHandler::handle(&intent, state);
                    IntentResult::with_commands([result.commands, redispatch.commands].concat())
                } else {
                    result
                }
            }
            Intent::PickerMoveUp => crate::feat::picker::intent::handle_move_up(state),
            Intent::PickerMoveDown => crate::feat::picker::intent::handle_move_down(state),
            Intent::PickerMoveCursorLeft => {
                crate::feat::picker::intent::handle_move_cursor_left(state)
            }
            Intent::PickerMoveCursorRight => {
                crate::feat::picker::intent::handle_move_cursor_right(state)
            }
            Intent::ToggleKeymapScopeFilter => {
                crate::feat::picker::intent::handle_toggle_keymap_scope_filter(state)
            }
            Intent::SessionNew => crate::feat::session::intent::handle_session_new(state),
            Intent::RefreshModels => crate::feat::session::intent::handle_refresh_models(state),
            Intent::RescanPromptTemplates => {
                crate::feat::session::intent::handle_rescan_prompt_templates(state)
            }

            // --- Dashboard ---
            Intent::DashboardSelectDown => {
                crate::feat::dashboard::intent::handle_select_down(state)
            }
            Intent::DashboardSelectUp => crate::feat::dashboard::intent::handle_select_up(state),
            Intent::DashboardSelectFirst => {
                crate::feat::dashboard::intent::handle_select_first(state)
            }
            Intent::DashboardSelectLast => {
                crate::feat::dashboard::intent::handle_select_last(state)
            }

            // --- Pinned Panel ---
            Intent::PinnedPanelToggle => crate::feat::pinned_panel::intent::handle_toggle(state),
            Intent::PinnedPanelOpen => crate::feat::pinned_panel::intent::handle_open(state),
            Intent::PinnedPanelClose => crate::feat::pinned_panel::intent::handle_close(state),
            Intent::PinnedPanelSelectDown => {
                crate::feat::pinned_panel::intent::handle_select_down(state)
            }
            Intent::PinnedPanelSelectUp => {
                crate::feat::pinned_panel::intent::handle_select_up(state)
            }
            Intent::PinnedPanelUnpin => {
                crate::feat::pinned_panel::intent::handle_pinned_panel_unpin(state)
            }
            Intent::PinnedPanelPinTop => {
                crate::feat::pinned_panel::intent::handle_pinned_panel_pin(state, PinPosition::Top)
            }
            Intent::PinnedPanelPinBottom => {
                crate::feat::pinned_panel::intent::handle_pinned_panel_pin(
                    state,
                    PinPosition::Bottom,
                )
            }
            Intent::PinnedPanelPinRelative => {
                crate::feat::pinned_panel::intent::handle_pinned_panel_pin(
                    state,
                    PinPosition::Relative,
                )
            }
            Intent::PinnedPanelPinCycle => {
                crate::feat::pinned_panel::intent::handle_pinned_panel_pin_cycle(state)
            }

            // --- Chat Entry Selection ---
            Intent::ChatEntrySelectNext => {
                crate::feat::chat_entry_selection::intent::handle_select_next(state)
            }
            Intent::ChatEntrySelectPrev => {
                crate::feat::chat_entry_selection::intent::handle_select_prev(state)
            }
            Intent::ChatEntryPinSelected => {
                crate::feat::chat_entry_selection::intent::handle_pin_selected(state)
            }

            // --- Tool Content Popup ---
            Intent::OpenToolContent => {
                crate::feat::ui::tool_content_popup::intent::handle_open(state)
            }
            Intent::CloseToolContent => {
                crate::feat::ui::tool_content_popup::intent::handle_close(state)
            }
            Intent::ToolContentScrollDown => {
                crate::feat::ui::tool_content_popup::intent::handle_scroll_down(state)
            }
            Intent::ToolContentScrollUp => {
                crate::feat::ui::tool_content_popup::intent::handle_scroll_up(state)
            }
        }
    }
}

#[cfg(test)]
#[path = "handler_tests.rs"]
mod tests;
