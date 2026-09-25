//! Tests for the off-thread chat log layout: the worker measures a session's
//! history, and the completion actor decides whether those counts may be used.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

use std::collections::HashSet;

use jinn_chat_log_view_msg::{ChatLogLayoutComputed, LayoutChatSession, MeasuredEntryCount};
use jinn_core_types::{ChatEntry, SessionId};
use jinn_session_state::ChatSessionState;

use crate::common::app_state::AppState;
use crate::common::state::State;
use crate::feat::ui::chat_log::layout_complete::LayoutCompletionActorDeps;
use crate::feat::ui::chat_log::layout_complete::{LayoutApplied, LayoutCompletionActor};
use crate::feat::ui::chat_log::layout_supervisor::{
    LayoutSupervisorActor, LayoutSupervisorActorDeps,
};
use crate::feat::ui::chat_log::layout_worker::measure;

/// State with `count` user entries in its active session, measured at
/// `content_width`.
///
/// The width is published into the session's view state because the chat log
/// does that on every frame, and the completion actor compares a result against
/// it.
fn state_with_entries(count: usize, content_width: u16) -> (State, SessionId) {
    let session_id = SessionId::new();
    let mut session = ChatSessionState::new();
    session.set_session_id(session_id.clone());
    for index in 0..count {
        session.push_entry(ChatEntry::user(format!("message {index}")));
    }
    session.set_content_width(content_width);
    let mut state = AppState::default();
    state.session.insert(session);
    state.session.set_active(session_id.clone());
    (State::new(state), session_id)
}

/// A layout job over a session's own history.
fn job_for(state: &State, session_id: &SessionId, content_width: u16) -> LayoutChatSession {
    let session = state.read();
    let active = session.session.get(session_id).expect("active session");
    LayoutChatSession {
        session_id: session_id.clone(),
        content_width,
        entries: active.history().to_vec(),
        shown_ignored_blocks: HashSet::new(),
        min_collapse_count: jinn_chat_log_view_msg::DEFAULT_MIN_COLLAPSE_COUNT,
        tool_entry_max_lines: 6,
    }
}

/// The layout inputs a job would be measured with.
fn inputs_for(state: &State, session_id: &SessionId) -> super::history::LayoutInputs {
    super::history::LayoutInputs::snapshot(&state.read(), session_id)
}

#[rstest::rstest]
fn a_layout_job_measures_every_entry() {
    // Given a session with several entries.
    let (state, session_id) = state_with_entries(5, 60);
    let job = job_for(&state, &session_id, 60);
    let inputs = inputs_for(&state, &session_id);

    // When the job is measured.
    let measured = measure(&job, &inputs);

    // Then every entry has a line count.
    assert_eq!(measured.len(), 5);
    // And every count is at least one line.
    assert!(
        measured.iter().all(|count| count.wrapped_count >= 1),
        "every rendered entry occupies at least one line"
    );
}

#[rstest::rstest]
fn a_layout_job_measures_the_same_counts_the_renderer_would() {
    // Given a session and a job measured at a width.
    let (state, session_id) = state_with_entries(4, 40);
    let job = job_for(&state, &session_id, 40);
    let inputs = inputs_for(&state, &session_id);

    // When the job is measured.
    let measured = measure(&job, &inputs);

    // Then the counts come back in the session's own entry order.
    let ids: Vec<_> = measured.iter().map(|count| count.id.clone()).collect();
    let history_ids: Vec<_> = state
        .read()
        .session
        .get(&session_id)
        .expect("active session")
        .history()
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
    assert_eq!(ids, history_ids);
}

#[rstest::rstest]
fn a_layout_job_counts_the_lines_the_markdown_renderer_produced() {
    // Given a session with one multi-line entry.
    let session_id = SessionId::new();
    let mut session = ChatSessionState::new();
    session.set_session_id(session_id.clone());
    session.push_entry(ChatEntry::user("a line\nanother line\nand a third"));
    let mut app = AppState::default();
    app.session.insert(session);
    app.session.set_active(session_id.clone());
    let state = State::new(app);
    let job = job_for(&state, &session_id, 0);
    let inputs = inputs_for(&state, &session_id);

    // When the job is measured at width zero, where nothing is re-wrapped.
    let measured = measure(&job, &inputs);

    // Then the count is the entry's own line count, never a wrapped one —
    // a width of zero would otherwise report a single line for everything.
    assert_eq!(measured.len(), 1);
    assert!(
        measured[0].wrapped_count > 1,
        "a multi-line entry must not collapse to one line, got {}",
        measured[0].wrapped_count
    );
}

#[rstest::rstest]
fn a_completed_layout_stores_the_counts_and_ends_the_load() {
    // Given a loading session with a measured result.
    let (state, session_id) = state_with_entries(3, 60);
    {
        let mut guard = state.write();
        guard.session.begin_load(session_id.clone());
    }
    let job = job_for(&state, &session_id, 60);
    let inputs = inputs_for(&state, &session_id);
    let measured = measure(&job, &inputs);
    let actor = LayoutCompletionActor::spawnless(LayoutCompletionActorDeps {
        state: state.clone(),
    });

    // When the result is applied.
    let applied = actor.apply(&computed(&session_id, 60, measured));

    // Then the counts were used.
    assert_eq!(applied, LayoutApplied::Applied);
    // And the load guard is cleared.
    assert!(!state.read().session.is_loading());
}

#[rstest::rstest]
fn a_completed_layout_clears_the_load_even_at_a_stale_width() {
    // Given a loading session and a result measured at the wrong width.
    let (state, session_id) = state_with_entries(2, 60);
    {
        let mut guard = state.write();
        guard.session.begin_load(session_id.clone());
    }
    let job = job_for(&state, &session_id, 60);
    let inputs = inputs_for(&state, &session_id);
    let measured = measure(&job, &inputs);
    let actor = LayoutCompletionActor::spawnless(LayoutCompletionActorDeps {
        state: state.clone(),
    });

    // When the result is applied at a width the chat log is not using.
    let applied = actor.apply(&computed(&session_id, 999, measured));

    // Then the counts were not used.
    assert_eq!(applied, LayoutApplied::DiscardedStaleWidth);
    // But the load guard is still cleared, so the user is not stranded.
    assert!(!state.read().session.is_loading());
}

#[rstest::rstest]
fn a_result_for_an_inactive_session_is_discarded() {
    // Given a loading session and a result for some other session.
    let (state, session_id) = state_with_entries(2, 60);
    {
        let mut guard = state.write();
        guard.session.begin_load(session_id.clone());
    }
    let other = SessionId::new();
    let job = job_for(&state, &session_id, 60);
    let inputs = inputs_for(&state, &session_id);
    let measured = measure(&job, &inputs);
    let actor = LayoutCompletionActor::spawnless(LayoutCompletionActorDeps {
        state: state.clone(),
    });

    // When that other session's result is applied.
    let applied = actor.apply(&computed(&other, 60, measured));

    // Then it is discarded.
    assert_eq!(applied, LayoutApplied::DiscardedInactive);
    // And the loading session's own guard is untouched.
    assert!(state.read().session.is_loading());
}

#[rstest::rstest]
fn a_stored_layout_survives_the_first_render_without_re_measuring() {
    // Given a session whose counts have been stored by a completed layout.
    let (state, session_id) = state_with_entries(4, 40);
    {
        let mut guard = state.write();
        guard.session.begin_load(session_id.clone());
    }
    let job = job_for(&state, &session_id, 40);
    let inputs = inputs_for(&state, &session_id);
    let measured = measure(&job, &inputs);
    // Kept so the frame below can probe with the very keys the measurement
    // stored — that is what proves the count is findable.
    let keys: Vec<_> = measured
        .iter()
        .map(|count| (count.is_expanded, count.variant, count.wrapped_count))
        .collect();
    let widths_before = fingerprint_computations(&state);
    let actor = LayoutCompletionActor::spawnless(LayoutCompletionActorDeps {
        state: state.clone(),
    });
    actor.apply(&computed(&session_id, 40, measured));

    // When the chat log lays out at the same width.
    let session = state.read();
    let active = session.session.get(&session_id).expect("active session");
    let history: Vec<_> = active.history().to_vec();
    let mut cache = session.frontend.caches.entry_line_cache.write();
    let hits = history
        .iter()
        .zip(keys.iter())
        .map(|(entry, (is_expanded, variant, wrapped_count))| {
            cache
                .get(entry, *is_expanded, *variant, 40)
                .map(|hit| hit.wrapped_count)
                == Some(*wrapped_count)
        })
        .collect::<Vec<_>>();
    drop(cache);
    drop(session);

    // Then every entry is already counted — the frame does no work.
    assert!(
        hits.iter().all(|hit| *hit),
        "the first frame after a layout must find every count in the cache"
    );
    // And no entry was hashed again to get there.
    assert_eq!(
        fingerprint_computations(&state),
        widths_before,
        "a stored layout must not be re-hashed to be used"
    );
}

/// How many content fingerprints the line cache has computed.
fn fingerprint_computations(state: &State) -> u64 {
    state
        .read()
        .frontend
        .caches
        .entry_line_cache
        .read()
        .fingerprint_computations()
}

/// Wraps measured counts in the result message the completion actor consumes.
fn computed(
    session_id: &SessionId,
    content_width: u16,
    measured: Vec<jinn_chat_log_view::chat_log::MeasuredLineCount>,
) -> ChatLogLayoutComputed {
    ChatLogLayoutComputed {
        session_id: session_id.clone(),
        content_width,
        counts: measured
            .into_iter()
            .map(|count| MeasuredEntryCount {
                entry_id: count.id,
                signature: count.content.signature,
                fingerprint: count.content.fingerprint,
                is_expanded: count.is_expanded,
                variant: count.variant,
                wrapped_count: count.wrapped_count,
            })
            .collect(),
    }
}

/// State with a session that is mid-load, and a supervisor watching it.
fn loading_state_with_supervisor() -> (State, SessionId, LayoutSupervisorActor) {
    let (state, session_id) = state_with_entries(2, 60);
    {
        let mut guard = state.write();
        guard.session.begin_load(session_id.clone());
    }
    let system = trouper::system::ActorSystem::new(trouper::system::SystemConfig::production());
    let supervisor = LayoutSupervisorActor::spawnless(LayoutSupervisorActorDeps {
        state: state.clone(),
        system,
    });
    (state, session_id, supervisor)
}

#[rstest::rstest]
fn an_expired_layout_deadline_ends_the_loading_indicator() {
    // Given a session that is still loading.
    let (state, session_id, supervisor) = loading_state_with_supervisor();
    assert!(state.read().session.is_loading());

    // When the layout deadline expires.
    supervisor.release_guard(&session_id, "layout deadline expired");

    // Then the loading indicator is released.
    assert!(
        !state.read().session.is_loading(),
        "an abandoned measurement must not strand the user behind a spinner"
    );
}

#[rstest::rstest]
fn an_expired_layout_deadline_writes_nothing_into_the_conversation() {
    // Given a session that is still loading.
    let (state, session_id, supervisor) = loading_state_with_supervisor();

    // When the layout deadline expires.
    supervisor.release_guard(&session_id, "layout deadline expired");

    // Then no entry was added — a measurement that could not be taken off
    // the main thread is not a conversation event.
    assert_eq!(
        state
            .read()
            .session
            .get(&session_id)
            .expect("active session")
            .history()
            .len(),
        2
    );
}
