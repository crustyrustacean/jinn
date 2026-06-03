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

/// No-op: workflow preview removed with node-graph.
fn update_preview(_state: &mut AppState) {}

/// Find the next session entry starting from `start` (inclusive), searching in `direction`.
///
/// Returns `None` if no session entry is found before hitting the list boundary.
fn next_session_index(
    entries: &[crate::feat::ui::sidebar::sessions::state::SessionEntry],
    start: usize,
    direction: Direction,
) -> Option<usize> {
    match direction {
        Direction::Down => {
            let mut i = start;
            while i < entries.len() {
                if matches!(entries[i].kind, SessionEntryKind::Session) {
                    return Some(i);
                }
                i += 1;
            }
            None
        }
        Direction::Up => {
            let mut i = start;
            loop {
                if matches!(entries[i].kind, SessionEntryKind::Session) {
                    return Some(i);
                }
                if i == 0 {
                    return None;
                }
                i -= 1;
            }
        }
    }
}

enum Direction {
    Down,
    Up,
}

/// Navigate within the sessions section.
///
/// Moves the cursor within the sessions list and immediately switches
/// the active session. Returns `Exhausted` when at a boundary or when
/// the list is empty. Skips over workflow entries — the cursor only lands
/// on session entries.
pub fn navigate(intent: &SidebarIntent, state: &mut AppState) -> SectionNavResult {
    let sessions = sorted_open_sessions(state);
    if sessions.is_empty() {
        return SectionNavResult::Exhausted;
    }

    let result = match intent {
        SidebarIntent::MoveDown => {
            let current = state.frontend.sessions_section.selected_index.unwrap_or(0);
            let start = current.saturating_add(1);
            let Some(new_index) = next_session_index(&sessions, start, Direction::Down) else {
                return SectionNavResult::Exhausted;
            };
            state.frontend.sessions_section.selected_index = Some(new_index);
            SectionNavResult::Moved
        }
        SidebarIntent::MoveUp => {
            let current = state.frontend.sessions_section.selected_index.unwrap_or(0);
            if current == 0 {
                return SectionNavResult::Exhausted;
            }
            let start = current - 1;
            let Some(new_index) = next_session_index(&sessions, start, Direction::Up) else {
                return SectionNavResult::Exhausted;
            };
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
/// Skips over workflow entries so the cursor lands on a session.
pub fn receive_cursor(state: &mut AppState, enter_from: EnterFrom) {
    let sessions = sorted_open_sessions(state);
    if sessions.is_empty() {
        return;
    }
    let index = match enter_from {
        EnterFrom::Top => next_session_index(&sessions, 0, Direction::Down),
        EnterFrom::Bottom => next_session_index(&sessions, sessions.len() - 1, Direction::Up),
    };
    if let Some(index) = index {
        state.frontend.sessions_section.selected_index = Some(index);
        scroll_to_cursor(state);
        update_preview(state);
    }
}
