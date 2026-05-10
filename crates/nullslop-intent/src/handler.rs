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
use nullslop_protocol::context::{PinChatEntry, SwitchPromptStrategy};
use nullslop_protocol::provider::{CancelStream, ProviderSwitch};
use nullslop_protocol::session::SessionLoadRequested;
use nullslop_protocol::system::LoadPickerEntries;
use nullslop_protocol::{Command, Mode, PickerKind, PinPosition, SessionId, TabDirection};

use crate::Intent;
use crate::validators::{app, chat_entry, picker};

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
    /// Number of lines to scroll per keyboard step.
    const SCROLL_STEP: u16 = 10;
    /// Number of lines to scroll per mouse wheel tick.
    const MOUSE_SCROLL_STEP: u16 = 3;
    /// Maximum number of visible result rows for picker scroll clamping.
    const PICKER_MAX_VISIBLE: usize = 100;

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
            Intent::ScrollUp => {
                state.active_session_mut().scroll_up(Self::SCROLL_STEP);
                IntentResult::empty()
            }
            Intent::ScrollDown => {
                state.active_session_mut().scroll_down(Self::SCROLL_STEP);
                IntentResult::empty()
            }
            Intent::MouseScrollUp => {
                state
                    .active_session_mut()
                    .scroll_up(Self::MOUSE_SCROLL_STEP);
                IntentResult::empty()
            }
            Intent::MouseScrollDown => {
                state
                    .active_session_mut()
                    .scroll_down(Self::MOUSE_SCROLL_STEP);
                IntentResult::empty()
            }
            Intent::ScrollToTop => {
                state.active_session_mut().scroll_to_top();
                IntentResult::empty()
            }
            Intent::ScrollToBottom => {
                state.active_session_mut().scroll_to_bottom();
                IntentResult::empty()
            }
            Intent::SwitchTab { direction } => {
                state.frontend.active_tab = match direction {
                    TabDirection::Next => state.frontend.active_tab.next(),
                    TabDirection::Prev => state.frontend.active_tab.prev(),
                };
                IntentResult::empty()
            }
            Intent::EditInput => {
                state.frontend.tui_signals.edit_requested = true;
                IntentResult::empty()
            }

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
            Intent::OpenPicker { kind } => handle_open_picker(state, *kind),
            Intent::PickerInsertChar { ch } => {
                picker::validate_picker_insert_char(state, *ch);
                match state.frontend.active_picker_kind {
                    Some(PickerKind::Provider) => state.provider.provider_picker.insert_char(*ch),
                    Some(PickerKind::ContextAssembly) => {
                        state.frontend.context_strategy_picker.insert_char(*ch);
                    }
                    Some(PickerKind::Keymap) => state.frontend.keymap_picker.insert_char(*ch),
                    Some(PickerKind::Session) => state.frontend.session_picker.insert_char(*ch),
                    None => {}
                }
                IntentResult::empty()
            }
            Intent::PickerBackspace => {
                picker::validate_picker_backspace(state);
                match state.frontend.active_picker_kind {
                    Some(PickerKind::Provider) => state.provider.provider_picker.backspace(),
                    Some(PickerKind::ContextAssembly) => {
                        state.frontend.context_strategy_picker.backspace();
                    }
                    Some(PickerKind::Keymap) => state.frontend.keymap_picker.backspace(),
                    Some(PickerKind::Session) => state.frontend.session_picker.backspace(),
                    None => {}
                }
                IntentResult::empty()
            }
            Intent::PickerConfirm => handle_picker_confirm(state),
            Intent::PickerMoveUp => {
                picker::validate_picker_move_up(state);
                match state.frontend.active_picker_kind {
                    Some(PickerKind::Provider) => {
                        state
                            .provider
                            .provider_picker
                            .move_up(Self::PICKER_MAX_VISIBLE);
                    }
                    Some(PickerKind::ContextAssembly) => {
                        state
                            .frontend
                            .context_strategy_picker
                            .move_up(Self::PICKER_MAX_VISIBLE);
                    }
                    Some(PickerKind::Keymap) => {
                        state
                            .frontend
                            .keymap_picker
                            .move_up(Self::PICKER_MAX_VISIBLE);
                    }
                    Some(PickerKind::Session) => {
                        state
                            .frontend
                            .session_picker
                            .move_up(Self::PICKER_MAX_VISIBLE);
                    }
                    None => {}
                }
                IntentResult::empty()
            }
            Intent::PickerMoveDown => {
                picker::validate_picker_move_down(state);
                match state.frontend.active_picker_kind {
                    Some(PickerKind::Provider) => {
                        state
                            .provider
                            .provider_picker
                            .move_down(Self::PICKER_MAX_VISIBLE);
                    }
                    Some(PickerKind::ContextAssembly) => {
                        state
                            .frontend
                            .context_strategy_picker
                            .move_down(Self::PICKER_MAX_VISIBLE);
                    }
                    Some(PickerKind::Keymap) => {
                        state
                            .frontend
                            .keymap_picker
                            .move_down(Self::PICKER_MAX_VISIBLE);
                    }
                    Some(PickerKind::Session) => {
                        state
                            .frontend
                            .session_picker
                            .move_down(Self::PICKER_MAX_VISIBLE);
                    }
                    None => {}
                }
                IntentResult::empty()
            }
            Intent::PickerMoveCursorLeft => {
                picker::validate_picker_move_cursor_left(state);
                match state.frontend.active_picker_kind {
                    Some(PickerKind::Provider) => state.provider.provider_picker.move_cursor_left(),
                    Some(PickerKind::ContextAssembly) => {
                        state.frontend.context_strategy_picker.move_cursor_left();
                    }
                    Some(PickerKind::Keymap) => state.frontend.keymap_picker.move_cursor_left(),
                    Some(PickerKind::Session) => state.frontend.session_picker.move_cursor_left(),
                    None => {}
                }
                IntentResult::empty()
            }
            Intent::PickerMoveCursorRight => {
                picker::validate_picker_move_cursor_right(state);
                match state.frontend.active_picker_kind {
                    Some(PickerKind::Provider) => {
                        state.provider.provider_picker.move_cursor_right();
                    }
                    Some(PickerKind::ContextAssembly) => {
                        state.frontend.context_strategy_picker.move_cursor_right();
                    }
                    Some(PickerKind::Keymap) => state.frontend.keymap_picker.move_cursor_right(),
                    Some(PickerKind::Session) => state.frontend.session_picker.move_cursor_right(),
                    None => {}
                }
                IntentResult::empty()
            }
            Intent::ToggleKeymapScopeFilter => handle_toggle_keymap_scope_filter(state),
            Intent::SessionNew => handle_session_new(state),
            Intent::RefreshModels => handle_refresh_models(state),
            Intent::RescanPromptTemplates => handle_rescan_prompt_templates(state),

            // --- Dashboard ---
            Intent::DashboardSelectDown => {
                state.frontend.dashboard.select_next();
                IntentResult::empty()
            }
            Intent::DashboardSelectUp => {
                state.frontend.dashboard.select_prev();
                IntentResult::empty()
            }
            Intent::DashboardSelectFirst => {
                state.frontend.dashboard.select_first();
                IntentResult::empty()
            }
            Intent::DashboardSelectLast => {
                state.frontend.dashboard.select_last();
                IntentResult::empty()
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
                chat_entry::validate_chat_entry_select_next(state);
                state.active_session_mut().select_next_entry();
                IntentResult::empty()
            }
            Intent::ChatEntrySelectPrev => {
                chat_entry::validate_chat_entry_select_prev(state);
                state.active_session_mut().select_prev_entry();
                IntentResult::empty()
            }
            Intent::ChatEntryPinSelected => handle_chat_entry_pin_selected(state),
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

// --- Picker handlers ---

fn handle_open_picker(state: &mut AppState, kind: PickerKind) -> IntentResult {
    if picker::validate_open_picker(state, &kind).is_err() {
        return IntentResult::empty();
    }

    state.frontend.active_picker_kind = Some(kind);

    match kind {
        PickerKind::Provider => {
            state.provider.provider_picker.reset();
        }
        PickerKind::ContextAssembly => {
            state.frontend.context_strategy_picker.reset();
        }
        PickerKind::Keymap => {
            state.frontend.keymap_picker.reset();
            state.frontend.keymap_picker_show_all = false;
        }
        PickerKind::Session => {
            state.frontend.session_picker.reset();
        }
    }

    state.frontend.mode = Mode::Picker;

    // Keymap entries come from state, not services.
    if matches!(kind, PickerKind::Keymap) {
        IntentResult::empty()
    } else {
        IntentResult::with_commands(vec![Command::LoadPickerEntries {
            payload: LoadPickerEntries { kind },
        }])
    }
}

fn handle_picker_confirm(state: &mut AppState) -> IntentResult {
    if picker::validate_picker_confirm(state).is_err() {
        return IntentResult::empty();
    }

    match state.frontend.active_picker_kind {
        Some(PickerKind::Provider) => confirm_provider(state),
        Some(PickerKind::ContextAssembly) => confirm_strategy(state),
        Some(PickerKind::Keymap) => confirm_keymap(state),
        Some(PickerKind::Session) => confirm_session(state),
        None => IntentResult::empty(),
    }
}

fn confirm_provider(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.provider.provider_picker.selected_item() else {
        return IntentResult::empty();
    };
    if !entry.is_available {
        return IntentResult::empty();
    }
    let provider_id = entry.provider_id.clone();

    state.frontend.mode = Mode::Normal;
    IntentResult::with_commands(vec![Command::ProviderSwitch {
        payload: ProviderSwitch { provider_id },
    }])
}

fn confirm_strategy(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.context_strategy_picker.selected_item() else {
        return IntentResult::empty();
    };
    let strategy_id = entry.strategy_id.clone();
    let session_id = state.session.active_session.clone();

    state.set_default_strategy(strategy_id.clone());

    state.frontend.mode = Mode::Normal;
    IntentResult::with_commands(vec![Command::SwitchPromptStrategy {
        payload: SwitchPromptStrategy {
            session_id,
            strategy_id,
        },
    }])
}

fn confirm_keymap(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.keymap_picker.selected_item() else {
        return IntentResult::empty();
    };
    let intent = entry.command.clone();

    // Set mode to Normal, then execute the selected keymap intent.
    state.frontend.mode = Mode::Normal;
    IntentHandler::handle(&intent, state)
}

fn confirm_session(state: &mut AppState) -> IntentResult {
    let Some(entry) = state.frontend.session_picker.selected_item() else {
        return IntentResult::empty();
    };
    let session_id = entry.session_id.clone();
    let byte_offset = entry.byte_offset;

    state.session.session_loading = true;
    state.frontend.mode = Mode::Normal;

    IntentResult::with_commands(vec![Command::SessionLoadRequested {
        payload: SessionLoadRequested {
            session_id,
            byte_offset,
        },
    }])
}

fn handle_toggle_keymap_scope_filter(state: &mut AppState) -> IntentResult {
    picker::validate_toggle_keymap_scope_filter(state);

    state.frontend.keymap_picker_show_all = !state.frontend.keymap_picker_show_all;

    let scope = state
        .frontend
        .keymap_picker_origin_scope
        .clone()
        .unwrap_or_default();

    let filtered: Vec<_> = if state.frontend.keymap_picker_show_all {
        state.frontend.all_keymap_entries.clone()
    } else {
        state
            .frontend
            .all_keymap_entries
            .iter()
            .filter(|e| e.scope == scope)
            .cloned()
            .collect()
    };

    state.frontend.keymap_picker.set_items(filtered);
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

// --- Chat Entry handlers ---

fn handle_chat_entry_pin_selected(state: &mut AppState) -> IntentResult {
    if chat_entry::validate_chat_entry_pin_selected(state).is_err() {
        return IntentResult::empty();
    }

    let session_id = state.session.active_session.clone();
    let Some(entry_id) = state.active_session().selected_entry_id().cloned() else {
        return IntentResult::empty();
    };

    IntentResult::with_commands(vec![Command::PinChatEntry {
        payload: PinChatEntry {
            session_id,
            entry_id,
            position: PinPosition::Relative,
        },
    }])
}

#[cfg(test)]
#[path = "handler_tests.rs"]
mod tests;
