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
//!
//! Because requests come from the keyboard, they are also where duplicates are
//! stopped: a second request for content already cached or already rendering
//! would queue a job for lines nothing would read.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use jinn_chat_log_view_msg::{PREVIEW_ENTRY_COUNT, PreviewSessionRequested};
use jinn_core_types::{ChatEntry, SessionId};
use jinn_domain::common::app_state::AppState;
use jinn_preferences_config::schemas::ChatLogConfig;
use jinn_slices::ConfigLayer;

use crate::sections::sessions::preview::DEFAULT_TOOL_ENTRY_MAX_LINES;
use crate::sections::sessions::state::sorted_open_sessions;

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
/// Returns `None` when the preview is already cached at this content and width,
/// or when an identical request is already running — re-requesting would
/// rebuild the lines off-thread only for the render pass to discard them, and
/// each duplicate is a job on a pool that is already busy with chat-log
/// measurement.
#[must_use]
pub fn update_preview(
    state: &mut AppState,
    session_id: &SessionId,
    config: &ConfigLayer,
) -> Option<PreviewSessionRequested> {
    let width = state
        .frontend
        .with_sections(|s| s.sessions.preview_content_width, || 0);

    // A width of zero means the render pass has not yet measured a frame, so
    // there is nothing to wrap for. Requesting anyway — at a nominal width, or
    // at whatever stale value a previous frame left behind — produces lines
    // that can never match the width the render pass looks them up at, and the
    // preview spins forever. Returning nothing instead leaves the cursor move
    // that follows the first frame to publish the real request.
    if width == 0 {
        return None;
    }

    // The signature is computed over the trailing entries but folded with the
    // *whole* history's length, so it is taken from a borrow of the full
    // history rather than the tail that is about to be shared. Skipping ahead
    // first would make dropping an entry indistinguishable from its content
    // changing, and a stale preview would be served after a prune.
    let signature = {
        let session = state.session.get(session_id)?;
        preview_signature(session.history(), PREVIEW_ENTRY_COUNT)
    };

    if preview_is_current(state, session_id, signature, width) {
        // A hit is also a use: it says which session the user is looking at, so
        // it refreshes that preview's recency against eviction.
        state
            .frontend
            .update_sections(|s| s.sessions.preview.touch(session_id));
        return None;
    }

    // Only now is a copy worth making. The worker reads only the trailing
    // entries — `render_preview` slices to its own `max_entries` regardless —
    // so a long history's earlier entries are copied on every keystroke for no
    // reader. The borrow of the session has to end first, since `update_sections`
    // below takes the sections lock and the Arc is built from what it borrowed.
    let entries = {
        let session = state.session.get(session_id)?;
        let history = session.history();
        let start = history.len().saturating_sub(PREVIEW_ENTRY_COUNT);
        Arc::<[ChatEntry]>::from(history.get(start..).unwrap_or_default())
    };

    // The generation is bumped here, at the one place a request originates, so a
    // result belonging to a superseded request can be dropped on arrival. It is
    // read out of a local because `update_sections` yields `()`.
    let mut generation = 0;
    state.frontend.update_sections(|s| {
        generation = s
            .sessions
            .preview
            .request(session_id.clone(), signature, width);
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

/// Whether the preview for this session, content, and width is already handled.
///
/// A cache hit means the render pass can draw immediately. An in-flight match
/// means the render is already running and will land on its own. Both are read
/// under one lock so they cannot disagree with each other.
fn preview_is_current(
    state: &AppState,
    session_id: &SessionId,
    signature: u64,
    content_width: u16,
) -> bool {
    state.frontend.with_sections(
        |s| {
            s.sessions
                .preview
                .cached(session_id, signature, content_width)
                .is_some()
                || s.sessions
                    .preview
                    .in_flight_matches(session_id, signature, content_width)
        },
        || false,
    )
}

/// The preview request to publish for the session under the cursor, if one is
/// needed and the popup is actually on screen.
///
/// The keyboard path asks for previews when the cursor *moves*, but the cursor
/// is not the only thing that changes what the popup should show: a session can
/// finish loading into the list, or a frame can measure the width, long after
/// the last keystroke. Every such transition left a popup spinning on a request
/// nobody had made, resolved only by nudging the cursor.
///
/// So the render pass asks too, through the same builder the keyboard path
/// uses, with the same dedupe against the cache and against in-flight renders.
/// A settled cursor therefore asks once and then stays silent: the request is
/// only produced while the cache is empty and nothing is running, and the
/// result's arrival fills the cache and ends the ask.
///
/// Returns `None` — publishing nothing — when the sessions section is not
/// focused, the cursor is on nothing, or the popup has what it needs.
#[must_use]
pub fn request_preview_if_needed(
    state: &mut AppState,
    config: &jinn_slices::ConfigLayer,
) -> Option<PreviewSessionRequested> {
    if state.frontend.sidebar_section() != Some(jinn_sidebar_msg::SidebarSectionId::Sessions) {
        return None;
    }
    let index = state
        .frontend
        .with_sections(|s| s.sessions.selected_index, || None)?;
    let session_id = sorted_open_sessions(state).get(index)?.id.clone();
    update_preview(state, &session_id, config)
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

    /// The signature the trigger would compute for `id` right now.
    fn signature_of(state: &AppState, id: &SessionId) -> u64 {
        let session = state.session.get(id).expect("session");
        preview_signature(session.history(), PREVIEW_ENTRY_COUNT)
    }

    /// Marks the current session's preview as served, so the trigger sees a hit.
    fn mark_ready(state: &AppState, id: &SessionId, width: u16) {
        let signature = signature_of(state, id);
        state.frontend.update_sections(|s| {
            let generation = s.sessions.preview.request(id.clone(), signature, width);
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
    fn an_identical_in_flight_request_is_not_republished() {
        // Given a request already running for a session at this width.
        let (mut state, id) = state_with_session(40);
        update_preview(&mut state, &id, jinn_slices::empty_config_layer()).expect("first request");

        // When the trigger runs again for the same session, content, and width.
        let request = update_preview(&mut state, &id, jinn_slices::empty_config_layer());

        // Then nothing is published — the render is already running and a
        // second job would queue behind it for lines nothing would read.
        assert!(
            request.is_none(),
            "an identical in-flight request must not be republished"
        );
    }

    #[rstest::rstest]
    fn a_different_width_still_yields_a_request_while_one_is_in_flight() {
        // Given a request running for a session at 40.
        let (mut state, id) = state_with_session(40);
        update_preview(&mut state, &id, jinn_slices::empty_config_layer()).expect("first request");

        // When the terminal is resized and the trigger runs again.
        state
            .frontend
            .update_sections(|s| s.sessions.preview_content_width = 60);
        let request = update_preview(&mut state, &id, jinn_slices::empty_config_layer());

        // Then a request is published, because the running one wrapped the text
        // at a width the popup no longer has.
        assert_eq!(request.map(|r| r.content_width), Some(60));
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
    fn a_request_arms_the_preview_as_in_flight() {
        // Given app state with a never-served session.
        let (mut state, id) = state_with_session(40);

        // When the trigger runs.
        let request = update_preview(&mut state, &id, jinn_slices::empty_config_layer())
            .expect("a fresh session must request");

        // Then the preview is armed at the request's own generation, so the
        // result is recognised as the live one when it lands.
        let armed = state.frontend.with_sections(
            |s| {
                s.sessions
                    .preview
                    .in_flight_matches(&id, request.signature, 40)
            },
            || false,
        );
        assert!(
            armed,
            "the preview must be armed before the request publishes"
        );
    }

    #[rstest::rstest]
    fn a_completed_empty_preview_is_served_as_empty_not_loading() {
        // Given a session with no entries at all — a brand-new session, whose
        // chat view is already showing because there is nothing to load.
        let mut state = AppState::default_with_scope_focus();
        let session = ChatSessionState::new();
        let id = session.session_id().clone();
        state.session.insert(session);
        state.session.set_active(id.clone());
        state
            .frontend
            .update_sections(|s| s.sessions.preview_content_width = 46);

        // When a preview is requested and its (empty) result comes back.
        let request = update_preview(&mut state, &id, jinn_slices::empty_config_layer())
            .expect("an empty session still needs rendering to establish it is empty");
        let signature = request.signature;
        let generation = request.generation;
        let width = request.content_width;
        state.frontend.update_sections(|s| {
            s.sessions.preview.complete(
                id.clone(),
                generation,
                signature,
                width,
                Arc::new(Vec::new()),
            );
        });

        // When the render pass looks it up.
        let found = state.frontend.with_sections(
            |s| s.sessions.preview.cached(&id, signature, width).is_some(),
            || false,
        );

        // Then it is found. An empty session is a *completed* preview holding
        // zero lines, not a missing one: the popup tells loading from empty by
        // whether the lookup hit, so reporting a miss here is what puts a
        // spinner on a session that has nothing to wait for.
        assert!(
            found,
            "an empty session's preview was reported as missing, so the popup spins forever"
        );
    }

    /// State focused on the sessions section, with the cursor on its first row.
    ///
    /// The session is marked `Loaded` because the list the popup resolves its
    /// entry through only contains loaded sessions — a session still loading has
    /// no row to preview, which is a different situation entirely.
    fn state_focused_on_sessions() -> AppState {
        let (mut state, id) = state_with_session(46);
        if let Some(session) = state.session.get_mut(&id) {
            session.set_session_state(jinn_session_store_msg::SessionState::Loaded);
        }
        // The scope is pushed before the section is set: `set_sidebar_section`
        // is a no-op on a stack with no sidebar scope on it, so setting the
        // section alone would leave the sidebar unfocused and silent.
        state
            .frontend
            .scope_push(jinn_sidebar_msg::SidebarSectionId::Sessions.focus_scope());
        state
            .frontend
            .update_sections(|s| s.sessions.selected_index = Some(0));
        state
    }

    #[rstest::rstest]
    fn a_stationary_cursor_still_asks_for_its_preview() {
        // Given the sidebar focused on a session whose preview has never been
        // rendered, and the cursor not going to move again.
        let mut state = state_focused_on_sessions();

        // When the render pass asks.
        let request = request_preview_if_needed(&mut state, jinn_slices::empty_config_layer());

        // Then it asks. The cursor is the only thing that used to trigger this,
        // so a session that loaded, or a width that got measured, after the last
        // key left the popup spinning until the user nudged the cursor.
        assert!(
            request.is_some(),
            "nothing asked for the preview, so a stationary cursor would spin forever"
        );
    }

    #[rstest::rstest]
    fn a_stationary_cursor_stops_asking_once_the_preview_is_cached() {
        // Given the sidebar focused, and a preview already rendered and cached
        // for the session the list resolves the cursor to — the same session the
        // render path will ask about, since a cache keyed to any other id would
        // not be the one it finds.
        let mut state = state_focused_on_sessions();
        let id = sorted_open_sessions(&state)[0].id.clone();
        let signature = signature_of(&state, &id);
        // Armed first, exactly as a real request is: a result is only accepted
        // for a generation that was actually issued, so completing one that was
        // never armed is refused by design rather than filling the cache.
        let armed = state
            .frontend
            .update_sections(|s| s.sessions.preview.request(id.clone(), signature, 46))
            .expect("the sections cell is attached");
        state.frontend.update_sections(|s| {
            s.sessions
                .preview
                .complete(id.clone(), armed, signature, 46, Arc::new(Vec::new()));
        });

        // When the render pass asks again, as it does every frame.
        let request = request_preview_if_needed(&mut state, jinn_slices::empty_config_layer());

        // Then it stops asking. Publishing per frame would flood the render pool
        // with work that is already done, which is the failure this dedupe
        // exists to prevent.
        assert!(
            request.is_none(),
            "the render pass re-requested a preview it already has"
        );
    }

    #[rstest::rstest]
    fn nothing_is_asked_for_while_the_sessions_section_is_unfocused() {
        // Given the sidebar not focused on sessions, where no popup is drawn.
        let (mut state, _) = state_with_session(46);

        // When the render pass asks.
        let request = request_preview_if_needed(&mut state, jinn_slices::empty_config_layer());

        // Then it asks for nothing: there is no popup on screen to fill.
        assert!(request.is_none());
    }

    #[rstest::rstest]
    fn a_request_waits_for_a_measured_width() {
        // Given app state where no frame has been measured yet — the width is
        // still zero, so there is nothing to wrap for.
        let (mut state, id) = state_with_session(0);

        // When the trigger runs.
        let request = update_preview(&mut state, &id, jinn_slices::empty_config_layer());

        // Then it asks for nothing. A request at a width no frame will ever
        // report produces lines the render pass cannot match, and the popup
        // spins forever; the pre-render pass measures the width before the
        // next cursor move republishes.
        assert!(
            request.is_none(),
            "no request should be made before a width has been measured"
        );
    }

    #[rstest::rstest]
    fn a_request_names_the_width_the_render_pass_looks_up_at() {
        // Given app state carrying the width the pre-render pass recorded.
        let (mut state, id) = state_with_session(3);
        let measured = 46;
        state
            .frontend
            .update_sections(|s| s.sessions.preview_content_width = measured);

        // When the trigger runs.
        let request = update_preview(&mut state, &id, jinn_slices::empty_config_layer())
            .expect("a measured width must request");

        // Then it names exactly that width. A request at any other width can
        // never match the lookup the render pass performs, which is what left
        // the popup spinning on a cache entry that had already been filled.
        assert_eq!(request.content_width, measured);
    }

    #[rstest::rstest]
    fn a_completed_preview_is_found_at_the_width_the_render_pass_derives() {
        // Given a session, and the width the pre-render pass measured for the
        // frame the render pass will draw.
        let (mut state, id) = state_with_session(3);
        let frame_area = ratatui::layout::Rect::new(0, 0, 100, 40);
        let measured = crate::sections::sessions::preview::preview_content_width(frame_area);
        state
            .frontend
            .update_sections(|s| s.sessions.preview_content_width = measured);

        // When a preview is requested and its result comes back.
        let request = update_preview(&mut state, &id, jinn_slices::empty_config_layer())
            .expect("a measured width must request");
        let signature = request.signature;
        let generation = request.generation;
        let width = request.content_width;
        state.frontend.update_sections(|s| {
            s.sessions.preview.complete(
                id.clone(),
                generation,
                signature,
                width,
                std::sync::Arc::new(Vec::new()),
            );
        });

        // When the render pass then looks the preview up, at the width it
        // derives from the same frame rather than reading back what was stored.
        let lookup_width = crate::sections::sessions::preview::preview_content_width(frame_area);
        let found = state.frontend.with_sections(
            |s| {
                s.sessions
                    .preview
                    .cached(&id, signature, lookup_width)
                    .is_some()
            },
            || false,
        );

        // Then the preview is found. The two widths are derived from the same
        // frame by the same function, so a request can never be rendered at
        // one width and looked up at another — the defect that filled the cache
        // and still reported a miss on every frame, spinning forever.
        assert!(
            found,
            "a preview rendered at width {} was not found at the width the render pass \
             looks up at ({lookup_width})",
            request.content_width
        );
    }

    #[rstest::rstest]
    fn a_request_carries_only_the_trailing_history() {
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

        // Then only the entries a preview can show travel with it. The worker
        // slices to `PREVIEW_ENTRY_COUNT` regardless, so a longer slice would be
        // copied on every keystroke with nothing reading it.
        assert_eq!(request.entries.len(), PREVIEW_ENTRY_COUNT);
    }

    #[rstest::rstest]
    fn a_request_carries_the_newest_entry() {
        // Given a session whose newest entry is distinguishable.
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

        // Then the entries that travelled are the newest ones, so trimming to
        // the preview window kept the tail rather than the head. Read from
        // `text()` rather than `content_signature()`: the signature folds only
        // content *lengths*, so equally-sized entries are indistinguishable by it
        // and it would compare equal here for the wrong reason.
        assert_eq!(
            request.entries.last().map(ChatEntry::text),
            Some("m6".to_owned()),
            "the newest entry must travel with the request"
        );
        assert_eq!(
            request.entries.first().map(ChatEntry::text),
            Some("m2".to_owned()),
            "the tail must be kept, not the head of the history"
        );
    }

    #[rstest::rstest]
    fn a_signature_moves_when_an_earlier_entry_is_dropped() {
        // Given a session whose history has seven entries.
        let mut session = ChatSessionState::new();
        for i in 0..7 {
            session.push_entry(ChatEntry::assistant(format!("entry {i}")));
        }
        let before = preview_signature(session.history(), PREVIEW_ENTRY_COUNT);

        // When its earliest entries are pruned, leaving the same trailing window.
        let trimmed = ChatSessionState::new();
        let after = {
            let mut kept = trimmed;
            for i in 3..7 {
                kept.push_entry(ChatEntry::assistant(format!("entry {i}")));
            }
            preview_signature(kept.history(), PREVIEW_ENTRY_COUNT)
        };

        // Then the signature moved, so a preview of the longer history is not
        // served for the shorter one — the length fold is what distinguishes a
        // prune from the trailing entries merely being unchanged.
        assert_ne!(before, after);
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
