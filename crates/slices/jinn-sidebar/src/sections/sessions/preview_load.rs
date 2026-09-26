//! Requesting a session preview render, and the content identity that decides
//! whether one is needed.
//!
//! The preview popup's lines are rendered by the layout worker rather than the
//! frame loop, so moving the cursor over a large session costs the UI nothing but
//! a spinner. That only works if a render is *asked for* at the right moments,
//! which is what lives here.
//!
//! Requests originate on the keyboard path only. The render pass has no bus
//! handle — the frontend's job is to read state, not to publish — so it records
//! the width it drew at and draws a spinner when nothing is cached. A terminal
//! resize therefore re-requests on the next cursor move rather than instantly;
//! the alternative was handing the render path a publisher for one message.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use jinn_chat_log_view_msg::{PREVIEW_ENTRY_COUNT, PreviewSessionRequested};
use jinn_core_types::{ChatEntry, SessionId};
use jinn_domain::common::app_state::AppState;
use jinn_preferences_config::schemas::ChatLogConfig;
use jinn_slices::ConfigLayer;

use crate::sections::sessions::preview::DEFAULT_TOOL_ENTRY_MAX_LINES;

/// A summary of the content a preview would show.
///
/// Folds the trailing entries' own O(1) content signatures together, so it moves
/// when a streaming entry grows — which is the point. The history *length* does
/// not change for a whole turn while its content changes on every token, so a
/// length-keyed preview shows stale text until the reply ends.
#[must_use]
pub fn preview_signature(entries: &[ChatEntry], max_entries: usize) -> u64 {
    let start = entries.len().saturating_sub(max_entries);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for entry in entries.get(start..).unwrap_or_default() {
        entry.content_signature().hash(&mut hasher);
    }
    // The length is folded in as well, so dropping trailing entries stays
    // distinguishable from those entries merely changing content.
    entries.len().hash(&mut hasher);
    hasher.finish()
}

/// Arms a preview render for `session_id`, returning the request to publish.
///
/// Returns `None` when the preview is already current — re-requesting would
/// rebuild the lines off-thread only for the render pass to discard them.
#[must_use]
pub fn update_preview(
    state: &mut AppState,
    session_id: &SessionId,
    config: &ConfigLayer,
) -> Option<PreviewSessionRequested> {
    let width = state
        .frontend
        .with_sections(|s| s.sessions.preview_content_width, || 0);

    // One borrow of the history yields both the signature and the shared slice
    // the worker will read: taking them separately would walk the session map
    // twice and let the two disagree if it moved underneath.
    let (signature, entries) = {
        let session = state.session.get(session_id)?;
        let entries: Arc<[ChatEntry]> = Arc::from(session.history());
        (preview_signature(&entries, PREVIEW_ENTRY_COUNT), entries)
    };

    let already_current = state.frontend.with_sections(
        |s| {
            s.sessions
                .preview
                .cached(session_id, signature, width)
                .is_some()
        },
        || false,
    );
    if already_current {
        return None;
    }

    // The generation is bumped here, at the one place a request originates, so a
    // result belonging to a superseded request can be dropped on arrival. It is
    // read out of a local because `update_sections` yields `()`.
    let mut generation = 0;
    state.frontend.update_sections(|s| {
        generation = s.sessions.preview.request(session_id.clone());
    });

    Some(PreviewSessionRequested {
        session_id: session_id.clone(),
        content_width: width,
        generation,
        entries,
        tool_entry_max_lines: config
            .read::<ChatLogConfig>()
            .tool_entry_max_lines
            .unwrap_or(DEFAULT_TOOL_ENTRY_MAX_LINES),
        signature,
    })
}

#[cfg(test)]
mod preview_load_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::*;
    use jinn_domain::protocol::ChatEntry;
    use jinn_session_state::ChatSessionState;
    use jinn_sidebar_msg::PreviewLoad;

    /// App state holding one loaded session, with a recorded preview width.
    ///
    /// The id is read back off the inserted session rather than minted
    /// separately: the map keys on the session's own id, so an independently
    /// generated one would never be found.
    fn state_with_session(width: u16) -> (AppState, SessionId) {
        let mut state = AppState::default_with_scope_focus();
        let mut session = ChatSessionState::new();
        session.push_entry(ChatEntry::user("hello"));
        let id = session.session_id().clone();
        state.session.insert(session);
        state.session.set_active(id.clone());
        state
            .frontend
            .update_sections(|s| s.sessions.preview_content_width = width);
        (state, id)
    }

    /// Marks the current session's preview as served, so the trigger sees a hit.
    fn mark_ready(state: &AppState, id: &SessionId, width: u16) {
        let signature = {
            let session = state.session.get(id).expect("session");
            preview_signature(session.history(), PREVIEW_ENTRY_COUNT)
        };
        state.frontend.update_sections(|s| {
            let generation = s.sessions.preview.request(id.clone());
            s.sessions.preview.complete(
                id.clone(),
                generation,
                signature,
                width,
                Arc::new(vec![ratatui::text::Line::from("hello")]),
            );
        });
    }

    #[rstest::rstest]
    fn a_fresh_session_yields_a_preview_request() {
        // Given app state with a session whose preview has never been served.
        let (mut state, id) = state_with_session(40);

        // When the trigger runs.
        let request = update_preview(&mut state, &id, jinn_slices::empty_config_layer());

        // Then a request for that session is returned.
        assert_eq!(
            request.map(|r| r.session_id),
            Some(id),
            "expected a request for the highlighted session"
        );
    }

    #[rstest::rstest]
    fn a_current_preview_yields_no_request() {
        // Given app state whose preview is already served at the current width.
        let (mut state, id) = state_with_session(40);
        mark_ready(&state, &id, 40);

        // When the trigger runs.
        let request = update_preview(&mut state, &id, jinn_slices::empty_config_layer());

        // Then nothing is requested.
        assert!(request.is_none(), "a served preview must not re-request");
    }

    #[rstest::rstest]
    fn a_width_change_yields_a_request() {
        // Given app state whose preview was served at 40.
        let (mut state, id) = state_with_session(40);
        mark_ready(&state, &id, 40);

        // When the terminal is resized and the trigger runs.
        state
            .frontend
            .update_sections(|s| s.sessions.preview_content_width = 60);
        let request = update_preview(&mut state, &id, jinn_slices::empty_config_layer());

        // Then the preview is re-requested at the new width.
        assert_eq!(request.map(|r| r.content_width), Some(60));
    }

    #[rstest::rstest]
    fn a_streamed_token_yields_a_request() {
        // Given app state whose preview was served for a one-token reply.
        let (mut state, id) = state_with_session(40);
        mark_ready(&state, &id, 40);

        // When a further token lands in the streaming entry.
        state
            .session
            .get_mut(&id)
            .expect("session")
            .push_entry(ChatEntry::assistant("hello world"));

        // Then the preview is re-requested, because the text it shows moved.
        let request = update_preview(&mut state, &id, jinn_slices::empty_config_layer());
        assert!(
            request.is_some(),
            "a changed entry must invalidate a served preview"
        );
    }

    #[rstest::rstest]
    fn an_unknown_session_yields_no_request() {
        // Given app state with a different session than the one asked about.
        let (mut state, _id) = state_with_session(40);

        // When the trigger runs for an absent session.
        let request = update_preview(
            &mut state,
            &SessionId::new(),
            jinn_slices::empty_config_layer(),
        );

        // Then nothing is requested.
        assert!(
            request.is_none(),
            "an absent session has nothing to preview"
        );
    }

    #[rstest::rstest]
    fn a_request_arms_the_preview_as_loading() {
        // Given app state with a never-served session.
        let (mut state, id) = state_with_session(40);

        // When the trigger runs.
        let request = update_preview(&mut state, &id, jinn_slices::empty_config_layer());

        // Then the preview is armed, so the render pass draws a spinner.
        let armed = state.frontend.with_sections(
            |s| matches!(s.sessions.preview, PreviewLoad::Loading { .. }),
            || false,
        );
        assert!(
            armed,
            "the preview must be armed before the request publishes"
        );
        // And the request carries the generation it was armed with.
        let expected = state
            .frontend
            .with_sections(|s| s.sessions.preview.generation(), || 0);
        assert_eq!(request.map(|r| r.generation), Some(expected));
    }

    #[rstest::rstest]
    fn a_request_carries_the_trailing_history() {
        // Given a session holding eight entries.
        let (mut state, id) = state_with_session(40);
        for i in 0..7 {
            state
                .session
                .get_mut(&id)
                .expect("session")
                .push_entry(ChatEntry::user(format!("m{i}")));
        }

        // When the trigger runs.
        let request =
            update_preview(&mut state, &id, jinn_slices::empty_config_layer()).expect("request");

        // Then only the entries a preview can show travel with it.
        assert_eq!(request.entries.len(), 8);
    }

    #[rstest::rstest]
    fn the_signature_moves_when_an_entry_grows() {
        // Given the signature of a one-token reply.
        let before = {
            let session = ChatSessionState::new();
            let mut session = session;
            session.push_entry(ChatEntry::assistant("hel"));
            preview_signature(session.history(), PREVIEW_ENTRY_COUNT)
        };

        // When another token lands in the same entry.
        let mut session = ChatSessionState::new();
        session.push_entry(ChatEntry::assistant("hello"));
        let after = preview_signature(session.history(), PREVIEW_ENTRY_COUNT);

        // Then the signature moved, so a cached preview is invalidated.
        assert_ne!(before, after);
    }

    #[rstest::rstest]
    fn the_signature_is_stable_for_unchanged_content() {
        // Given the same content built twice.
        let signature = |text: &str| {
            let mut session = ChatSessionState::new();
            session.push_entry(ChatEntry::assistant(text));
            preview_signature(session.history(), PREVIEW_ENTRY_COUNT)
        };

        // Then the signatures match.
        assert_eq!(signature("hello"), signature("hello"));
    }

    #[rstest::rstest]
    fn the_signature_moves_when_the_history_grows() {
        // Given signatures for one and two entries of the same total content.
        let signature = |count: usize| {
            let mut session = ChatSessionState::new();
            for _ in 0..count {
                session.push_entry(ChatEntry::assistant("hi"));
            }
            preview_signature(session.history(), PREVIEW_ENTRY_COUNT)
        };

        // Then they differ, so dropping an entry is not mistaken for a no-op.
        assert_ne!(signature(1), signature(2));
    }
}
