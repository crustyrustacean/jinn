//! Navigation within the sessions sidebar section.

use crate::common::app_state::AppState;
use crate::feat::ui::sidebar::section_trait::{EnterFrom, SectionNavResult, SidebarIntent};
use crate::feat::ui::sidebar::sessions::state::{SessionEntryKind, sorted_open_sessions};

use super::MAX_VISIBLE_SESSIONS;

/// Adjusts scroll offset to ensure the selected index is visible within the window.
///
/// If no index is selected, does nothing.
pub fn scroll_to_cursor(state: &mut AppState) {
    let Some(index) = state.frontend.sessions_section.selected_index else {
        return;
    };
    let total = sorted_open_sessions(state).len();
    let visible = MAX_VISIBLE_SESSIONS.min(total);
    if visible == 0 {
        return;
    }
    let offset = &mut state.frontend.sessions_section.scroll_offset;

    if index < *offset {
        *offset = index;
    } else if index >= *offset + visible {
        *offset = index - visible + 1;
    }

    // Clamp offset so the window doesn't extend past the end of the list.
    // Without this, archiving sessions can leave the offset too large,
    // causing fewer entries to render than content_height reports and
    // the Sessions footer label to shift upward.
    let max_offset = total.saturating_sub(visible);
    *offset = (*offset).min(max_offset);
}

/// Updates the workflow preview based on the current cursor position.
///
/// If the cursor is on a workflow entry whose workflow exists in `WorkflowMap`,
/// sets `previewed_workflow_id`. Otherwise clears it.
fn update_preview(state: &mut AppState) {
    let preview = state
        .frontend
        .sessions_section
        .selected_index
        .and_then(|idx| {
            let sessions = sorted_open_sessions(state);
            let entry = sessions.get(idx)?;
            if entry.kind != SessionEntryKind::Workflow {
                return None;
            }
            let wf_id = entry.workflow_id.as_ref()?;
            state.workflow.get(wf_id)?;
            entry.workflow_id.clone()
        });
    state.frontend.sessions_section.previewed_workflow_id = preview;
}

/// Navigate within the sessions section.
///
/// Moves the cursor within the sessions list and immediately switches
/// the active session. Returns `Exhausted` when at a boundary or when
/// the list is empty.
pub fn navigate(intent: &SidebarIntent, state: &mut AppState) -> SectionNavResult {
    let sessions = sorted_open_sessions(state);
    if sessions.is_empty() {
        return SectionNavResult::Exhausted;
    }

    let result = match intent {
        SidebarIntent::MoveDown => {
            let current = state.frontend.sessions_section.selected_index.unwrap_or(0);
            if current >= sessions.len() - 1 {
                return SectionNavResult::Exhausted;
            }
            let new_index = current + 1;
            state.frontend.sessions_section.selected_index = Some(new_index);
            SectionNavResult::Moved
        }
        SidebarIntent::MoveUp => {
            let current = state.frontend.sessions_section.selected_index.unwrap_or(0);
            if current == 0 {
                return SectionNavResult::Exhausted;
            }
            let new_index = current - 1;
            state.frontend.sessions_section.selected_index = Some(new_index);
            SectionNavResult::Moved
        }
        SidebarIntent::Action(_) => SectionNavResult::Moved,
    };

    scroll_to_cursor(state);
    update_preview(state);
    result
}

/// Place the cursor on this section from a given direction.
///
/// Positions at the edge of the list: index 0 from top, last index from bottom.
/// This keeps the linear `j`/`k` scroll model consistent.
pub fn receive_cursor(state: &mut AppState, enter_from: EnterFrom) {
    let sessions = sorted_open_sessions(state);
    if sessions.is_empty() {
        return;
    }
    let index = match enter_from {
        EnterFrom::Top => 0,
        EnterFrom::Bottom => sessions.len() - 1,
    };
    state.frontend.sessions_section.selected_index = Some(index);
    scroll_to_cursor(state);
    update_preview(state);
}
