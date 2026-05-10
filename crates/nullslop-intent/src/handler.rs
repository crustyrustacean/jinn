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
use nullslop_protocol::provider::CancelStream;
use nullslop_protocol::{Command, Mode, PinPosition, SessionId};

use crate::Intent;
use crate::validators::{app, chat_entry};


use nullslop_protocol::IntentResult;

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
                nsslice_chat_input_box::intent::handle_insert_char(*ch, state)
            }
            Intent::DeleteGrapheme => {
                nsslice_chat_input_box::intent::handle_delete_grapheme(state)
            }
            Intent::DeleteGraphemeForward => {
                nsslice_chat_input_box::intent::handle_delete_grapheme_forward(state)
            }
            Intent::SubmitMessage => {
                nsslice_chat_input_box::intent::handle_submit_message(state)
            }
            Intent::AutocompleteConfirm => {
                nsslice_chat_input_box::intent::handle_autocomplete_confirm(state)
            }
            Intent::MoveCursorLeft => {
                nsslice_chat_input_box::intent::handle_move_cursor_left(state)
            }
            Intent::MoveCursorRight => {
                nsslice_chat_input_box::intent::handle_move_cursor_right(state)
            }
            Intent::MoveCursorToStart => {
                nsslice_chat_input_box::intent::handle_move_cursor_to_start(state)
            }
            Intent::MoveCursorToEnd => {
                nsslice_chat_input_box::intent::handle_move_cursor_to_end(state)
            }
            Intent::MoveCursorWordLeft => {
                nsslice_chat_input_box::intent::handle_move_cursor_word_left(state)
            }
            Intent::MoveCursorWordRight => {
                nsslice_chat_input_box::intent::handle_move_cursor_word_right(state)
            }
            Intent::MoveCursorUp => {
                nsslice_chat_input_box::intent::handle_move_cursor_up(state)
            }
            Intent::MoveCursorDown => {
                nsslice_chat_input_box::intent::handle_move_cursor_down(state)
            }

            // --- Navigation ---
            Intent::ScrollUp => nsslice_navigation::intent::handle_scroll_up(state),
            Intent::ScrollDown => nsslice_navigation::intent::handle_scroll_down(state),
            Intent::MouseScrollUp => {
                nsslice_navigation::intent::handle_mouse_scroll_up(state)
            }
            Intent::MouseScrollDown => {
                nsslice_navigation::intent::handle_mouse_scroll_down(state)
            }
            Intent::ScrollToTop => nsslice_navigation::intent::handle_scroll_to_top(state),
            Intent::ScrollToBottom => {
                nsslice_navigation::intent::handle_scroll_to_bottom(state)
            }
            Intent::SwitchTab { direction } => {
                nsslice_navigation::intent::handle_switch_tab(state, *direction)
            }
            Intent::EditInput => nsslice_navigation::intent::handle_edit_input(state),

            // --- Mode & App ---
            Intent::Quit => {
                app::validate_quit(state);
                state.frontend.should_quit = true;
                IntentResult::empty()
            }
            Intent::Interrupt => handle_interrupt(state),
            Intent::SetMode { mode } => handle_set_mode(state, *mode),
            Intent::ToggleWhichkey => {
                app::validate_toggle_whichkey(state);
                state.frontend.tui_signals.toggle_whichkey = true;
                IntentResult::empty()
            }
            Intent::NormalEscape => handle_normal_escape(state),

            // --- Picker ---
            Intent::OpenPicker { kind } => {
                nsslice_picker::intent::handle_open_picker(state, *kind)
            }
            Intent::PickerInsertChar { ch } => {
                nsslice_picker::intent::handle_insert_char(state, *ch)
            }
            Intent::PickerBackspace => nsslice_picker::intent::handle_backspace(state),
            Intent::PickerConfirm => {
                let (result, maybe_intent) = nsslice_picker::intent::handle_picker_confirm(state);
                if let Some(intent) = maybe_intent {
                    let redispatch = IntentHandler::handle(&intent, state);
                    IntentResult::with_commands(
                        [result.commands, redispatch.commands].concat(),
                    )
                } else {
                    result
                }
            }
            Intent::PickerMoveUp => nsslice_picker::intent::handle_move_up(state),
            Intent::PickerMoveDown => nsslice_picker::intent::handle_move_down(state),
            Intent::PickerMoveCursorLeft => {
                nsslice_picker::intent::handle_move_cursor_left(state)
            }
            Intent::PickerMoveCursorRight => {
                nsslice_picker::intent::handle_move_cursor_right(state)
            }
            Intent::ToggleKeymapScopeFilter => {
                nsslice_picker::intent::handle_toggle_keymap_scope_filter(state)
            }
            Intent::SessionNew => handle_session_new(state),
            Intent::RefreshModels => handle_refresh_models(state),
            Intent::RescanPromptTemplates => handle_rescan_prompt_templates(state),

            // --- Dashboard ---
            Intent::DashboardSelectDown => {
                nsslice_dashboard::intent::handle_select_down(state)
            }
            Intent::DashboardSelectUp => {
                nsslice_dashboard::intent::handle_select_up(state)
            }
            Intent::DashboardSelectFirst => {
                nsslice_dashboard::intent::handle_select_first(state)
            }
            Intent::DashboardSelectLast => {
                nsslice_dashboard::intent::handle_select_last(state)
            }

            // --- Pinned Panel ---
            Intent::PinnedPanelToggle => {
                nsslice_pinned_panel::intent::handle_toggle(state)
            }
            Intent::PinnedPanelOpen => {
                nsslice_pinned_panel::intent::handle_open(state)
            }
            Intent::PinnedPanelClose => {
                nsslice_pinned_panel::intent::handle_close(state)
            }
            Intent::PinnedPanelSelectDown => {
                nsslice_pinned_panel::intent::handle_select_down(state)
            }
            Intent::PinnedPanelSelectUp => {
                nsslice_pinned_panel::intent::handle_select_up(state)
            }
            Intent::PinnedPanelUnpin => {
                nsslice_pinned_panel::intent::handle_pinned_panel_unpin(state)
            }
            Intent::PinnedPanelPinTop => {
                nsslice_pinned_panel::intent::handle_pinned_panel_pin(state, PinPosition::Top)
            }
            Intent::PinnedPanelPinBottom => {
                nsslice_pinned_panel::intent::handle_pinned_panel_pin(state, PinPosition::Bottom)
            }
            Intent::PinnedPanelPinRelative => {
                nsslice_pinned_panel::intent::handle_pinned_panel_pin(state, PinPosition::Relative)
            }
            Intent::PinnedPanelPinCycle => {
                nsslice_pinned_panel::intent::handle_pinned_panel_pin_cycle(state)
            }

            // --- Chat Entry Selection ---
            Intent::ChatEntrySelectNext => {
                nsslice_chat_entry_selection::intent::handle_select_next(state)
            }
            Intent::ChatEntrySelectPrev => {
                nsslice_chat_entry_selection::intent::handle_select_prev(state)
            }
            Intent::ChatEntryPinSelected => {
                nsslice_chat_entry_selection::intent::handle_pin_selected(state)
            }
        }
    }
}

// --- Chat input handlers ---

// --- Mode & App handlers ---

fn handle_interrupt(state: &mut AppState) -> IntentResult {
    if nsslice_chat_input_box::validator::validate_interrupt(state).is_err() {
        return IntentResult::empty();
    }

    state.active_chat_input_mut().deactivate_autocomplete();

    if state.active_chat_input().is_empty() {
        let session_id = state.session.active_session.clone();
        cancel_stream_and_drain(state);
        IntentResult::with_commands(vec![Command::CancelStream {
            payload: CancelStream { session_id },
        }])
    } else {
        state.active_chat_input_mut().reset();
        IntentResult::empty()
    }
}

fn handle_set_mode(state: &mut AppState, mode: Mode) -> IntentResult {
    let mut commands = vec![];

    if state.frontend.mode == Mode::Input
        && mode == Mode::Normal
        && !state.active_session().is_idle()
    {
        let session_id = state.session.active_session.clone();
        cancel_stream_and_drain(state);
        commands.push(Command::CancelStream {
            payload: CancelStream { session_id },
        });
    }

    if state.frontend.mode == Mode::Picker && mode != Mode::Picker {
        state.frontend.active_picker_kind = None;
    }

    state.frontend.mode = mode;

    IntentResult::with_commands(commands)
}

fn handle_normal_escape(state: &mut AppState) -> IntentResult {
    app::validate_normal_escape(state);

    if state.active_session().selected_entry_index().is_some() {
        state.active_session_mut().clear_selection();
    }

    state.frontend.tui_signals.pinned_pane_close = true;

    IntentResult::empty()
}

fn handle_session_new(state: &mut AppState) -> IntentResult {
    if chat_entry::validate_session_new(state).is_err() {
        return IntentResult::empty();
    }

    state.session.sessions.remove(&state.session.active_session);

    let new_id = SessionId::new();
    state.session.sessions.insert(
        new_id.clone(),
        nullslop_component::chat_session::ChatSessionState::new(),
    );
    state.session.active_session = new_id;
    state.frontend.mode = Mode::Normal;

    IntentResult::empty()
}

fn handle_refresh_models(state: &mut AppState) -> IntentResult {
    if chat_entry::validate_refresh_models(state).is_err() {
        return IntentResult::empty();
    }

    // Post system message to active session.
    state
        .active_session_mut()
        .push_entry(nullslop_protocol::ChatEntry::system("Refreshing models..."));

    IntentResult::with_commands(vec![Command::RefreshModels])
}

fn handle_rescan_prompt_templates(state: &mut AppState) -> IntentResult {
    let _ = chat_entry::validate_rescan_prompt_templates(state);

    // Post system message to active session.
    state
        .active_session_mut()
        .push_entry(nullslop_protocol::ChatEntry::system(
            "Rescanning prompt templates...",
        ));

    IntentResult::with_commands(vec![Command::RescanPromptTemplates])
}

/// Cancels streaming on the active session and drains any queued messages
/// back to the input buffer.
fn cancel_stream_and_drain(state: &mut AppState) {
    let session = state.active_session_mut();
    session.cancel_streaming();
    let drained: Vec<String> = session.drain_queue().into_iter().collect();
    let drained_text = drained.join("\n");
    if !drained_text.is_empty() {
        session.chat_input_mut().replace_all(drained_text);
    }
}

#[cfg(test)]
#[path = "handler_tests.rs"]
mod tests;
