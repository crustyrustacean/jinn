//! Navigation within the sessions sidebar section.

use crate::sections::section_trait::{EnterFrom, SectionNavResult, SidebarIntent};
use crate::sections::sessions::state::sorted_open_sessions;
use jinn_domain::common::app_state::AppState;

/// No-op: session preview removed with node-graph.
fn update_preview(_state: &mut AppState) {}

/// Navigate within the sessions section.
///
/// Moves the cursor within the sessions list.
/// Returns `Exhausted` when at a boundary or when the list is empty.
/// The cursor lands on all entries (sessions only).
pub fn navigate(intent: &SidebarIntent, state: &mut AppState) -> SectionNavResult {
    let sessions = sorted_open_sessions(state);
    if sessions.is_empty() {
        return SectionNavResult::Exhausted;
    }

    let result = match intent {
        SidebarIntent::MoveDown => {
            let current = state
                .frontend
                .with_sections(|s| s.sessions.selected_index, || None)
                .unwrap_or(0);
            let new_index = current.saturating_add(1);
            if new_index >= sessions.len() {
                return SectionNavResult::Exhausted;
            }
            state
                .frontend
                .update_sections(|s| s.sessions.selected_index = Some(new_index));
            SectionNavResult::Moved
        }
        SidebarIntent::MoveUp => {
            let current = state
                .frontend
                .with_sections(|s| s.sessions.selected_index, || None)
                .unwrap_or(0);
            if current == 0 {
                return SectionNavResult::Exhausted;
            }
            state
                .frontend
                .update_sections(|s| s.sessions.selected_index = Some(current - 1));
            SectionNavResult::Moved
        }
        SidebarIntent::Action(_) => SectionNavResult::Moved,
    };

    update_preview(state);
    result
}

/// Place the cursor on this section from a given direction.
///
/// Positions at the edge of the list: index 0 from top, last index from bottom.
pub fn receive_cursor(state: &mut AppState, enter_from: EnterFrom) {
    let sessions = sorted_open_sessions(state);
    if sessions.is_empty() {
        return;
    }
    let index = match enter_from {
        EnterFrom::Top => 0,
        EnterFrom::Bottom => sessions.len() - 1,
    };
    state
        .frontend
        .update_sections(|s| s.sessions.selected_index = Some(index));

    update_preview(state);
}
