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
use crate::protocol::{Command, PinPosition};

use crate::Intent;
use crate::feat;

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

        // Cancel stream prompt intercept: if the prompt is showing,
        // ESC (NormalEscape or SidebarLeave) confirms the cancel;
        // any other intent dismisses the prompt and continues processing.
        if state.frontend.cancel_stream_prompt {
            state.frontend.cancel_stream_prompt = false;
            if matches!(intent, Intent::NormalEscape | Intent::SidebarLeave) {
                // Second ESC — cancel the stream.
                let session_id = state.session.active_session.clone();
                state.active_session_mut().cancel_stream_and_drain();
                return IntentResult::with_commands(vec![Command::CancelStream(
                    crate::feat::provider::protocol::command::CancelStream { session_id },
                )]);
            }
            // Any other key — dismiss prompt, fall through to normal processing.
        }

        match intent {
            // --- Chat Input ---
            Intent::InsertChar { ch } => feat::chat_input::intent::handle_insert_char(*ch, state),
            Intent::DeleteGrapheme => feat::chat_input::intent::handle_delete_grapheme(state),
            Intent::DeleteGraphemeForward => {
                feat::chat_input::intent::handle_delete_grapheme_forward(state)
            }
            Intent::SubmitMessage => feat::chat_input::intent::handle_submit_message(state),
            Intent::AutocompleteConfirm => {
                feat::chat_input::intent::handle_autocomplete_confirm(state)
            }
            Intent::MoveCursorLeft => feat::chat_input::intent::handle_move_cursor_left(state),
            Intent::MoveCursorRight => feat::chat_input::intent::handle_move_cursor_right(state),
            Intent::MoveCursorToStart => {
                feat::chat_input::intent::handle_move_cursor_to_start(state)
            }
            Intent::MoveCursorToEnd => feat::chat_input::intent::handle_move_cursor_to_end(state),
            Intent::MoveCursorWordLeft => {
                feat::chat_input::intent::handle_move_cursor_word_left(state)
            }
            Intent::MoveCursorWordRight => {
                feat::chat_input::intent::handle_move_cursor_word_right(state)
            }
            Intent::MoveCursorUp => feat::chat_input::intent::handle_move_cursor_up(state),
            Intent::MoveCursorDown => feat::chat_input::intent::handle_move_cursor_down(state),

            // --- Navigation ---
            Intent::ScrollUp => feat::navigation::intent::handle_scroll_up(state),
            Intent::ScrollDown => feat::navigation::intent::handle_scroll_down(state),
            Intent::MouseScrollUp => feat::navigation::intent::handle_mouse_scroll_up(state),
            Intent::MouseScrollDown => feat::navigation::intent::handle_mouse_scroll_down(state),
            Intent::ScrollToTop => feat::navigation::intent::handle_scroll_to_top(state),
            Intent::ScrollToBottom => feat::navigation::intent::handle_scroll_to_bottom(state),
            Intent::SwitchTab { direction } => {
                feat::navigation::intent::handle_switch_tab(state, *direction)
            }
            Intent::EditInput => feat::navigation::intent::handle_edit_input(state),

            // --- Mode & App ---
            Intent::Quit => feat::global::intent::handle_quit(state),
            Intent::Interrupt { session_id } => {
                feat::global::intent::handle_interrupt(state, session_id.as_ref())
            }
            Intent::EnterInsertMode => feat::chat_input::intent::handle_enter_insert_mode(state),
            Intent::EnterNormalMode => feat::chat_input::intent::handle_enter_normal_mode(state),
            Intent::ToggleWhichkey => feat::global::intent::handle_toggle_whichkey(state),
            Intent::NormalEscape => feat::chat_input::intent::handle_normal_escape(state),

            // --- Picker ---
            Intent::OpenPicker { kind } => feat::picker::intent::handle_open_picker(state, *kind),
            Intent::PickerInsertChar { ch } => feat::picker::intent::handle_insert_char(state, *ch),
            Intent::PickerBackspace => feat::picker::intent::handle_backspace(state),
            Intent::PickerConfirm => {
                let (result, maybe_intent) = feat::picker::intent::handle_picker_confirm(state);
                if let Some(intent) = maybe_intent {
                    let redispatch = IntentHandler::handle(&intent, state);
                    IntentResult::with_commands([result.commands, redispatch.commands].concat())
                } else {
                    result
                }
            }
            Intent::PickerMoveUp => feat::picker::intent::handle_move_up(state),
            Intent::PickerMoveDown => feat::picker::intent::handle_move_down(state),
            Intent::PickerMoveCursorLeft => feat::picker::intent::handle_move_cursor_left(state),
            Intent::PickerMoveCursorRight => feat::picker::intent::handle_move_cursor_right(state),
            Intent::ToggleKeymapScopeFilter => {
                feat::picker::intent::handle_toggle_keymap_scope_filter(state)
            }
            Intent::SessionNew => feat::session::intent::handle_session_new(state),
            Intent::RefreshModels => feat::session::intent::handle_refresh_models(state),
            Intent::RescanPromptTemplates => {
                feat::session::intent::handle_rescan_prompt_templates(state)
            }

            // --- Dashboard ---
            Intent::DashboardSelectDown => feat::dashboard::intent::handle_select_down(state),
            Intent::DashboardSelectUp => feat::dashboard::intent::handle_select_up(state),
            Intent::DashboardSelectFirst => feat::dashboard::intent::handle_select_first(state),
            Intent::DashboardSelectLast => feat::dashboard::intent::handle_select_last(state),

            // --- Sidebar ---
            Intent::SidebarFocus => {
                feat::ui::sidebar::pins::pins_section::handle_sidebar_focus(state)
            }
            Intent::SidebarLeave => {
                feat::ui::sidebar::pins::pins_section::handle_sidebar_leave(state)
            }
            Intent::SidebarMoveDown => {
                feat::ui::sidebar::navigate_sidebar(
                    &feat::ui::sidebar::SidebarIntent::MoveDown,
                    state,
                );
                IntentResult::empty()
            }
            Intent::SidebarMoveUp => {
                feat::ui::sidebar::navigate_sidebar(
                    &feat::ui::sidebar::SidebarIntent::MoveUp,
                    state,
                );
                IntentResult::empty()
            }
            Intent::SidebarSectionNext => {
                feat::ui::sidebar::jump_to_section(
                    &feat::ui::sidebar::SidebarIntent::MoveDown,
                    state,
                );
                IntentResult::empty()
            }
            Intent::SidebarSectionPrev => {
                feat::ui::sidebar::jump_to_section(
                    &feat::ui::sidebar::SidebarIntent::MoveUp,
                    state,
                );
                IntentResult::empty()
            }
            Intent::PinsUnpin => feat::ui::sidebar::pins::pins_section::handle_pins_unpin(state),
            Intent::PinsPinTop => {
                feat::ui::sidebar::pins::pins_section::handle_pins_pin(state, PinPosition::Top)
            }
            Intent::PinsPinBottom => {
                feat::ui::sidebar::pins::pins_section::handle_pins_pin(state, PinPosition::Bottom)
            }
            Intent::PinsPinRelative => {
                feat::ui::sidebar::pins::pins_section::handle_pins_pin(state, PinPosition::Relative)
            }
            Intent::PinsPinCycle => {
                feat::ui::sidebar::pins::pins_section::handle_pins_pin_cycle(state)
            }
            Intent::SidebarPersonaEdit => {
                feat::ui::sidebar::pins::pins_section::handle_sidebar_persona_edit(state)
            }
            Intent::SidebarSessionClose => feat::ui::sidebar::sessions::handle_session_close(state),
            Intent::SidebarConfirm => {
                feat::ui::sidebar::sessions::handle_session_activate(state);
                IntentResult::empty()
            }

            // --- Chat Entry Selection ---
            Intent::ChatEntrySelectNext => {
                feat::chat_entry_selection::intent::handle_select_next(state)
            }
            Intent::ChatEntrySelectPrev => {
                feat::chat_entry_selection::intent::handle_select_prev(state)
            }
            Intent::ChatEntryPinSelected => {
                feat::chat_entry_selection::intent::handle_pin_selected(state)
            }
            Intent::ExpandToolResult => {
                feat::chat_entry_selection::intent::handle_expand_tool_result(state)
            }
            Intent::ToggleForkUserFilter => {
                feat::picker::intent::handle_toggle_fork_user_filter(state)
            }
            Intent::ToggleForkAssistantFilter => {
                feat::picker::intent::handle_toggle_fork_assistant_filter(state)
            }
        }
    }
}
