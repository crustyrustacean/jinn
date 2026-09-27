//! Navigation within the sessions sidebar section.

use crate::sections::section_trait::{EnterFrom, SectionNavResult, SidebarIntent};
use crate::sections::sessions::preview_load::update_preview;
use crate::sections::sessions::state::sorted_open_sessions;
use jinn_core_types::SessionId;
use jinn_kernel::common::app_state::AppState;
use jinn_kernel::protocol::IntentResult;
use jinn_slices::ConfigLayer;

use crate::sections::sidebar_state_actor::PREVIEW_DEADLINE;
use jinn_chat_log_view_msg::{ArmPreviewDeadline, PreviewSessionRequested};

/// The session under the cursor, if there is one.
fn highlighted_session(
    state: &AppState,
    sessions: &[jinn_sidebar_msg::SessionEntry],
) -> Option<SessionId> {
    let index = state
        .frontend
        .with_sections(|s| s.sessions.selected_index, || None)?;
    sessions.get(index).map(|entry| entry.id.clone())
}

/// Asks for the highlighted session's preview, if the cursor is on one.
///
/// Best-effort: a missing session, or a preview already current, simply yields
/// nothing to publish.
///
/// The deadline rides along with the request rather than being armed by an actor
/// subscribed to it. `PreviewSessionRequested` is a *command*, so trouper routes
/// it to exactly one handler; an actor that merely wanted to arm a timer would
/// be a second handler competing with the preview workers, and every request it
/// won would be consumed without ever being rendered.
///
/// A request without its deadline is a spinner that never expires, so the two
/// travel together and every caller — the keyboard path here and the render
/// pass — sends them as a pair. [`preview_messages`] is the single place that
/// builds them.
fn request_preview(state: &mut AppState, config: &ConfigLayer) -> IntentResult {
    let sessions = sorted_open_sessions(state);
    let Some(session_id) = highlighted_session(state, &sessions) else {
        return IntentResult::empty();
    };
    update_preview(state, &session_id, config).map_or_else(IntentResult::empty, preview_messages)
}

/// A built request paired with the deadline that bounds it.
///
/// Published together by every caller. Keeping them in one builder is what
/// stops a path from arming a render it never watches, which is a spinner with
/// nothing to end it.
#[must_use]
pub fn preview_messages(request: PreviewSessionRequested) -> IntentResult {
    let deadline = ArmPreviewDeadline {
        session_id: request.session_id.clone(),
        generation: request.generation,
        after: PREVIEW_DEADLINE,
    };
    IntentResult::new_message(request).with_message(deadline)
}

/// Navigate within the sessions section.
///
/// Moves the cursor within the sessions list.
/// Returns `Exhausted` when at a boundary or when the list is empty.
/// The cursor lands on all entries (sessions only).
///
/// The second element is the preview request for wherever the cursor came to
/// rest, so the caller can publish it alongside whatever else the move did.
#[must_use]
pub fn navigate(
    intent: &SidebarIntent,
    state: &mut AppState,
    config: &ConfigLayer,
) -> (SectionNavResult, IntentResult) {
    let sessions = sorted_open_sessions(state);
    if sessions.is_empty() {
        return (SectionNavResult::Exhausted, IntentResult::empty());
    }

    let result = match intent {
        SidebarIntent::MoveDown => {
            let current = state
                .frontend
                .with_sections(|s| s.sessions.selected_index, || None)
                .unwrap_or(0);
            let new_index = current.saturating_add(1);
            if new_index >= sessions.len() {
                return (SectionNavResult::Exhausted, IntentResult::empty());
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
                return (SectionNavResult::Exhausted, IntentResult::empty());
            }
            state
                .frontend
                .update_sections(|s| s.sessions.selected_index = Some(current - 1));
            SectionNavResult::Moved
        }
        SidebarIntent::Action(_) => SectionNavResult::Moved,
    };

    (result, request_preview(state, config))
}

/// Place the cursor on this section from a given direction.
///
/// Positions at the edge of the list: index 0 from top, last index from bottom.
/// Returns the preview request for the session the cursor landed on.
#[must_use]
pub fn receive_cursor(
    state: &mut AppState,
    enter_from: EnterFrom,
    config: &ConfigLayer,
) -> IntentResult {
    let sessions = sorted_open_sessions(state);
    if sessions.is_empty() {
        return IntentResult::empty();
    }
    let index = match enter_from {
        EnterFrom::Top => 0,
        EnterFrom::Bottom => sessions.len() - 1,
    };
    state
        .frontend
        .update_sections(|s| s.sessions.selected_index = Some(index));

    request_preview(state, config)
}
