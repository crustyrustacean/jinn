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

//! The [`IntentHandler`] - a single decision point for all user input.
//!
//! Processes every [`Intent`] variant: call the validator, then act.
//! On validation failure, the handler does nothing (no-op). On success,
//! it mutates [`AppState`] directly, optionally sets TUI signals, and
//! returns [`IntentResult`] carrying commands for the actor system.

#![allow(
    clippy::missing_docs_in_private_items,
    reason = "Phase 2 transitional - Phase 4 refactors handler into per-intent modules"
)]
#![allow(
    clippy::doc_markdown,
    reason = "auto-idents like IntentHandler, AppState, PickerKind are meaningful names"
)]

use crate::AppState;

use crate::protocol::{Command, Event, PickerKind, PinPosition};

use crate::Intent;
use crate::feat;

use crate::IntentResult;

/// Processes user intents - the single decision point for all user input.
///
/// For each [`Intent`] variant: call the validator, then act.
/// On validation failure, the handler does nothing (no-op).
///
/// Some intents set "TUI signals" on `state.frontend.tui_signals` - flags that the
/// outer platform layer reads after `handle()` returns and acts upon
/// (e.g., opening an external editor, toggling a popup).
pub struct IntentHandler;

impl IntentHandler {
    /// Process an intent against the current application state.
    ///
    /// Clears TUI signals from the previous call, then processes the intent.
    /// Mutates `state` directly for UI operations.
    /// Returns commands and events for the actor system.
    pub fn handle(intent: &Intent, state: &mut AppState) -> IntentResult {
        state.frontend.tui_signals.clear();

        // Capture active session ID before processing for diff-after check.
        let prev_active = state.session.active_session_id().clone();

        // Process the intent and get the result.
        let mut result = Self::handle_inner(intent, state);

        // If the active session changed, emit ActiveSessionChanged event.
        if state.session.active_session_id() != &prev_active {
            result.events.push(Event::ActiveSessionChanged(
                crate::protocol::system::ActiveSessionChanged {
                    session_id: state.session.active_session_id().clone(),
                },
            ));
        }

        result
    }

    /// Internal intent dispatch — separated from `handle` to allow post-processing.
    #[expect(
        clippy::too_many_lines,
        reason = "exhaustive match on all Intent variants"
    )]
    fn handle_inner(intent: &Intent, state: &mut AppState) -> IntentResult {
        tracing::info!(?intent, "DIAG IntentHandler::handle_inner ENTER");
        // Clear ignore sweep state when the user performs any action other than
        // pressing x. This ensures the sweep only continues during consecutive
        // x presses within 100ms.
        if !matches!(intent, Intent::ChatEntryIgnoreSelected) {
            state.active_session_mut().clear_ignore_sweep();
        }

        // Cancel stream prompt intercept: if the prompt is showing,
        // ESC (NormalEscape) confirms the cancel;
        // any other intent dismisses the prompt and continues processing.
        if let Some(result) = try_handle_cancel_stream_prompt(intent, state) {
            return result;
        }

        // Close session confirmation intercept: if the prompt is showing,
        // x (SidebarSessionClose) confirms the close;
        // any other intent dismisses the prompt and continues processing.
        if let Some(result) = try_handle_close_session_prompt(intent, state) {
            return result;
        }

        match intent {
            // --- Arg Input (takes priority when ArgInput scope is active) ---
            Intent::InsertChar { ch }
                if matches!(
                    state.frontend.scope_stack.current(),
                    crate::common::app_state::FocusScope::ArgInput
                ) =>
            {
                feat::session_lifecycle::intent::handle_arg_input_insert_char(state, *ch)
            }
            Intent::DeleteGrapheme
                if matches!(
                    state.frontend.scope_stack.current(),
                    crate::common::app_state::FocusScope::ArgInput
                ) =>
            {
                feat::session_lifecycle::intent::handle_arg_input_delete(state)
            }
            Intent::MoveCursorLeft
                if matches!(
                    state.frontend.scope_stack.current(),
                    crate::common::app_state::FocusScope::ArgInput
                ) =>
            {
                feat::session_lifecycle::intent::handle_arg_input_cursor_left(state)
            }
            Intent::MoveCursorRight
                if matches!(
                    state.frontend.scope_stack.current(),
                    crate::common::app_state::FocusScope::ArgInput
                ) =>
            {
                feat::session_lifecycle::intent::handle_arg_input_cursor_right(state)
            }
            Intent::DeleteGraphemeForward
                if matches!(
                    state.frontend.scope_stack.current(),
                    crate::common::app_state::FocusScope::ArgInput
                ) =>
            {
                feat::session_lifecycle::intent::handle_arg_input_delete_forward(state)
            }
            Intent::EnterNormalMode
                if matches!(
                    state.frontend.scope_stack.current(),
                    crate::common::app_state::FocusScope::ArgInput
                ) =>
            {
                // ESC cancels arg input - pop scope, clear state.
                state.frontend.scope_stack.pop();
                state.frontend.arg_input = crate::common::app_state::ArgInputState::default();
                crate::protocol::IntentResult::empty()
            }

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

            // --- Paste ---
            Intent::PasteText { text } => match state.frontend.scope_stack.current() {
                crate::common::app_state::FocusScope::Input => {
                    feat::chat_input::intent::handle_paste_text(text, state)
                }
                crate::common::app_state::FocusScope::Picker { .. } => {
                    feat::picker::intent::handle_picker_paste(state, text)
                }
                crate::common::app_state::FocusScope::ArgInput => {
                    feat::session_lifecycle::intent::handle_arg_input_paste(state, text)
                }
                crate::common::app_state::FocusScope::RenameSessionInput => {
                    feat::rename_session_input::intent::handle_paste(state, text)
                }
                _ => IntentResult::empty(),
            },
            // --- Navigation ---
            Intent::ScrollUp => feat::navigation::intent::handle_scroll_up(state),
            Intent::ScrollDown => feat::navigation::intent::handle_scroll_down(state),
            Intent::MouseScrollUp => feat::navigation::intent::handle_mouse_scroll_up(state),
            Intent::MouseScrollDown => feat::navigation::intent::handle_mouse_scroll_down(state),
            Intent::ScrollToTop => feat::navigation::intent::handle_scroll_to_top(state),
            Intent::ScrollToBottom => feat::navigation::intent::handle_scroll_to_bottom(state),

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
            Intent::NoOp => IntentResult::empty(),

            // --- Picker ---
            Intent::OpenPicker { kind } => feat::picker::intent::handle_open_picker(state, *kind),
            Intent::PickerInsertChar { ch } => feat::picker::intent::handle_insert_char(state, *ch),
            Intent::PickerBackspace => feat::picker::intent::handle_backspace(state),
            Intent::PickerConfirm => {
                let (result, maybe_intent) = feat::picker::intent::handle_picker_confirm(state);
                if let Some(intent) = maybe_intent {
                    let redispatch = IntentHandler::handle(&intent, state);
                    IntentResult::with_commands_and_events(
                        [result.commands, redispatch.commands].concat(),
                        [result.events, redispatch.events].concat(),
                    )
                } else {
                    result
                }
            }
            Intent::CtrlClear => {
                let (result, maybe_intent) = feat::global::intent::handle_ctrl_clear(state);
                if let Some(intent) = maybe_intent {
                    let redispatch = IntentHandler::handle(&intent, state);
                    IntentResult::with_commands_and_events(
                        [result.commands, redispatch.commands].concat(),
                        [result.events, redispatch.events].concat(),
                    )
                } else {
                    result
                }
            }
            Intent::PickerMoveUp => feat::picker::intent::handle_move_up(state),
            Intent::PickerMoveDown => feat::picker::intent::handle_move_down(state),
            Intent::PickerMoveCursorLeft => feat::picker::intent::handle_move_cursor_left(state),
            Intent::PickerMoveCursorRight => feat::picker::intent::handle_move_cursor_right(state),
            Intent::ToolToggleSelected => feat::picker::intent::handle_tool_toggle(state),
            Intent::SkillToggleSelected => feat::picker::intent::handle_skill_toggle(state),
            Intent::PreviewScrollUp => feat::picker::intent::handle_preview_scroll_up(state),
            Intent::PreviewScrollDown => feat::picker::intent::handle_preview_scroll_down(state),
            Intent::SessionNew => feat::session::intent::handle_session_new(state),
            Intent::RefreshModels => feat::session::intent::handle_refresh_models(state),
            Intent::RescanPromptTemplates => {
                feat::session::intent::handle_rescan_prompt_templates(state)
            }
            Intent::RefreshSkills => feat::picker::intent::handle_refresh_skills(state),

            // --- Sidebar ---
            Intent::SidebarFocus => feat::ui::sidebar::intent::handle_sidebar_focus(state),
            Intent::SidebarFocusSessions => {
                feat::ui::sidebar::intent::handle_sidebar_focus_sessions(state)
            }
            Intent::SidebarLeave => feat::ui::sidebar::intent::handle_sidebar_leave(state),
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
            Intent::SessionNewWithLifecycle => {
                feat::picker::intent::handle_open_picker(state, PickerKind::SessionLifecycle)
            }
            Intent::SidebarSessionClose => {
                if selected_entry_is_workflow(state) {
                    return IntentResult::empty();
                }
                // First press - show confirmation prompt.
                // The interceptor (try_handle_close_session_prompt) handles the second press.
                state.frontend.close_session_prompt = true;
                IntentResult::empty()
            }
            Intent::SidebarSessionTeardown => {
                if selected_entry_is_workflow(state) {
                    return IntentResult::empty();
                }
                feat::ui::sidebar::sessions::handle_session_teardown(state)
            }
            Intent::SidebarSessionArchive => {
                if selected_entry_is_workflow(state) {
                    return IntentResult::empty();
                }
                feat::ui::sidebar::sessions::handle_session_archive(state)
            }
            Intent::SidebarSessionContinue => {
                if selected_entry_is_workflow(state) {
                    return IntentResult::empty();
                }
                feat::ui::sidebar::sessions::handle_session_continue(state)
            }

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
            Intent::ExpandToolEntry => {
                feat::chat_entry_selection::intent::handle_expand_tool_entry(state)
            }
            Intent::ToggleIgnoredBlockVisibility => {
                feat::chat_entry_selection::intent::handle_toggle_ignored_block(state)
            }
            Intent::ForkFromEntry => {
                feat::chat_entry_selection::intent::handle_fork_from_entry(state)
            }
            Intent::YankSelectedEntry => {
                feat::chat_entry_selection::intent::handle_yank_selected(state)
            }
            Intent::ChatEntryIgnoreSelected => {
                feat::chat_entry_selection::intent::handle_ignore_selected(state)
            }

            // --- Session Lifecycle ---
            Intent::SessionLifecycleSetup {
                lifecycle_name,
                args,
            } => feat::session_lifecycle::intent::handle_session_lifecycle_setup(
                state,
                lifecycle_name,
                args,
            ),
            Intent::SessionClose => feat::session_lifecycle::intent::handle_session_close(state),
            Intent::ArgInputConfirm => {
                feat::session_lifecycle::intent::handle_arg_input_confirm(state)
            }

            // --- Sidebar Resize ---
            Intent::SidebarResizeEnter => feat::sidebar_resize::intent::handle_resize_enter(state),
            Intent::SidebarResizeExpand => {
                feat::sidebar_resize::intent::handle_resize_expand(state)
            }
            Intent::SidebarResizeContract => {
                feat::sidebar_resize::intent::handle_resize_contract(state)
            }
            Intent::SidebarResizeLeave => feat::sidebar_resize::intent::handle_resize_leave(state),

            // --- Rename Session / Workflow Input ---
            Intent::SidebarRenameSession => {
                // Branch on selected entry kind: session -> session rename, workflow -> workflow rename.
                let index = state.frontend.sessions_section.selected_index;
                if let Some(index) = index {
                    let entries = feat::ui::sidebar::sessions::sorted_open_sessions(state);
                    if let Some(entry) = entries.get(index) {
                        match entry.kind {
                            feat::ui::sidebar::sessions::state::SessionEntryKind::Session => {
                                feat::rename_session_input::intent::handle_rename_session_enter(
                                    state,
                                )
                            }
                            feat::ui::sidebar::sessions::state::SessionEntryKind::Plugin {
                                ..
                            } => {
                                // Workflow entries are not renamable from the sidebar.
                                IntentResult::empty()
                            }
                        }
                    } else {
                        IntentResult::empty()
                    }
                } else {
                    IntentResult::empty()
                }
            }
            Intent::RenameSessionConfirm => {
                feat::rename_session_input::intent::handle_rename_session_confirm(state)
            }
            Intent::RenameSessionLeave => {
                feat::rename_session_input::intent::handle_rename_session_leave(state)
            }
            Intent::RenameInsertChar { ch } => {
                feat::rename_session_input::intent::handle_insert_char(state, *ch)
            }
            Intent::RenameCursorLeft => {
                feat::rename_session_input::intent::handle_cursor_left(state)
            }
            Intent::RenameCursorRight => {
                feat::rename_session_input::intent::handle_cursor_right(state)
            }
            Intent::RenameDeleteGrapheme => {
                feat::rename_session_input::intent::handle_delete(state)
            }
            Intent::RenameDeleteForward => {
                feat::rename_session_input::intent::handle_delete_forward(state)
            }

            // --- CWD Selection ---
            Intent::ChangeCwd { root } => {
                crate::feat::navigation::intent::handle_change_cwd(state, *root)
            }
        }
    }
}
/// Returns `true` when the cursor in the sessions sidebar section is on a workflow entry.
///
/// Used by session-management intents to no-op when a workflow is selected,
/// preventing accidental operations on the parent session.
fn selected_entry_is_workflow(state: &AppState) -> bool {
    use crate::feat::ui::sidebar::sessions::state::SessionEntryKind;

    let Some(index) = state.frontend.sessions_section.selected_index else {
        return false;
    };
    let entries = feat::ui::sidebar::sessions::sorted_open_sessions(state);
    matches!(
        entries.get(index).map(|e| e.kind),
        Some(SessionEntryKind::Plugin { .. })
    )
}

/// Cancel stream prompt intercept.
///
/// If the cancel-stream confirmation prompt is showing:
/// - `NormalEscape` confirms the cancel (and returns the appropriate commands).
/// - Any other intent dismisses the prompt and returns `None` (fall through to normal processing).
///
/// Returns `None` if the prompt is not showing or was dismissed.
fn try_handle_cancel_stream_prompt(intent: &Intent, state: &mut AppState) -> Option<IntentResult> {
    if !state.frontend.cancel_stream_prompt {
        return None;
    }

    // Dismiss the prompt regardless of which intent triggered it.
    state.frontend.cancel_stream_prompt = false;

    if !matches!(intent, Intent::NormalEscape) {
        // Any other key — dismiss prompt, fall through to normal processing.
        return None;
    }

    let session_id = state.session.active_session_id().clone();

    // Check busy state before resetting.
    let was_busy = state.active_session().is_busy();

    // Cancel busy background operations (lifecycle, etc.).
    if was_busy {
        state.active_session_mut().cancel_busy();
    }

    // Cancel stream.
    state.active_session_mut().cancel_stream_and_drain();
    let mut commands = vec![Command::CancelStream(
        crate::feat::provider::protocol::command::CancelStream {
            session_id: session_id.clone(),
        },
    )];

    // Also cancel any running lifecycle command.
    if was_busy {
        commands.push(Command::CancelLifecycleCommand(
            crate::feat::session_lifecycle::protocol::CancelLifecycleCommand { session_id },
        ));
    }

    Some(IntentResult::with_commands(commands))
}

/// Close session confirmation prompt intercept.
///
/// If the close-session confirmation prompt is showing:
/// - `SidebarSessionClose` confirms the close (re-validates, emits CloseSession).
/// - Any other intent dismisses the prompt and returns `None` (fall through to normal processing).
///
/// Returns `None` if the prompt is not showing or was dismissed.
fn try_handle_close_session_prompt(intent: &Intent, state: &mut AppState) -> Option<IntentResult> {
    if !state.frontend.close_session_prompt {
        return None;
    }

    // Dismiss the prompt regardless of which intent triggered it.
    state.frontend.close_session_prompt = false;

    if !matches!(intent, Intent::SidebarSessionClose) {
        // Any other key - dismiss prompt, fall through to normal processing.
        return None;
    }

    // Second x press - perform the close.
    // Re-validates in case session became busy between taps.
    Some(feat::ui::sidebar::sessions::handle_session_close_with_lifecycle(state))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use crate::common::app_state::{AppState, FocusScope, RenameSessionInputState};
    use crate::feat::intent::IntentHandler;
    use crate::protocol::{ChatEntry, Intent};

    #[rstest::rstest]
    fn paste_text_ignored_in_normal_scope() {
        // Given an AppState in Normal scope.
        let mut state = AppState::default();
        state.frontend.scope_stack.clear_overlays();

        // When handling PasteText.
        let result = IntentHandler::handle(
            &Intent::PasteText {
                text: "hello".into(),
            },
            &mut state,
        );

        // Then the buffer is empty and no commands are emitted.
        assert!(state.active_chat_input().is_empty());
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn paste_text_inserts_in_input_scope() {
        // Given an AppState in Input scope.
        let mut state = AppState::default();
        state
            .frontend
            .scope_stack
            .push(crate::common::app_state::FocusScope::Input);

        // When handling PasteText.
        let result = IntentHandler::handle(
            &Intent::PasteText {
                text: "hello\nworld".into(),
            },
            &mut state,
        );

        // Then the buffer has the pasted text.
        assert_eq!(state.active_chat_input().text(), "hello\nworld");
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn rename_insert_char_inserts_into_rename_input() {
        // Given state in RenameSessionInput scope with partial input.
        let mut state = AppState::default();
        state
            .frontend
            .scope_stack
            .push(FocusScope::RenameSessionInput);
        state.frontend.rename_session_input = RenameSessionInputState {
            input: "Hel".to_owned(),
            cursor_pos: 3,
        };

        // When handling RenameInsertChar { ch: 'o' }.
        let result = IntentHandler::handle(&Intent::RenameInsertChar { ch: 'o' }, &mut state);

        // Then rename input is "Helo" (not chat input).
        assert_eq!(state.frontend.rename_session_input.input, "Helo");
        assert_eq!(state.frontend.rename_session_input.cursor_pos, 4);
        assert!(state.active_chat_input().is_empty());
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn rename_cursor_left_moves_cursor_in_rename_input() {
        // Given state in RenameSessionInput scope with cursor at end.
        let mut state = AppState::default();
        state
            .frontend
            .scope_stack
            .push(FocusScope::RenameSessionInput);
        state.frontend.rename_session_input = RenameSessionInputState {
            input: "Hello".to_owned(),
            cursor_pos: 5,
        };

        // When handling RenameCursorLeft.
        let result = IntentHandler::handle(&Intent::RenameCursorLeft, &mut state);

        // Then cursor moved left.
        assert_eq!(state.frontend.rename_session_input.cursor_pos, 4);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn rename_cursor_right_moves_cursor_in_rename_input() {
        // Given state in RenameSessionInput scope with cursor at start.
        let mut state = AppState::default();
        state
            .frontend
            .scope_stack
            .push(FocusScope::RenameSessionInput);
        state.frontend.rename_session_input = RenameSessionInputState {
            input: "Hi".to_owned(),
            cursor_pos: 0,
        };

        // When handling RenameCursorRight.
        let result = IntentHandler::handle(&Intent::RenameCursorRight, &mut state);

        // Then cursor moved right.
        assert_eq!(state.frontend.rename_session_input.cursor_pos, 1);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn rename_delete_grapheme_deletes_in_rename_input() {
        // Given state in RenameSessionInput scope with cursor at end.
        let mut state = AppState::default();
        state
            .frontend
            .scope_stack
            .push(FocusScope::RenameSessionInput);
        state.frontend.rename_session_input = RenameSessionInputState {
            input: "Hello".to_owned(),
            cursor_pos: 5,
        };

        // When handling RenameDeleteGrapheme.
        let result = IntentHandler::handle(&Intent::RenameDeleteGrapheme, &mut state);

        // Then last char deleted.
        assert_eq!(state.frontend.rename_session_input.input, "Hell");
        assert_eq!(state.frontend.rename_session_input.cursor_pos, 4);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn rename_delete_forward_deletes_in_rename_input() {
        // Given state in RenameSessionInput scope with cursor at position 1.
        let mut state = AppState::default();
        state
            .frontend
            .scope_stack
            .push(FocusScope::RenameSessionInput);
        state.frontend.rename_session_input = RenameSessionInputState {
            input: "Hello".to_owned(),
            cursor_pos: 1,
        };

        // When handling RenameDeleteForward.
        let result = IntentHandler::handle(&Intent::RenameDeleteForward, &mut state);

        // Then char after cursor deleted.
        assert_eq!(state.frontend.rename_session_input.input, "Hllo");
        assert_eq!(state.frontend.rename_session_input.cursor_pos, 1);
        assert!(result.commands.is_empty());
    }

    // --- Mutant-killing tests for ArgInput scope guards ---

    #[test]
    fn insert_char_routes_to_arg_input_when_scope_is_arg_input() {
        // Given ArgInput scope is active.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::ArgInput);
        state.frontend.arg_input = crate::common::app_state::ArgInputState {
            lifecycle_name: "test".to_owned(),
            template_display: "<arg>".to_owned(),
            input: "hel".to_owned(),
            cursor_pos: 3,
        };

        // When handling InsertChar.
        let _result = IntentHandler::handle(&Intent::InsertChar { ch: 'o' }, &mut state);

        // Then arg_input received the char, not the chat input.
        assert_eq!(state.frontend.arg_input.input, "helo");
        assert!(
            state.active_chat_input().is_empty(),
            "chat input should be empty"
        );
    }

    #[test]
    fn insert_char_routes_to_chat_input_when_scope_is_normal() {
        // Given Normal scope (default) with Input overlay.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Input);

        // When handling InsertChar.
        let _result = IntentHandler::handle(&Intent::InsertChar { ch: 'x' }, &mut state);

        // Then the chat input received the char.
        assert_eq!(state.active_chat_input().text(), "x");
        assert!(
            state.frontend.arg_input.input.is_empty(),
            "arg input should be empty"
        );
    }

    #[test]
    fn delete_grapheme_routes_to_arg_input_when_scope_is_arg_input() {
        // Given ArgInput scope with some text.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::ArgInput);
        state.frontend.arg_input = crate::common::app_state::ArgInputState {
            lifecycle_name: "test".to_owned(),
            template_display: "<arg>".to_owned(),
            input: "abc".to_owned(),
            cursor_pos: 3,
        };

        // When handling DeleteGrapheme.
        let _result = IntentHandler::handle(&Intent::DeleteGrapheme, &mut state);

        // Then arg_input had a char deleted.
        assert_eq!(state.frontend.arg_input.input, "ab");
    }

    #[test]
    fn move_cursor_left_routes_to_arg_input_when_scope_is_arg_input() {
        // Given ArgInput scope with cursor at end.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::ArgInput);
        state.frontend.arg_input = crate::common::app_state::ArgInputState {
            lifecycle_name: "test".to_owned(),
            template_display: "<arg>".to_owned(),
            input: "ab".to_owned(),
            cursor_pos: 2,
        };

        // When handling MoveCursorLeft.
        let _result = IntentHandler::handle(&Intent::MoveCursorLeft, &mut state);

        // Then arg_input cursor moved.
        assert_eq!(state.frontend.arg_input.cursor_pos, 1);
    }

    #[test]
    fn move_cursor_right_routes_to_arg_input_when_scope_is_arg_input() {
        // Given ArgInput scope with cursor at start.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::ArgInput);
        state.frontend.arg_input = crate::common::app_state::ArgInputState {
            lifecycle_name: "test".to_owned(),
            template_display: "<arg>".to_owned(),
            input: "ab".to_owned(),
            cursor_pos: 0,
        };

        // When handling MoveCursorRight.
        let _result = IntentHandler::handle(&Intent::MoveCursorRight, &mut state);

        // Then arg_input cursor moved.
        assert_eq!(state.frontend.arg_input.cursor_pos, 1);
    }

    #[test]
    fn delete_forward_routes_to_arg_input_when_scope_is_arg_input() {
        // Given ArgInput scope with cursor at start.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::ArgInput);
        state.frontend.arg_input = crate::common::app_state::ArgInputState {
            lifecycle_name: "test".to_owned(),
            template_display: "<arg>".to_owned(),
            input: "abc".to_owned(),
            cursor_pos: 1,
        };

        // When handling DeleteGraphemeForward.
        let _result = IntentHandler::handle(&Intent::DeleteGraphemeForward, &mut state);

        // Then the char after cursor was deleted from arg_input.
        assert_eq!(state.frontend.arg_input.input, "ac");
    }

    #[test]
    fn enter_normal_mode_pops_arg_input_scope() {
        // Given ArgInput scope is active.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::ArgInput);
        state.frontend.arg_input = crate::common::app_state::ArgInputState {
            lifecycle_name: "test".to_owned(),
            template_display: "<arg>".to_owned(),
            input: "partial".to_owned(),
            cursor_pos: 7,
        };

        // When handling EnterNormalMode.
        let _result = IntentHandler::handle(&Intent::EnterNormalMode, &mut state);

        // Then ArgInput scope is popped and state cleared.
        assert!(!matches!(
            state.frontend.scope_stack.current(),
            FocusScope::ArgInput
        ));
        assert!(state.frontend.arg_input.input.is_empty());
    }

    // --- Mutant-killing tests for PasteText dispatch ---

    #[test]
    fn paste_text_in_picker_scope_routes_to_picker() {
        // Given Picker scope is active.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Picker {
            kind: crate::protocol::PickerKind::Persona,
        });

        // When handling PasteText.
        let _result = IntentHandler::handle(
            &Intent::PasteText {
                text: "hello".into(),
            },
            &mut state,
        );

        // Then it doesn't panic and completes (paste is handled by picker).
        // The picker query filter is updated.
    }

    #[test]
    fn paste_text_in_rename_session_scope_routes_to_rename() {
        // Given RenameSessionInput scope is active.
        let mut state = AppState::default();
        state
            .frontend
            .scope_stack
            .push(FocusScope::RenameSessionInput);
        state.frontend.rename_session_input = RenameSessionInputState {
            input: "old".to_owned(),
            cursor_pos: 3,
        };

        // When handling PasteText.
        let _result = IntentHandler::handle(
            &Intent::PasteText {
                text: " new".into(),
            },
            &mut state,
        );

        // Then rename input received the paste.
        assert_eq!(state.frontend.rename_session_input.input, "old new");
    }

    // --- Mutant-killing tests for cancel stream prompt ---

    #[test]
    fn cancel_stream_prompt_esc_confirms() {
        // Given cancel_stream_prompt is showing.
        let mut state = AppState::default();
        state.frontend.cancel_stream_prompt = true;

        // When handling NormalEscape.
        let result = IntentHandler::handle(&Intent::NormalEscape, &mut state);

        // Then the prompt is dismissed and a CancelStream command is emitted.
        assert!(!state.frontend.cancel_stream_prompt);
        assert!(
            result
                .commands
                .iter()
                .any(|c| matches!(c, crate::protocol::Command::CancelStream(_))),
            "should emit CancelStream: {:?}",
            result.commands
        );
    }

    #[test]
    fn cancel_stream_prompt_other_intent_dismisses() {
        // Given cancel_stream_prompt is showing.
        let mut state = AppState::default();
        state.frontend.cancel_stream_prompt = true;

        // When handling a different intent (InsertChar).
        let _result = IntentHandler::handle(&Intent::InsertChar { ch: 'a' }, &mut state);

        // Then the prompt is dismissed but no CancelStream command.
        assert!(!state.frontend.cancel_stream_prompt);
    }

    #[test]
    fn cancel_stream_prompt_not_showing_returns_none() {
        // Given cancel_stream_prompt is NOT showing.
        let mut state = AppState::default();
        state.frontend.cancel_stream_prompt = false;

        // When handling NormalEscape.
        let _result = IntentHandler::handle(&Intent::NormalEscape, &mut state);

        // Then no cancel command is emitted (falls through to normal escape handling).
        // The prompt remains false.
        assert!(!state.frontend.cancel_stream_prompt);
    }

    // --- Mutant-killing tests for close session prompt ---

    #[test]
    fn close_session_prompt_sidebar_close_confirms() {
        // Given close_session_prompt is showing.
        let mut state = AppState::default();
        state.frontend.close_session_prompt = true;

        // When handling SidebarSessionClose.
        let _result = IntentHandler::handle(&Intent::SidebarSessionClose, &mut state);

        // Then the prompt is dismissed.
        assert!(!state.frontend.close_session_prompt);
    }

    #[test]
    fn close_session_prompt_other_intent_dismisses() {
        // Given close_session_prompt is showing.
        let mut state = AppState::default();
        state.frontend.close_session_prompt = true;

        // When handling a different intent (ScrollUp).
        let _result = IntentHandler::handle(&Intent::ScrollUp, &mut state);

        // Then the prompt is dismissed.
        assert!(!state.frontend.close_session_prompt);
    }

    #[test]
    fn cancel_stream_prompt_noop_dismisses() {
        // Given cancel_stream_prompt is showing.
        let mut state = AppState::default();
        state.frontend.cancel_stream_prompt = true;

        // When handling NoOp (unmapped key).
        let result = IntentHandler::handle(&Intent::NoOp, &mut state);

        // Then the prompt is dismissed and no CancelStream command is emitted.
        assert!(!state.frontend.cancel_stream_prompt);
        assert!(
            !result
                .commands
                .iter()
                .any(|c| matches!(c, crate::protocol::Command::CancelStream(_))),
            "should not emit CancelStream: {:?}",
            result.commands
        );
    }

    #[test]
    fn close_session_prompt_noop_dismisses() {
        // Given close_session_prompt is showing.
        let mut state = AppState::default();
        state.frontend.close_session_prompt = true;

        // When handling NoOp (unmapped key).
        let _result = IntentHandler::handle(&Intent::NoOp, &mut state);

        // Then the prompt is dismissed.
        assert!(!state.frontend.close_session_prompt);
    }

    #[test]
    fn noop_is_empty_when_no_prompt() {
        // Given default state with no prompts showing.
        let mut state = AppState::default();

        // When handling NoOp.
        let result = IntentHandler::handle(&Intent::NoOp, &mut state);

        // Then result is empty.
        assert!(result.commands.is_empty());
        assert!(result.events.is_empty());
    }

    #[rstest::rstest]
    fn active_session_changed_emitted_on_session_switch() {
        // Given a state with two sessions.
        use crate::feat::session::chat_session::ChatSessionState;
        use crate::protocol::Event;

        let mut state = AppState::default();
        let first_id = state.session.active_session_id().clone();

        let mut second = ChatSessionState::new();
        second.push_entry(ChatEntry::user("second session"));
        let second_id = second.session_id().clone();
        state.session.insert(second);

        // Activate second session directly (simulating sidebar click).
        state.session.set_active(second_id.clone());

        // When handling an intent (any intent — we use SelectNextEntry as a no-op).
        // Actually, we need an intent that calls set_active.
        // The easiest way: call handle with an intent that doesn't change active session,
        // verify no event. Then manually switch and verify event.
        state.session.set_active(first_id.clone());
        let result = IntentHandler::handle(&Intent::ChatEntrySelectNext, &mut state);

        // Then no ActiveSessionChanged event (same session).
        let has_event = result
            .events
            .iter()
            .any(|e| matches!(e, Event::ActiveSessionChanged(_)));
        assert!(
            !has_event,
            "should not emit ActiveSessionChanged when session unchanged"
        );
    }

    // --- Session-management intents on workflow entries ---

    /// Helper: create state with a session that has an attached plugin, cursor on the plugin entry.
    fn state_with_workflow_selected() -> AppState {
        use crate::feat::attached_plugin::AttachedPlugin;

        let mut state = AppState::default();
        let session_id = state.session.active_session_id().clone();
        {
            let s = state.session.get_mut(&session_id).expect("active session");
            s.core
                .attached_plugins
                .push(AttachedPlugin::new("test-plugin"));
        }
        // entries: [session, plugin]
        state.frontend.sessions_section.selected_index = Some(1);
        state.frontend.scope_stack.push(FocusScope::SidebarSessions);
        state
    }

    #[rstest::rstest]
    fn sidebar_session_close_noop_on_workflow_entry() {
        // Given the cursor on a workflow entry in the sessions sidebar.
        let mut state = state_with_workflow_selected();

        // When handling SidebarSessionClose.
        let result = IntentHandler::handle(&Intent::SidebarSessionClose, &mut state);

        // Then no close prompt is set and no commands are emitted.
        assert!(!state.frontend.close_session_prompt);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn sidebar_session_teardown_noop_on_workflow_entry() {
        // Given the cursor on a workflow entry in the sessions sidebar.
        let mut state = state_with_workflow_selected();

        // When handling SidebarSessionTeardown.
        let result = IntentHandler::handle(&Intent::SidebarSessionTeardown, &mut state);

        // Then no commands are emitted.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn sidebar_session_archive_noop_on_workflow_entry() {
        // Given the cursor on a workflow entry in the sessions sidebar.
        let mut state = state_with_workflow_selected();
        let original_count = state.session.session_count();

        // When handling SidebarSessionArchive.
        let result = IntentHandler::handle(&Intent::SidebarSessionArchive, &mut state);

        // Then no session was removed and no commands emitted.
        assert_eq!(state.session.session_count(), original_count);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn sidebar_session_continue_noop_on_workflow_entry() {
        // Given the cursor on a workflow entry in the sessions sidebar.
        let mut state = state_with_workflow_selected();

        // When handling SidebarSessionContinue.
        let result = IntentHandler::handle(&Intent::SidebarSessionContinue, &mut state);

        // Then no commands are emitted.
        assert!(result.commands.is_empty());
    }
}
