//! Picker intent handlers - navigation, filtering, confirmation, and scope toggling.
//!
//! Handles all picker intents: open, insert char, backspace, confirm, move up/down,
//! and cursor movement. Confirmation is owned entirely by the active spec's
//! confirm hook — there is no intent re-dispatch, so no handler here returns a
//! follow-up intent.

use crate::common::app_state::AppState;
use jinn_core_types::model_selection::ModelSelection;
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

    // Endpoint picker is only reachable for a Single (non-alloy) model. This
    // gate must run BEFORE the scope push, so it cannot live in the spec's
    // open hook (hooks run after the push). The backend gate (OpenRouter vs
    // direct) runs later in the discovery actor, which owns `Services`; here
    // we only reject the model-shape mismatch.
    if matches!(kind, PickerKind::Endpoint)
        && matches!(
            state.active_session().profile().model,
            ModelSelection::Alloy { .. }
        )
    {
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
    if let Some(picker) = state.active_picker_ops() {
        picker.insert_char(ch);
    }
    IntentResult::empty()
}

/// Handles `PasteText` in picker scope - bulk inserts pasted text into the filter.
///
/// Newlines are stripped by the picker's `insert_text` method since the filter
/// is a single-line input.
pub fn handle_picker_paste(state: &mut AppState, text: &str) -> IntentResult {
    if let Some(picker) = state.active_picker_ops() {
        picker.insert_text(text);
    }
    IntentResult::empty()
}

/// Removes the last character from the active picker's filter.
pub fn handle_backspace(state: &mut AppState) -> IntentResult {
    validator::validate_picker_backspace(state);
    if let Some(picker) = state.active_picker_ops() {
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
    if let Some(picker) = state.active_picker_ops() {
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
    if let Some(picker) = state.active_picker_ops() {
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
    if let Some(picker) = state.active_picker_ops() {
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
    if let Some(picker) = state.active_picker_ops() {
        picker.page_down(viewport);
    }
    reset_preview_scroll(state, pickers);
    crate::feat::picker::action::run_selection_change(state, pickers);
    IntentResult::empty()
}

/// Moves the filter cursor left in the active picker.
pub fn handle_move_cursor_left(state: &mut AppState) -> IntentResult {
    validator::validate_picker_move_cursor_left(state);
    if let Some(picker) = state.active_picker_ops() {
        picker.move_cursor_left();
    }
    IntentResult::empty()
}

/// Moves the filter cursor right in the active picker.
pub fn handle_move_cursor_right(state: &mut AppState) -> IntentResult {
    validator::validate_picker_move_cursor_right(state);
    if let Some(picker) = state.active_picker_ops() {
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
    use super::*;
    use crate::feat::ui::picker_states::PickerExt;

    fn empty_pickers() -> jinn_picker::PickerRegistry {
        jinn_picker::PickerRegistry::new()
    }
    use jinn_session_state::ChatSessionState;
    fn setup_state_with_task_list() -> (AppState, jinn_tools_msg::TaskId) {
        use jinn_tools_msg::{PhaseInput, TaskStatus};

        let mut state = AppState::default_with_scope_focus();
        let mut origin = ChatSessionState::new();

        origin.task_list_mut().set_from_inputs(&[
            // Phase 1 with 2 tasks (one Pending, one Completed).
            PhaseInput {
                description: "Research".to_owned(),
                tasks: vec![
                    ("Read codebase".to_owned(), TaskStatus::Pending),
                    ("Write notes".to_owned(), TaskStatus::Completed),
                ],
            },
            // Phase 2 with a Pending task, a Cancelled task, and a Postponed
            // source whose ID is surfaced to tests (a Pending copy with the
            // same description sits beside it).
            PhaseInput {
                description: "Build".to_owned(),
                tasks: vec![
                    ("Implement feature".to_owned(), TaskStatus::Pending),
                    ("Investigate alt".to_owned(), TaskStatus::Cancelled),
                    ("Refactor later".to_owned(), TaskStatus::Postponed),
                    ("Refactor later".to_owned(), TaskStatus::Pending),
                ],
            },
        ]);

        let postponed_id = {
            let list = origin.task_list();
            let build = &list.phases()[1];
            build
                .tasks
                .iter()
                .find(|t| t.status == TaskStatus::Postponed)
                .map(|t| t.id.clone())
                .expect("postponed source present")
        };

        let origin_id = origin.session_id().clone();
        state.session.insert(origin);
        assert!(
            state.session.set_active(origin_id),
            "origin session must be present for set_active"
        );
        (state, postponed_id)
    }

    #[rstest::rstest]
    fn handle_picker_confirm_task_list_is_noop_and_keeps_scope() {
        // Given state with the TaskList picker scope on the stack.
        let (mut state, _postponed_id) = setup_state_with_task_list();
        state.frontend.scope_push(FocusScope::Picker {
            kind: PickerKind::TaskList,
        });
        let len_before = state.frontend.scope_len();

        // When confirming.
        let result = handle_picker_confirm(&mut state, &empty_pickers());

        // Then no commands are emitted and the scope stack is unchanged.
        assert!(result.message_names.is_empty(), "no commands");
        assert_eq!(
            state.frontend.scope_len(),
            len_before,
            "scope stack must remain unchanged on no-op confirm"
        );
        assert!(matches!(
            state.frontend.scope(),
            FocusScope::Picker {
                kind: PickerKind::TaskList
            }
        ));
    }

    #[rstest::rstest]
    fn esc_from_task_list_picker_restores_sidebar_task_list_scope() {
        // Given a scope stack like: [Normal, sidebar task-list, Picker(TaskList)].
        let mut state = AppState::default_with_scope_focus();
        state
            .frontend
            .scope_push(jinn_sidebar_msg::SidebarSectionId::TaskList.focus_scope());
        state.frontend.scope_push(FocusScope::Picker {
            kind: PickerKind::TaskList,
        });

        // When Esc is pressed.
        let _ = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

        // Then we should return to the task-list section, not Normal.
        assert_eq!(
            state.frontend.sidebar_section(),
            Some(jinn_sidebar_msg::SidebarSectionId::TaskList),
            "Esc from TaskList picker should restore the task-list section, got: {:?}",
            state.frontend.scope()
        );
    }

    /// Builds a state with the provider picker open (Provider scope), `n` available
    /// single-model entries `model-0..model-n`, and the first entry highlighted.
    fn state_with_provider_picker(n: usize) -> AppState {
        let mut state = AppState::default_with_scope_focus();
        let origin = ChatSessionState::new();
        state.session.insert(origin);
        state
            .session
            .set_active(state.session.active_session_id().clone());
        state.frontend.scope_push(FocusScope::Picker {
            kind: PickerKind::Provider,
        });
        let entries: Vec<crate::protocol::ProviderPickerEntry> = (0..n)
            .map(|i| crate::protocol::ProviderPickerEntry {
                provider_id: format!("prov/model-{i}"),
                name: "prov".to_owned(),
                provider_name: "prov".to_owned(),
                backend: "openrouter".to_owned(),
                model: format!("model-{i}"),
                search_text: format!("model-{i}"),
                is_alias: false,
                alias_target: None,
                is_available: true,
                is_remote: false,
                is_active: false,
                selected: false,
                theme: jinn_theme::default_theme(),
            })
            .collect();
        state
            .frontend
            .pickers
            .provider_picker
            .set_items(jinn_picker::make_items_with_hooks(
                entries,
                jinn_picker::PickerItemHooks::new().search(
                    |entry: &crate::protocol::ProviderPickerEntry| {
                        format!("{} {}", entry.model, entry.provider_name)
                    },
                ),
            ));
        state.frontend.pickers.provider_picker.move_down(1); // highlight first entry
        state
    }

    #[rstest::rstest]
    fn handle_move_down_uses_measured_viewport() {
        // Given a provider picker with 20 entries and a measured viewport of 5,
        // selection already on the last visible row (index 4).
        let mut state = state_with_provider_picker(20);
        state.frontend.set_picker_results_viewport(5);
        state.frontend.pickers.provider_picker.move_up(5); // back to selection 0
        for _ in 0..4 {
            state.frontend.pickers.provider_picker.move_down(5);
        }
        assert_eq!(state.frontend.pickers.provider_picker.selection(), 4);
        assert_eq!(state.frontend.pickers.provider_picker.scroll_offset(), 0);

        // When moving down once more.
        handle_move_down(&mut state, &empty_pickers());

        // Then selection advances to 5 and scroll_offset advances by one
        // (measured viewport of 5, not the old hardcoded 100).
        assert_eq!(state.frontend.pickers.provider_picker.selection(), 5);
        assert_eq!(state.frontend.pickers.provider_picker.scroll_offset(), 1);
    }

    #[rstest::rstest]
    fn handle_page_down_advances_selection_by_half_viewport() {
        // Given a provider picker with 20 entries, selection at 0, viewport 10.
        let mut state = state_with_provider_picker(20);
        state.frontend.set_picker_results_viewport(10);
        state.frontend.pickers.provider_picker.move_up(5); // selection back to 0

        // When handling PickerPageDown (half of 10 = 5).
        handle_page_down(&mut state, &empty_pickers());

        // Then selection advances by 5.
        assert_eq!(state.frontend.pickers.provider_picker.selection(), 5);
    }

    #[rstest::rstest]
    fn handle_page_up_decrements_selection_by_half_viewport() {
        // Given a provider picker with 20 entries, selection at 10, viewport 10.
        let mut state = state_with_provider_picker(20);
        state.frontend.set_picker_results_viewport(10);
        // Advance selection to 10.
        for _ in 0..9 {
            state.frontend.pickers.provider_picker.move_down(10);
        }

        // When handling PickerPageUp (half of 10 = 5).
        handle_page_up(&mut state, &empty_pickers());

        // Then selection decrements by 5.
        assert_eq!(state.frontend.pickers.provider_picker.selection(), 5);
    }
}
