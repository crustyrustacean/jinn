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
use nullslop_component::chat_input_box::AutocompleteMatch;
use nullslop_component::prompt_template::PromptTemplateStore;
use nullslop_protocol::context::{PinChatEntry, SwitchPromptStrategy, UnpinChatEntry};
use nullslop_protocol::provider::{CancelStream, ProviderSwitch};
use nullslop_protocol::session::SessionLoadRequested;
use nullslop_protocol::system::LoadPickerEntries;
use nullslop_protocol::{Command, Mode, PickerKind, PinPosition, SessionId, TabDirection};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::Intent;
use crate::validators::{app, chat_entry, chat_input, picker, pinned_panel};

/// What the [`IntentHandler`] returns after processing an intent.
#[derive(Debug)]
pub struct IntentResult {
    /// Commands to send to the actor system.
    pub commands: Vec<Command>,
}

impl IntentResult {
    /// An empty result with no commands.
    #[must_use]
    pub fn empty() -> Self {
        Self { commands: vec![] }
    }

    /// A result with commands.
    #[must_use]
    pub fn with_commands(commands: Vec<Command>) -> Self {
        Self { commands }
    }
}

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
            Intent::InsertChar { ch } => handle_insert_char(*ch, state),
            Intent::DeleteGrapheme => handle_delete_grapheme(state),
            Intent::DeleteGraphemeForward => handle_delete_grapheme_forward(state),
            Intent::SubmitMessage => handle_submit_message(state),
            Intent::AutocompleteConfirm => handle_autocomplete_confirm(state),
            Intent::MoveCursorLeft => {
                state.active_chat_input_mut().move_cursor_left();
                let should_deactivate = should_deactivate_on_cursor_move(state);
                if should_deactivate {
                    state.active_chat_input_mut().deactivate_autocomplete();
                }
                IntentResult::empty()
            }
            Intent::MoveCursorRight => {
                state.active_chat_input_mut().move_cursor_right();
                let should_deactivate = should_deactivate_on_cursor_move(state);
                if should_deactivate {
                    state.active_chat_input_mut().deactivate_autocomplete();
                }
                IntentResult::empty()
            }
            Intent::MoveCursorToStart => {
                state.active_chat_input_mut().deactivate_autocomplete();
                state.active_chat_input_mut().move_cursor_to_start();
                IntentResult::empty()
            }
            Intent::MoveCursorToEnd => {
                state.active_chat_input_mut().deactivate_autocomplete();
                state.active_chat_input_mut().move_cursor_to_end();
                IntentResult::empty()
            }
            Intent::MoveCursorWordLeft => {
                state.active_chat_input_mut().deactivate_autocomplete();
                state.active_chat_input_mut().move_cursor_word_left();
                IntentResult::empty()
            }
            Intent::MoveCursorWordRight => {
                state.active_chat_input_mut().deactivate_autocomplete();
                state.active_chat_input_mut().move_cursor_word_right();
                IntentResult::empty()
            }
            Intent::MoveCursorUp => {
                if state.active_chat_input().autocomplete().is_some() {
                    state.active_chat_input_mut().autocomplete_move_up();
                } else {
                    state.active_chat_input_mut().move_cursor_up();
                }
                IntentResult::empty()
            }
            Intent::MoveCursorDown => {
                if state.active_chat_input().autocomplete().is_some() {
                    state.active_chat_input_mut().autocomplete_move_down();
                } else {
                    state.active_chat_input_mut().move_cursor_down();
                }
                IntentResult::empty()
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
                state.frontend.tui_signals.pinned_pane_toggle = true;
                IntentResult::empty()
            }
            Intent::PinnedPanelOpen => {
                state.frontend.tui_signals.pinned_pane_open = true;
                IntentResult::empty()
            }
            Intent::PinnedPanelClose => {
                state.frontend.tui_signals.pinned_pane_close = true;
                IntentResult::empty()
            }
            Intent::PinnedPanelSelectDown => {
                let sorted_ids = state.sorted_pinned_ids();
                state.frontend.pinned_panel.select_next(&sorted_ids);
                IntentResult::empty()
            }
            Intent::PinnedPanelSelectUp => {
                let sorted_ids = state.sorted_pinned_ids();
                state.frontend.pinned_panel.select_prev(&sorted_ids);
                IntentResult::empty()
            }
            Intent::PinnedPanelUnpin => handle_pinned_panel_unpin(state),
            Intent::PinnedPanelPinTop => handle_pinned_panel_pin(state, PinPosition::Top),
            Intent::PinnedPanelPinBottom => handle_pinned_panel_pin(state, PinPosition::Bottom),
            Intent::PinnedPanelPinRelative => handle_pinned_panel_pin(state, PinPosition::Relative),
            Intent::PinnedPanelPinCycle => handle_pinned_panel_pin_cycle(state),

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

fn handle_insert_char(ch: char, state: &mut AppState) -> IntentResult {
    let is_autocomplete_active = state.active_chat_input().autocomplete().is_some();

    if is_autocomplete_active {
        state.active_chat_input_mut().insert_grapheme_at_cursor(ch);

        match ch {
            ' ' => {
                state.active_chat_input_mut().deactivate_autocomplete();
            }
            '$' => {
                let Some(token_start) = state.active_chat_input().autocomplete_token_start() else {
                    return IntentResult::empty();
                };
                let cursor_before_insert = state.active_chat_input().cursor_pos() - 1;
                let filter: String = state
                    .active_chat_input()
                    .text()
                    .graphemes(true)
                    .enumerate()
                    .skip_while(|(i, _)| *i < token_start + 1)
                    .take_while(|(i, _)| *i < cursor_before_insert)
                    .map(|(_, g)| g)
                    .collect();
                if let Some(template) = state.context.prompt_templates.find_by_name(&filter) {
                    let body = template.body.clone();
                    state.active_chat_input_mut().expand_autocomplete(&body);
                } else {
                    state.active_chat_input_mut().deactivate_autocomplete();
                }
            }
            _ => {
                let filter = state
                    .active_chat_input()
                    .autocomplete_filter()
                    .unwrap_or_default();
                let matches = compute_matches(&state.context.prompt_templates, &filter);
                state
                    .active_chat_input_mut()
                    .update_autocomplete_matches(matches);
            }
        }
    } else {
        state.active_chat_input_mut().insert_grapheme_at_cursor(ch);

        if ch == '$' {
            let input = state.active_chat_input();
            if is_valid_trigger_position(input) {
                let token_start = input.cursor_pos() - 1;
                let matches = compute_matches(&state.context.prompt_templates, "");
                state
                    .active_chat_input_mut()
                    .activate_autocomplete(token_start, matches);
            }
        }
    }

    IntentResult::empty()
}

fn handle_delete_grapheme(state: &mut AppState) -> IntentResult {
    let should_deactivate =
        if let Some(token_start) = state.active_chat_input().autocomplete_token_start() {
            state.active_chat_input().cursor_pos() <= token_start + 1
        } else {
            false
        };

    if should_deactivate {
        state.active_chat_input_mut().deactivate_autocomplete();
        state
            .active_chat_input_mut()
            .delete_grapheme_before_cursor();
    } else if state.active_chat_input().autocomplete().is_some() {
        state
            .active_chat_input_mut()
            .delete_grapheme_before_cursor();
        let filter = state
            .active_chat_input()
            .autocomplete_filter()
            .unwrap_or_default();
        let matches = compute_matches(&state.context.prompt_templates, &filter);
        state
            .active_chat_input_mut()
            .update_autocomplete_matches(matches);
    } else {
        state
            .active_chat_input_mut()
            .delete_grapheme_before_cursor();
    }

    IntentResult::empty()
}

fn handle_delete_grapheme_forward(state: &mut AppState) -> IntentResult {
    let token_start = state.active_chat_input().autocomplete_token_start();
    let cursor = state.active_chat_input().cursor_pos();

    if let Some(token_start) = token_start {
        if cursor == token_start {
            state.active_chat_input_mut().deactivate_autocomplete();
            state.active_chat_input_mut().delete_grapheme_after_cursor();
        } else {
            state.active_chat_input_mut().delete_grapheme_after_cursor();
            let should_deactivate = should_deactivate_on_cursor_move(state);
            if should_deactivate {
                state.active_chat_input_mut().deactivate_autocomplete();
            } else {
                let filter = state
                    .active_chat_input()
                    .autocomplete_filter()
                    .unwrap_or_default();
                let matches = compute_matches(&state.context.prompt_templates, &filter);
                state
                    .active_chat_input_mut()
                    .update_autocomplete_matches(matches);
            }
        }
    } else {
        state.active_chat_input_mut().delete_grapheme_after_cursor();
    }

    IntentResult::empty()
}

fn handle_submit_message(state: &mut AppState) -> IntentResult {
    if chat_input::validate_submit_message(state).is_err() {
        return IntentResult::empty();
    }

    let text = state.active_chat_input().text().to_owned();
    let session_id = state.session.active_session.clone();
    state.active_chat_input_mut().reset();

    IntentResult::with_commands(vec![Command::EnqueueUserMessage {
        payload: nullslop_protocol::chat_input::EnqueueUserMessage { session_id, text },
    }])
}

fn handle_autocomplete_confirm(state: &mut AppState) -> IntentResult {
    if chat_input::validate_autocomplete_confirm(state).is_ok() {
        if let Some(selected) = state.active_chat_input().autocomplete_selected() {
            let name = selected.name.clone();
            state.active_chat_input_mut().complete_autocomplete(&name);
            let filter = state
                .active_chat_input()
                .autocomplete_filter()
                .unwrap_or_default();
            let matches = compute_matches(&state.context.prompt_templates, &filter);
            state
                .active_chat_input_mut()
                .update_autocomplete_matches(matches);
        }
        IntentResult::empty()
    } else {
        // Fallback: switch tab when autocomplete is inactive.
        state.frontend.active_tab = state.frontend.active_tab.next();
        IntentResult::empty()
    }
}

// --- Mode & App handlers ---

fn handle_interrupt(state: &mut AppState) -> IntentResult {
    if chat_input::validate_interrupt(state).is_err() {
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

// --- Pinned Panel handlers ---

fn resolve_selected_entry_id(
    state: &AppState,
) -> Option<(SessionId, nullslop_protocol::ChatEntryId)> {
    let sorted_ids = state.sorted_pinned_ids();
    let index = state.frontend.pinned_panel.selection_index(&sorted_ids);
    let session_id = state.session.active_session.clone();

    let mut pinned = state.active_session().pinned_entries();
    pinned.sort_by_key(|entry| nullslop_component::app_state::pin_sort_key(entry.pin_position));

    let entry = pinned.get(index)?;
    Some((session_id, entry.id.clone()))
}

fn cycle_position(pos: PinPosition) -> PinPosition {
    match pos {
        PinPosition::Top => PinPosition::Bottom,
        PinPosition::Bottom => PinPosition::Relative,
        PinPosition::Relative => PinPosition::Top,
    }
}

fn handle_pinned_panel_unpin(state: &mut AppState) -> IntentResult {
    if pinned_panel::validate_pinned_panel_unpin(state).is_err() {
        return IntentResult::empty();
    }

    if let Some((session_id, entry_id)) = resolve_selected_entry_id(state) {
        IntentResult::with_commands(vec![Command::UnpinChatEntry {
            payload: UnpinChatEntry {
                session_id,
                entry_id,
            },
        }])
    } else {
        IntentResult::empty()
    }
}

fn handle_pinned_panel_pin(state: &mut AppState, position: PinPosition) -> IntentResult {
    if pinned_panel::validate_pinned_panel_pin_top(state).is_err() {
        return IntentResult::empty();
    }

    if let Some((session_id, entry_id)) = resolve_selected_entry_id(state) {
        IntentResult::with_commands(vec![Command::PinChatEntry {
            payload: PinChatEntry {
                session_id,
                entry_id,
                position,
            },
        }])
    } else {
        IntentResult::empty()
    }
}

fn handle_pinned_panel_pin_cycle(state: &mut AppState) -> IntentResult {
    if pinned_panel::validate_pinned_panel_pin_cycle(state).is_err() {
        return IntentResult::empty();
    }

    let sorted_ids = state.sorted_pinned_ids();
    let index = state.frontend.pinned_panel.selection_index(&sorted_ids);

    let mut pinned = state.active_session().pinned_entries();
    pinned.sort_by_key(|entry| nullslop_component::app_state::pin_sort_key(entry.pin_position));

    let Some(entry) = pinned.get(index) else {
        return IntentResult::empty();
    };

    let current = entry.pin_position.unwrap_or(PinPosition::Relative);
    let next = cycle_position(current);
    let session_id = state.session.active_session.clone();
    let entry_id = entry.id.clone();

    IntentResult::with_commands(vec![Command::PinChatEntry {
        payload: PinChatEntry {
            session_id,
            entry_id,
            position: next,
        },
    }])
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

// --- Helpers ---

fn is_valid_trigger_position(
    input: &nullslop_component::chat_input_box::ChatInputBoxState,
) -> bool {
    let dollar_pos = input.cursor_pos() - 1;
    if dollar_pos == 0 {
        return true;
    }
    input.grapheme_at(dollar_pos - 1) == Some(" ")
}

fn should_deactivate_on_cursor_move(state: &AppState) -> bool {
    let Some(ac) = state.active_chat_input().autocomplete() else {
        return false;
    };
    state.active_chat_input().cursor_pos() <= ac.token_start()
}

fn compute_matches(store: &PromptTemplateStore, filter: &str) -> Vec<AutocompleteMatch> {
    store
        .fuzzy_search(filter)
        .into_iter()
        .map(|t| AutocompleteMatch {
            name: t.name.clone(),
            description: t.description.clone(),
        })
        .collect()
}

#[cfg(test)]
#[path = "handler_tests.rs"]
mod tests;
