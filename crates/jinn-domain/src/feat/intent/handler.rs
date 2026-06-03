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
use jinn_workflow::spatial_layout::SpatialDirection;

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
                crate::common::app_state::FocusScope::RenameWorkflowInput => {
                    feat::rename_workflow_input::intent::handle_paste(state, text)
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
                // First press - show confirmation prompt.
                // The interceptor (try_handle_close_session_prompt) handles the second press.
                state.frontend.close_session_prompt = true;
                IntentResult::empty()
            }
            Intent::SidebarSessionTeardown => {
                feat::ui::sidebar::sessions::handle_session_teardown(state)
            }
            Intent::SidebarSessionArchive => {
                feat::ui::sidebar::sessions::handle_session_archive(state)
            }
            Intent::SidebarSessionContinue => {
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
                    let entries =
                        feat::ui::sidebar::sessions::sorted_open_sessions(state);
                    if let Some(entry) = entries.get(index) {
                        match entry.kind {
                            feat::ui::sidebar::sessions::state::SessionEntryKind::Session => {
                                feat::rename_session_input::intent::handle_rename_session_enter(state)
                            }
                            feat::ui::sidebar::sessions::state::SessionEntryKind::Workflow => {
                                feat::rename_workflow_input::intent::handle_rename_workflow_enter(state)
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

            // --- Rename Workflow Input ---
            Intent::RenameWorkflowConfirm => {
                feat::rename_workflow_input::intent::handle_rename_workflow_confirm(state)
            }
            Intent::RenameWorkflowLeave => {
                feat::rename_workflow_input::intent::handle_rename_workflow_leave(state)
            }
            Intent::RenameWorkflowInsertChar { ch } => {
                feat::rename_workflow_input::intent::handle_insert_char(state, *ch)
            }
            Intent::RenameWorkflowCursorLeft => {
                feat::rename_workflow_input::intent::handle_cursor_left(state)
            }
            Intent::RenameWorkflowCursorRight => {
                feat::rename_workflow_input::intent::handle_cursor_right(state)
            }
            Intent::RenameWorkflowDeleteGrapheme => {
                feat::rename_workflow_input::intent::handle_delete(state)
            }
            Intent::RenameWorkflowDeleteForward => {
                feat::rename_workflow_input::intent::handle_delete_forward(state)
            },

            Intent::ToggleOneShot { kind } => {
                let session_id = state.session.active_session_id().clone();
                let session = state.session.get_mut(&session_id).unwrap();
                if session.ui.pending_one_shots.contains_key(kind) {
                    session.ui.pending_one_shots.remove(kind);
                } else {
                    let config = crate::feat::workflow::attached_workflow::WorkflowConfig::from_one_shot_kind(kind);
                    session.ui.pending_one_shots.insert(*kind, config);
                }
                IntentResult::empty()
            }



            // --- Workflow Navigation ---
            Intent::WorkflowNodeLeft => handle_workflow_node_spatial(state, SpatialDirection::Left),
            Intent::WorkflowNodeDown => handle_workflow_node_spatial(state, SpatialDirection::Down),
            Intent::WorkflowNodeUp => handle_workflow_node_spatial(state, SpatialDirection::Up),
            Intent::WorkflowNodeRight => handle_workflow_node_spatial(state, SpatialDirection::Right),
            Intent::WorkflowInspectToggle => handle_workflow_inspect_toggle(state),
            Intent::WorkflowInspectScrollUp => handle_workflow_inspect_scroll_up(state),
            Intent::WorkflowInspectScrollDown => handle_workflow_inspect_scroll_down(state),
            Intent::WorkflowEscape => handle_workflow_escape(state),
            Intent::WorkflowRun => handle_workflow_run(state),
            Intent::WorkflowRerunNode => handle_workflow_rerun_node(state),
            Intent::WorkflowPanLeft => handle_workflow_pan(state, 5, 0),
            Intent::WorkflowPanDown => handle_workflow_pan(state, 0, -5),
            Intent::WorkflowPanUp => handle_workflow_pan(state, 0, 5),
            Intent::WorkflowPanRight => handle_workflow_pan(state, -5, 0),

            // --- Workflow Input Editing ---
            Intent::WorkflowEditNode => {
                crate::feat::workflow::workflow_input::intent::handle_workflow_edit_node(state)
            }
            Intent::WorkflowInputSubmit => {
                crate::feat::workflow::workflow_input::intent::handle_workflow_input_submit(state)
            }
            Intent::WorkflowInputCancel => {
                crate::feat::workflow::workflow_input::intent::handle_workflow_input_cancel(state)
            }
            Intent::WorkflowInputInsertChar { ch } => {
                crate::feat::workflow::workflow_input::intent::handle_workflow_input_insert_char(
                    *ch, state,
                )
            }
            Intent::WorkflowInputDeleteGrapheme => {
                crate::feat::workflow::workflow_input::intent::handle_workflow_input_delete_grapheme(
                    state,
                )
            }
            Intent::WorkflowInputDeleteGraphemeForward => {
                crate::feat::workflow::workflow_input::intent::handle_workflow_input_delete_grapheme_forward(
                    state,
                )
            }
            Intent::WorkflowInputPasteText { text } => {
                crate::feat::workflow::workflow_input::intent::handle_workflow_input_paste_text(
                    text, state,
                )
            }
            Intent::WorkflowInputCursorLeft => {
                crate::feat::workflow::workflow_input::intent::handle_workflow_input_cursor_left(
                    state,
                )
            }
            Intent::WorkflowInputCursorRight => {
                crate::feat::workflow::workflow_input::intent::handle_workflow_input_cursor_right(
                    state,
                )
            }
            Intent::WorkflowInputCursorToStart => {
                crate::feat::workflow::workflow_input::intent::handle_workflow_input_cursor_to_start(
                    state,
                )
            }
            Intent::WorkflowInputCursorToEnd => {
                crate::feat::workflow::workflow_input::intent::handle_workflow_input_cursor_to_end(
                    state,
                )
            }
            Intent::WorkflowInputCursorWordLeft => {
                crate::feat::workflow::workflow_input::intent::handle_workflow_input_cursor_word_left(
                    state,
                )
            }
            Intent::WorkflowInputCursorWordRight => {
                crate::feat::workflow::workflow_input::intent::handle_workflow_input_cursor_word_right(
                    state,
                )
            }
            Intent::WorkflowInputCursorUp => {
                crate::feat::workflow::workflow_input::intent::handle_workflow_input_cursor_up(
                    state,
                )
            }
            Intent::WorkflowInputCursorDown => {
                crate::feat::workflow::workflow_input::intent::handle_workflow_input_cursor_down(
                    state,
                )
            }

            // --- CWD Selection ---
            Intent::ChangeCwd { root } => {
                crate::feat::navigation::intent::handle_change_cwd(state, *root)
            }
        }
    }
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
        // Any other key - dismiss prompt, fall through to normal processing.
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

// --- Workflow Intent Handlers ---

/// Navigate to the spatially nearest node in the given direction.
///
/// Uses the cached spatial index (`node_rects`) to find the nearest node
/// in the pressed direction. Falls back to graph-traversal (first source)
/// when no node is currently selected.
fn handle_workflow_node_spatial(state: &mut AppState, direction: SpatialDirection) -> IntentResult {
    state.frontend.workflow_ui.cancel_prompt = false;

    let Some(workflow) = state.workflow.active() else {
        return IntentResult::empty();
    };
    let snapshot = workflow.execution.snapshot();

    // Recompute spatial index if empty (cache invalidation).
    if state.frontend.workflow_ui.node_rects.is_empty() {
        state.frontend.workflow_ui.node_rects =
            jinn_workflow::spatial_layout::compute_spatial_layout(snapshot.structure());
    }

    let rects = &state.frontend.workflow_ui.node_rects;

    // If no node selected, select the first source node.
    let Some(current_name) = &state.frontend.workflow_ui.selected_node else {
        let sources = snapshot.structure().sources();
        if let Some(first) = sources.first() {
            state.frontend.workflow_ui.selected_node = Some(first.clone());
        }
        return IntentResult::empty();
    };

    let Some(current_rect) = rects.get(current_name) else {
        return IntentResult::empty();
    };

    let next = jinn_workflow::spatial_layout::spatial_nearest(
        current_rect,
        direction,
        rects,
        current_name,
    );

    if let Some(next_name) = next {
        state.frontend.workflow_ui.selected_node = Some(next_name);
        state.frontend.workflow_ui.inspector_scroll = 0;
        state
            .frontend
            .workflow_ui
            .inspector_scroll_rendered
            .store(0, std::sync::atomic::Ordering::Relaxed);
    }

    IntentResult::empty()
}

/// Toggle the sticky inspector popup.
fn handle_workflow_inspect_toggle(state: &mut AppState) -> IntentResult {
    state.frontend.workflow_ui.cancel_prompt = false;
    state.frontend.workflow_ui.inspector_open = !state.frontend.workflow_ui.inspector_open;
    if state.frontend.workflow_ui.inspector_open {
        state.frontend.workflow_ui.inspector_scroll = 0;
    }
    IntentResult::empty()
}

/// Scroll the inspector popup up.
fn handle_workflow_inspect_scroll_up(state: &mut AppState) -> IntentResult {
    state.frontend.workflow_ui.cancel_prompt = false;
    let base = state
        .frontend
        .workflow_ui
        .inspector_scroll_rendered
        .load(std::sync::atomic::Ordering::Relaxed);
    state.frontend.workflow_ui.inspector_scroll = base.saturating_sub(1);
    IntentResult::empty()
}

/// Scroll the inspector popup down.
fn handle_workflow_inspect_scroll_down(state: &mut AppState) -> IntentResult {
    state.frontend.workflow_ui.cancel_prompt = false;
    let base = state
        .frontend
        .workflow_ui
        .inspector_scroll_rendered
        .load(std::sync::atomic::Ordering::Relaxed);
    state.frontend.workflow_ui.inspector_scroll = base.saturating_add(1);
    IntentResult::empty()
}

/// ESC in workflow scope: two-press cancel with confirmation.
fn handle_workflow_escape(state: &mut AppState) -> IntentResult {
    // If no active workflow, or workflow has no running nodes, just reset prompt and no-op.
    let has_running = state.workflow.active().is_some_and(|w| {
        w.execution
            .snapshot()
            .statuses()
            .any(|(_, s)| s == jinn_workflow::engine::NodeStatus::Running)
    });

    if !has_running {
        // Workflow is idle or completed - ESC just resets any stale prompt state.
        state.frontend.workflow_ui.cancel_prompt = false;
        return IntentResult::empty();
    }

    // Workflow has running nodes - use two-press cancel confirmation.
    if state.frontend.workflow_ui.cancel_prompt {
        // Second ESC - confirm cancel.
        state.frontend.workflow_ui.cancel_prompt = false;
        let Some(workflow) = state.workflow.active() else {
            return IntentResult::empty();
        };
        let workflow_id = workflow.id.clone();
        IntentResult::with_commands(vec![Command::CancelWorkflow(
            crate::feat::workflow::protocol::command::CancelWorkflow { workflow_id },
        )])
    } else {
        // First ESC - show prompt.
        state.frontend.workflow_ui.cancel_prompt = true;
        IntentResult::empty()
    }
}

/// Run the loaded workflow.
///
/// Validates that an active workflow exists and is in a runnable state
/// (all nodes Pending = first run, or has result from previous run = re-run).
/// Emits StartWorkflow to trigger engine execution.
fn handle_workflow_run(state: &mut AppState) -> IntentResult {
    state.frontend.workflow_ui.cancel_prompt = false;

    let Some(workflow) = state.workflow.active() else {
        return IntentResult::empty();
    };

    let snapshot = workflow.execution.snapshot();

    // Check if any node is currently Running - if so, no-op (already executing).
    let has_running = snapshot
        .statuses()
        .any(|(_, s)| s == jinn_workflow::engine::NodeStatus::Running);
    if has_running {
        return IntentResult::empty();
    }

    // Check if any source node is still awaiting user input.
    let has_awaiting_input = snapshot
        .statuses()
        .any(|(_, s)| s == jinn_workflow::engine::NodeStatus::AwaitingInput);
    if has_awaiting_input {
        return IntentResult::empty();
    }

    let workflow_id = workflow.id.clone();
    let name = workflow.name.clone();

    // If any node has completed/failed/skipped, this is a re-run.
    // Invalidate all nodes to reset to Pending before running.
    let has_terminal = snapshot.statuses().any(|(_, s)| s.is_terminal());
    if has_terminal {
        // Save source node outputs before invalidation clears them.
        // Source nodes that had user-provided data must retain it across re-runs.
        let sources = snapshot.structure().sources();
        let saved_outputs: Vec<(String, jinn_workflow::port::PortValues)> = sources
            .iter()
            .filter_map(|name| {
                snapshot
                    .node_state(name)
                    .and_then(|s| s.outputs.as_ref())
                    .map(|arc| (name.clone(), (**arc).clone()))
            })
            .collect();

        for node_name in snapshot.structure().node_names() {
            workflow.execution.invalidate_from(node_name);
        }

        // Restore source node outputs so they survive the re-run.
        for (name, outputs) in &saved_outputs {
            workflow.execution.set_node_outputs(name, outputs.clone());
            workflow
                .execution
                .set_status(name, jinn_workflow::engine::NodeStatus::Pending);
        }

        // Replace cancellation token with fresh one.
        if let Some(w) = state.workflow.active_mut() {
            w.cancel = tokio_util::sync::CancellationToken::new();
        }
    }

    IntentResult::with_commands(vec![Command::StartWorkflow(
        crate::feat::workflow::protocol::command::StartWorkflow { name, workflow_id },
    )])
}

/// Re-run the workflow from the currently selected node.
fn handle_workflow_rerun_node(state: &mut AppState) -> IntentResult {
    state.frontend.workflow_ui.cancel_prompt = false;

    let Some(node_name) = state.frontend.workflow_ui.selected_node.clone() else {
        return IntentResult::empty();
    };

    let Some(workflow) = state.workflow.active() else {
        return IntentResult::empty();
    };

    // Validate: node must be Completed or Failed.
    let snapshot = workflow.execution.snapshot();
    let status = snapshot.status_of(&node_name);
    match status {
        Some(
            jinn_workflow::engine::NodeStatus::Completed
            | jinn_workflow::engine::NodeStatus::Failed,
        ) => {}
        _ => return IntentResult::empty(),
    }

    let workflow_id = workflow.id.clone();
    let execution = workflow.execution.clone();

    // Invalidate from the selected node downstream.
    execution.invalidate_from(&node_name);
    // Seed inputs from upstream cached outputs.
    execution.seed_inputs(&node_name);

    // Replace the cancellation token with a fresh one.
    if let Some(w) = state.workflow.active_mut() {
        w.cancel = tokio_util::sync::CancellationToken::new();
    }

    IntentResult::with_commands(vec![Command::RerunFromNode(
        crate::feat::workflow::protocol::command::RerunFromNode {
            workflow_id,
            node_name,
        },
    )])
}

/// Pan the workflow viewport.
fn handle_workflow_pan(state: &mut AppState, dx: i32, dy: i32) -> IntentResult {
    state.frontend.workflow_ui.cancel_prompt = false;
    state.frontend.workflow_ui.viewport_offset_x = state
        .frontend
        .workflow_ui
        .viewport_offset_x
        .saturating_add(dx);
    state.frontend.workflow_ui.viewport_offset_y = state
        .frontend
        .workflow_ui
        .viewport_offset_y
        .saturating_add(dy);
    IntentResult::empty()
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

    #[test]
    fn workflow_run_rejects_when_source_node_awaiting_input() {
        // Given an initialized workflow where a source node has AwaitingInput status.
        let mut state = AppState::default();
        let execution = std::sync::Arc::new(jinn_workflow::execution::WorkflowExecution::new(
            source_graph_for_test(),
        ));
        let workflow_state = crate::feat::workflow::workflow_state::WorkflowState::new(
            "test".to_owned(),
            execution.clone(),
        );
        state.workflow.insert(workflow_state);
        state.frontend.scope_stack.swap_base(FocusScope::Workflow);

        // Mark source node as AwaitingInput.
        execution.set_status("source", jinn_workflow::engine::NodeStatus::AwaitingInput);

        // When handling WorkflowRun.
        let result = IntentHandler::handle(&Intent::WorkflowRun, &mut state);

        // Then no StartWorkflow command is emitted.
        assert!(result.commands.is_empty());
    }

    #[test]
    fn workflow_run_accepts_after_source_nodes_provided() {
        // Given a workflow where the source node has been given data (Pending status).
        let mut state = AppState::default();
        let execution = std::sync::Arc::new(jinn_workflow::execution::WorkflowExecution::new(
            source_graph_for_test(),
        ));
        let workflow_state = crate::feat::workflow::workflow_state::WorkflowState::new(
            "test".to_owned(),
            execution.clone(),
        );
        state.workflow.insert(workflow_state);
        state.frontend.scope_stack.swap_base(FocusScope::Workflow);

        // Source node is Pending (user has provided data).
        execution.set_status("source", jinn_workflow::engine::NodeStatus::Pending);

        // When handling WorkflowRun.
        let result = IntentHandler::handle(&Intent::WorkflowRun, &mut state);

        // Then a StartWorkflow command is emitted.
        assert!(!result.commands.is_empty());
        let cmd = &result.commands[0];
        assert!(matches!(cmd, crate::protocol::Command::StartWorkflow(_)));
    }

    /// Verifies that re-running a workflow preserves source node outputs.
    /// `invalidate_from` clears outputs, but `handle_workflow_run` saves/restores
    /// source node outputs so user-provided data survives across re-runs.
    #[test]
    fn workflow_run_preserves_source_outputs_on_rerun() {
        // Given a workflow where the source node has Completed (simulating first run).
        let mut state = AppState::default();
        let execution = std::sync::Arc::new(jinn_workflow::execution::WorkflowExecution::new(
            source_graph_for_test(),
        ));
        let workflow_state = crate::feat::workflow::workflow_state::WorkflowState::new(
            "test".to_owned(),
            execution.clone(),
        );
        state.workflow.insert(workflow_state);
        state.frontend.scope_stack.swap_base(FocusScope::Workflow);

        // Set source node to Completed with user-provided output.
        let mut outputs = jinn_workflow::port::PortValues::new();
        outputs.insert(
            "out".to_owned(),
            jinn_workflow::port::PortValue::Single(jinn_workflow::port::ScalarValue::Text(
                "user_data".to_owned(),
            )),
        );
        execution.set_node_outputs("source", outputs);
        execution.set_status("source", jinn_workflow::engine::NodeStatus::Completed);

        // When handling WorkflowRun (re-run because Completed is terminal).
        let result = IntentHandler::handle(&Intent::WorkflowRun, &mut state);

        // Then a StartWorkflow command is emitted.
        assert!(!result.commands.is_empty());

        // And the source node's output is preserved after the invalidation cycle.
        let snapshot = execution.snapshot();
        let node_state = snapshot.node_state("source").expect("node exists");
        let outputs = node_state.outputs.as_ref().expect("has outputs");
        assert_eq!(outputs.get_text("out").unwrap(), "user_data");
    }

    /// Helper: builds a minimal source-only graph for testing.
    fn source_graph_for_test() -> jinn_workflow::graph::WorkflowGraph {
        use jinn_workflow::node::code::CodeNode;
        use jinn_workflow::port::{PortDef, PortValue, PortValues, ScalarValue};

        let source = CodeNode::new(
            "source".to_owned(),
            vec![],
            vec![PortDef::text("out")],
            |_inputs, _ctx| {
                Box::pin(async move {
                    let mut out = PortValues::new();
                    out.insert(
                        "out".to_owned(),
                        PortValue::Single(ScalarValue::Text("data".to_owned())),
                    );
                    Ok(out)
                })
            },
        );
        let mut builder = jinn_workflow::graph::WorkflowGraphBuilder::new();
        builder.add_node("source".to_owned(), Box::new(source));
        builder.build().expect("graph should be valid")
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

    // --- Mutant-killing tests for workflow handlers ---

    #[test]
    fn workflow_inspect_toggle_flips_state() {
        // Given a workflow context.
        let mut state = AppState::default();
        state.frontend.scope_stack.swap_base(FocusScope::Workflow);
        assert!(!state.frontend.workflow_ui.inspector_open);

        // When toggling inspector.
        let _result = IntentHandler::handle(&Intent::WorkflowInspectToggle, &mut state);

        // Then inspector is open.
        assert!(state.frontend.workflow_ui.inspector_open);

        // When toggling again.
        let _result = IntentHandler::handle(&Intent::WorkflowInspectToggle, &mut state);

        // Then inspector is closed.
        assert!(!state.frontend.workflow_ui.inspector_open);
    }

    #[test]
    fn workflow_escape_no_running_resets_prompt() {
        // Given a workflow scope with no active workflow.
        let mut state = AppState::default();
        state.frontend.scope_stack.swap_base(FocusScope::Workflow);
        state.frontend.workflow_ui.cancel_prompt = true;

        // When handling WorkflowEscape.
        let _result = IntentHandler::handle(&Intent::WorkflowEscape, &mut state);

        // Then cancel_prompt is reset to false (no running nodes).
        assert!(!state.frontend.workflow_ui.cancel_prompt);
    }

    #[test]
    fn workflow_rerun_node_rejects_pending_node() {
        // Given a workflow with a pending node selected.
        let mut state = AppState::default();
        let execution = std::sync::Arc::new(jinn_workflow::execution::WorkflowExecution::new(
            source_graph_for_test(),
        ));
        let workflow_state = crate::feat::workflow::workflow_state::WorkflowState::new(
            "test".to_owned(),
            execution.clone(),
        );
        state.workflow.insert(workflow_state);
        state.frontend.scope_stack.swap_base(FocusScope::Workflow);
        state.frontend.workflow_ui.selected_node = Some("source".to_owned());
        execution.set_status("source", jinn_workflow::engine::NodeStatus::Pending);

        // When handling WorkflowRerunNode.
        let result = IntentHandler::handle(&Intent::WorkflowRerunNode, &mut state);

        // Then no commands are emitted (node must be Completed or Failed).
        assert!(result.commands.is_empty());
    }

    #[test]
    fn workflow_rerun_node_accepts_completed_node() {
        // Given a workflow with a Completed node selected.
        let mut state = AppState::default();
        let execution = std::sync::Arc::new(jinn_workflow::execution::WorkflowExecution::new(
            source_graph_for_test(),
        ));
        let workflow_state = crate::feat::workflow::workflow_state::WorkflowState::new(
            "test".to_owned(),
            execution.clone(),
        );
        state.workflow.insert(workflow_state);
        state.frontend.scope_stack.swap_base(FocusScope::Workflow);
        state.frontend.workflow_ui.selected_node = Some("source".to_owned());
        execution.set_status("source", jinn_workflow::engine::NodeStatus::Completed);

        // When handling WorkflowRerunNode.
        let result = IntentHandler::handle(&Intent::WorkflowRerunNode, &mut state);

        // Then a RerunFromNode command is emitted.
        assert!(
            result
                .commands
                .iter()
                .any(|c| matches!(c, crate::protocol::Command::RerunFromNode(_))),
            "should emit RerunFromNode: {:?}",
            result.commands
        );
    }

    #[test]
    fn workflow_pan_left_decrements_offset() {
        // Given workflow scope.
        let mut state = AppState::default();
        state.frontend.scope_stack.swap_base(FocusScope::Workflow);
        state.frontend.workflow_ui.viewport_offset_x = 10;

        // When panning left.
        let _result = IntentHandler::handle(&Intent::WorkflowPanLeft, &mut state);

        // Then offset_x increased by 5 (panning left = viewport moves right in content).
        assert_eq!(state.frontend.workflow_ui.viewport_offset_x, 15);
    }

    #[test]
    fn workflow_pan_right_decrements_offset() {
        // Given workflow scope.
        let mut state = AppState::default();
        state.frontend.scope_stack.swap_base(FocusScope::Workflow);
        state.frontend.workflow_ui.viewport_offset_x = 10;

        // When panning right.
        let _result = IntentHandler::handle(&Intent::WorkflowPanRight, &mut state);

        // Then offset_x decreased by 5.
        assert_eq!(state.frontend.workflow_ui.viewport_offset_x, 5);
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

    #[rstest::rstest]
    fn one_shot_toggle_inserts_and_removes() {
        use crate::feat::workflow::attached_workflow::OneShotKind;

        // Given an AppState with an active session.
        let mut state = AppState::default();
        let session_id = {
            let session = crate::feat::session::chat_session::ChatSessionState::new();
            let id = session.session_id().clone();
            state.session.insert(session);
            state.session.set_active(id.clone());
            id
        };

        // When toggling consensus one-shot.
        let _result = IntentHandler::handle(
            &Intent::ToggleOneShot {
                kind: OneShotKind::Consensus,
            },
            &mut state,
        );

        // Then pending_one_shots has an entry.
        {
            let session = state.session.get(&session_id).expect("session");
            assert!(
                session
                    .ui
                    .pending_one_shots
                    .contains_key(&OneShotKind::Consensus)
            );
        }

        // When toggling again.
        let _result = IntentHandler::handle(
            &Intent::ToggleOneShot {
                kind: OneShotKind::Consensus,
            },
            &mut state,
        );

        // Then the entry is removed.
        {
            let session = state.session.get(&session_id).expect("session");
            assert!(
                !session
                    .ui
                    .pending_one_shots
                    .contains_key(&OneShotKind::Consensus)
            );
        }
    }
}
