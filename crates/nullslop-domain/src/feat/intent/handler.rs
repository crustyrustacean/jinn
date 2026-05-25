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
use crate::feat::session::chat_session::SessionPhase;
use crate::protocol::{Command, PickerKind, PinPosition, SessionId};

use crate::Intent;
use crate::feat;

use crate::IntentResult;
use nullslop_workflow::spatial_layout::SpatialDirection;

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
                // ESC cancels arg input — pop scope, clear state.
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
                // First press — show confirmation prompt.
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

            // --- Rename Session Input ---
            Intent::SidebarRenameSession => {
                feat::rename_session_input::intent::handle_rename_session_enter(state)
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
            Intent::SwitchTab => {
                let next_tab = state.frontend.active_tab.next();
                let new_scope = match next_tab {
                    crate::protocol::tab::ActiveTab::Chat => {
                        crate::common::app_state::FocusScope::Normal
                    }
                    crate::protocol::tab::ActiveTab::Workflow => {
                        crate::common::app_state::FocusScope::Workflow
                    }
                };
                state.frontend.active_tab = next_tab;
                state.frontend.scope_stack.swap_base(new_scope);
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
        // Any other key — dismiss prompt, fall through to normal processing.
        return None;
    }

    let session_id = state.session.active_session_id().clone();

    if state.active_session().phase() == SessionPhase::Compacting {
        return Some(handle_compaction_cancel(state, session_id));
    }

    // Existing stream cancel behavior (Streaming, Sending, Assembling, Idle).
    state.active_session_mut().cancel_stream_and_drain();
    Some(IntentResult::with_commands(vec![Command::CancelStream(
        crate::feat::provider::protocol::command::CancelStream { session_id },
    )]))
}

/// Close session confirmation prompt intercept.
///
/// If the close-session confirmation prompt is showing:
/// - `SidebarSessionClose` confirms the close (re-validates, emits CloseSession).
/// - Any other intent dismisses the prompt and returns `None` (fall through to normal processing).
///
/// Returns `None` if the prompt is not showing or was dismissed.
fn try_handle_close_session_prompt(
    intent: &Intent,
    state: &mut AppState,
) -> Option<IntentResult> {
    if !state.frontend.close_session_prompt {
        return None;
    }

    // Dismiss the prompt regardless of which intent triggered it.
    state.frontend.close_session_prompt = false;

    if !matches!(intent, Intent::SidebarSessionClose) {
        // Any other key — dismiss prompt, fall through to normal processing.
        return None;
    }

    // Second x press — perform the close.
    // Re-validates in case session became busy between taps.
    Some(feat::ui::sidebar::sessions::handle_session_close_with_lifecycle(
        state,
    ))
}

/// Handle cancelling an in-progress context compaction.
///
/// ESC = universal abort. Cancels compaction, aborts the compaction LLM task,
/// un-ignores entries, and drains all queue items back to the input buffer.
/// No synthetic continue is enqueued.
fn handle_compaction_cancel(state: &mut AppState, session_id: SessionId) -> IntentResult {
    let _drained = state.active_session_mut().cancel_compacting();
    // cancel_compacting already drains the queue into VecDeque<QueueItem>,
    // but we need to put UserMessage display text into the input buffer.
    // Use cancel_stream_and_drain which handles typed queue items.
    // Actually, cancel_compacting already drained the queue. Let's handle
    // the drained items manually to put text into input buffer.
    // For now, the queue is already drained by cancel_compacting. The
    // drained items are discarded (ESC = stop all the things).

    IntentResult::with_commands(vec![
        Command::PushChatEntry(crate::feat::chat_input::protocol::command::PushChatEntry {
            session_id: session_id.clone(),
            entry: crate::ChatEntry::system("Context compaction cancelled."),
        }),
        Command::CancelCompaction(
            crate::feat::compaction_actor::protocol::command::CancelCompaction { session_id },
        ),
    ])
}

// --- Workflow Intent Handlers ---

/// Navigate to the spatially nearest node in the given direction.
///
/// Uses the cached spatial index (`node_rects`) to find the nearest node
/// in the pressed direction. Falls back to graph-traversal (first source)
/// when no node is currently selected.
fn handle_workflow_node_spatial(
    state: &mut AppState,
    direction: SpatialDirection,
) -> IntentResult {
    state.frontend.workflow_ui.cancel_prompt = false;

    let Some(workflow) = state.workflow.active() else {
        return IntentResult::empty();
    };
    let snapshot = workflow.execution.snapshot();

    // Recompute spatial index if empty (cache invalidation).
    if state.frontend.workflow_ui.node_rects.is_empty() {
        state.frontend.workflow_ui.node_rects =
            nullslop_workflow::spatial_layout::compute_spatial_layout(snapshot.structure());
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

    let next = nullslop_workflow::spatial_layout::spatial_nearest(
        current_rect,
        direction,
        rects,
        current_name,
    );

    if let Some(next_name) = next {
        state.frontend.workflow_ui.selected_node = Some(next_name);
        state.frontend.workflow_ui.inspector_scroll = 0;
        state.frontend.workflow_ui.inspector_scroll_rendered.store(
            0,
            std::sync::atomic::Ordering::Relaxed,
        );
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
        w.execution.snapshot().statuses().any(|(_, s)| {
            s == nullslop_workflow::engine::NodeStatus::Running
        })
    });

    if !has_running {
        // Workflow is idle or completed — ESC just resets any stale prompt state.
        state.frontend.workflow_ui.cancel_prompt = false;
        return IntentResult::empty();
    }

    // Workflow has running nodes — use two-press cancel confirmation.
    if state.frontend.workflow_ui.cancel_prompt {
        // Second ESC — confirm cancel.
        state.frontend.workflow_ui.cancel_prompt = false;
        let Some(workflow) = state.workflow.active() else {
            return IntentResult::empty();
        };
        let workflow_id = workflow.id.clone();
        IntentResult::with_commands(vec![Command::CancelWorkflow(
            crate::feat::workflow::protocol::command::CancelWorkflow { workflow_id },
        )])
    } else {
        // First ESC — show prompt.
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

    // Check if any node is currently Running — if so, no-op (already executing).
    let has_running = snapshot.statuses().any(|(_, s)| {
        s == nullslop_workflow::engine::NodeStatus::Running
    });
    if has_running {
        return IntentResult::empty();
    }

    // Check if any source node is still awaiting user input.
    let has_awaiting_input = snapshot.statuses().any(|(_, s)| {
        s == nullslop_workflow::engine::NodeStatus::AwaitingInput
    });
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
        let saved_outputs: Vec<(String, nullslop_workflow::port::PortValues)> = sources
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
                .set_status(name, nullslop_workflow::engine::NodeStatus::Pending);
        }

        // Replace cancellation token with fresh one.
        if let Some(w) = state.workflow.active_mut() {
            w.cancel = tokio_util::sync::CancellationToken::new();
        }
    }

    IntentResult::with_commands(vec![Command::StartWorkflow(
        crate::feat::workflow::protocol::command::StartWorkflow {
            name,
            workflow_id,
        },
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
            nullslop_workflow::engine::NodeStatus::Completed
            | nullslop_workflow::engine::NodeStatus::Failed,
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
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use crate::common::app_state::{AppState, FocusScope, RenameSessionInputState};
    use crate::feat::intent::IntentHandler;
    use crate::protocol::Intent;

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
        let execution = std::sync::Arc::new(
            nullslop_workflow::execution::WorkflowExecution::new(source_graph_for_test()),
        );
        let workflow_state = crate::feat::workflow::workflow_state::WorkflowState::new(
            "test".to_owned(),
            execution.clone(),
        );
        state.workflow.insert(workflow_state);
        state.frontend.scope_stack.swap_base(FocusScope::Workflow);

        // Mark source node as AwaitingInput.
        execution.set_status("source", nullslop_workflow::engine::NodeStatus::AwaitingInput);

        // When handling WorkflowRun.
        let result = IntentHandler::handle(&Intent::WorkflowRun, &mut state);

        // Then no StartWorkflow command is emitted.
        assert!(result.commands.is_empty());
    }

    #[test]
    fn workflow_run_accepts_after_source_nodes_provided() {
        // Given a workflow where the source node has been given data (Pending status).
        let mut state = AppState::default();
        let execution = std::sync::Arc::new(
            nullslop_workflow::execution::WorkflowExecution::new(source_graph_for_test()),
        );
        let workflow_state = crate::feat::workflow::workflow_state::WorkflowState::new(
            "test".to_owned(),
            execution.clone(),
        );
        state.workflow.insert(workflow_state);
        state.frontend.scope_stack.swap_base(FocusScope::Workflow);

        // Source node is Pending (user has provided data).
        execution.set_status("source", nullslop_workflow::engine::NodeStatus::Pending);

        // When handling WorkflowRun.
        let result = IntentHandler::handle(&Intent::WorkflowRun, &mut state);

        // Then a StartWorkflow command is emitted.
        assert!(!result.commands.is_empty());
        let cmd = &result.commands[0];
        assert!(matches!(
            cmd,
            crate::protocol::Command::StartWorkflow(_)
        ));
    }

    /// Verifies that re-running a workflow preserves source node outputs.
    /// `invalidate_from` clears outputs, but `handle_workflow_run` saves/restores
    /// source node outputs so user-provided data survives across re-runs.
    #[test]
    fn workflow_run_preserves_source_outputs_on_rerun() {
        // Given a workflow where the source node has Completed (simulating first run).
        let mut state = AppState::default();
        let execution = std::sync::Arc::new(
            nullslop_workflow::execution::WorkflowExecution::new(source_graph_for_test()),
        );
        let workflow_state = crate::feat::workflow::workflow_state::WorkflowState::new(
            "test".to_owned(),
            execution.clone(),
        );
        state.workflow.insert(workflow_state);
        state.frontend.scope_stack.swap_base(FocusScope::Workflow);

        // Set source node to Completed with user-provided output.
        let mut outputs = nullslop_workflow::port::PortValues::new();
        outputs.insert(
            "out".to_owned(),
            nullslop_workflow::port::PortValue::Single(
                nullslop_workflow::port::ScalarValue::Text("user_data".to_owned()),
            ),
        );
        execution.set_node_outputs("source", outputs);
        execution.set_status("source", nullslop_workflow::engine::NodeStatus::Completed);

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
    fn source_graph_for_test() -> nullslop_workflow::graph::WorkflowGraph {
        use nullslop_workflow::node::code::CodeNode;
        use nullslop_workflow::port::{PortDef, PortValue, PortValues, ScalarValue};

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
        let mut builder = nullslop_workflow::graph::WorkflowGraphBuilder::new();
        builder.add_node("source".to_owned(), Box::new(source));
        builder.build().expect("graph should be valid")
    }
}
