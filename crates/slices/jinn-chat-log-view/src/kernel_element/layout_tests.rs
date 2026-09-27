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
use std::sync::Arc;

use jinn_chat_log_view_msg::{ChatLogLayoutComputed, LayoutChatSession, MeasuredEntryCount};
use jinn_core_types::{ChatEntry, SessionId};
use jinn_session_state::ChatSessionState;

use crate::kernel_element::layout_complete::LayoutCompletionActorDeps;
use crate::kernel_element::layout_complete::{LayoutApplied, LayoutCompletionActor};
use crate::kernel_element::layout_supervisor::{LayoutSupervisorActor, LayoutSupervisorActorDeps};
use crate::kernel_element::layout_worker::{MeasureJob, measure, render_preview};
use jinn_kernel::common::app_state::AppState;
use jinn_kernel::common::state::State;

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
        entries: Arc::from(active.history().to_vec()),
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
fn a_shared_history_measures_the_same_counts_a_copied_one_does() {
    // Given a job whose history is shared rather than copied per worker.
    let (state, session_id) = state_with_entries(6, 50);
    let shared = job_for(&state, &session_id, 50);
    let inputs = inputs_for(&state, &session_id);

    // When the same history is measured as a plain owned vector.
    let mut owned = shared.clone();
    owned.entries = shared.entries.to_vec().into();
    let from_shared = measure(&MeasureJob::from(&shared), &inputs);
    let from_owned = measure(&MeasureJob::from(&owned), &inputs);

    // Then both paths agree on every entry's identity and count.
    let shared_rows: Vec<_> = from_shared
        .iter()
        .map(|c| (c.id.clone(), c.wrapped_count))
        .collect();
    let owned_rows: Vec<_> = from_owned
        .iter()
        .map(|c| (c.id.clone(), c.wrapped_count))
        .collect();
    assert_eq!(
        shared_rows, owned_rows,
        "sharing the history must not change what is measured"
    );
}

#[rstest::rstest]
fn cloning_a_layout_job_shares_the_history_instead_of_copying_it() {
    // Given a layout job carrying a shared history.
    let (state, session_id) = state_with_entries(4, 60);
    let job = job_for(&state, &session_id, 60);

    // When the job is cloned.
    let clone = job.clone();

    // Then both jobs point at one entry buffer.
    assert!(
        Arc::ptr_eq(&job.entries, &clone.entries),
        "a cloned job must share the history, not transcribe it"
    );
}

#[rstest::rstest]
fn a_measure_job_shares_the_history_it_was_built_from() {
    // Given a layout job, and the measure job handed to a blocking thread.
    let (state, session_id) = state_with_entries(4, 60);
    let msg = job_for(&state, &session_id, 60);

    // When the job is detached for measurement.
    let job = MeasureJob::from(&msg);

    // Then the measure job reads the same entry buffer the message carries.
    assert!(
        Arc::ptr_eq(&msg.entries, &job.entries),
        "handing work to a blocking thread must share the history, not copy it"
    );
}

#[rstest::rstest]
fn a_measure_job_carries_the_measurement_settings_the_message_had() {
    // Given a layout job at a known width and collapse threshold.
    let (state, session_id) = state_with_entries(4, 47);
    let mut msg = job_for(&state, &session_id, 47);
    msg.min_collapse_count = 9;
    msg.tool_entry_max_lines = 3;

    // When the job is detached for measurement.
    let job = MeasureJob::from(&msg);

    // Then the settings the measurement depends on survive the hand-off.
    assert_eq!(job.content_width, 47);
    // And the collapse threshold is carried across unchanged.
    assert_eq!(job.min_collapse_count, 9);
    // And so is the truncation depth.
    assert_eq!(job.tool_entry_max_lines, 3);
}

#[rstest::rstest]
fn a_layout_job_measures_every_entry() {
    // Given a session with several entries.
    let (state, session_id) = state_with_entries(5, 60);
    let job = job_for(&state, &session_id, 60);
    let inputs = inputs_for(&state, &session_id);

    // When the job is measured.
    let measured = measure(&MeasureJob::from(&job), &inputs);

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
    let measured = measure(&MeasureJob::from(&job), &inputs);

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
    let measured = measure(&MeasureJob::from(&job), &inputs);

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
    let measured = measure(&MeasureJob::from(&job), &inputs);
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
    let measured = measure(&MeasureJob::from(&job), &inputs);
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
fn a_result_for_a_session_that_has_never_rendered_is_used() {
    // Given a session that has just been loaded and has therefore never
    // painted a frame, so it records no width of its own.
    let (state, session_id) = state_with_entries(2, 60);
    {
        // Forget the width, as a session that has not painted a frame would be.
        state
            .write()
            .session
            .get_mut(&session_id)
            .expect("active session")
            .set_content_width(0);
    }
    assert_eq!(
        state
            .read()
            .session
            .get(&session_id)
            .unwrap()
            .content_width(),
        0,
        "fixture must start with a never-rendered session"
    );
    let job = job_for(&state, &session_id, 60);
    let inputs = inputs_for(&state, &session_id);
    let measured = measure(&MeasureJob::from(&job), &inputs);
    let actor = LayoutCompletionActor::spawnless(LayoutCompletionActorDeps {
        state: state.clone(),
    });

    // When a result measured at the width the chat log is about to render at
    // is applied.
    let applied = actor.apply(&computed(&session_id, 60, measured));

    // Then the counts are used rather than discarded as stale.
    assert_eq!(applied, LayoutApplied::Applied);
    // And the session's own width is still unset, so the next frame is the
    // one that records it.
    assert_eq!(
        state
            .read()
            .session
            .get(&session_id)
            .unwrap()
            .content_width(),
        0
    );
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
    let measured = measure(&MeasureJob::from(&job), &inputs);
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
    let measured = measure(&MeasureJob::from(&job), &inputs);
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
    measured: Vec<jinn_chat_log_view_msg::MeasuredLineCount>,
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
fn a_deadline_for_another_session_does_not_release_the_loading_guard() {
    // Given a session that is loading.
    let (state, session_id, supervisor) = loading_state_with_supervisor();
    // And a different session whose deadline is still ticking — the session
    // the user switched away from.
    let departed = SessionId::new();

    // When that other session's deadline expires.
    supervisor.release_guard(&departed, "layout deadline expired");

    // Then the loading session keeps its guard.
    assert!(
        state.read().session.is_loading(),
        "a stale deadline must not release the session that is loading now"
    );
    // And the guard still names the session that is actually loading.
    assert_eq!(
        state
            .read()
            .session
            .session_load_guard()
            .map(|g| &g.session_id),
        Some(&session_id)
    );
}

#[rstest::rstest]
fn a_deadline_for_another_session_does_not_end_the_loading_indicator() {
    // Given a session that is still loading.
    let (state, _session_id, supervisor) = loading_state_with_supervisor();

    // When a stale deadline for a different session expires.
    supervisor.release_guard(&SessionId::new(), "layout deadline expired");

    // Then the user is still behind the loading indicator for the session
    // that is genuinely being measured.
    assert!(
        state.read().session.is_loading(),
        "releasing on a stale id would drop the spinner and send the next \
         frame back to measuring the whole history inline"
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

// ---------------------------------------------------------------------------
// Preview rendering — the shared entry-to-lines arithmetic, narrowed
// ---------------------------------------------------------------------------

/// A preview render context at `content_width`, with every per-entry render
/// input pinned off — which is exactly how the sidebar's preview builds it.
fn preview_ctx(content_width: u16) -> crate::chat_log::RenderContext {
    crate::chat_log::RenderContext {
        content_width,
        is_selected: false,
        is_expanded: false,
        tool_entry_max_lines: 6,
        theme: jinn_theme::default_theme(),
        paired_status: None,
        is_streaming: false,
        is_waiting_on_subagent: false,
    }
}

/// A history of `count` user entries, each one line at a wide width.
fn preview_entries(count: usize) -> Vec<ChatEntry> {
    (0..count)
        .map(|index| ChatEntry::user(format!("entry {index}")))
        .collect()
}

/// The plain text of a rendered line, spans concatenated.
fn line_text(line: &ratatui::text::Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

/// The non-blank text of the rendered lines, in order.
///
/// [`entry_to_lines`] pads each entry with blank lines and each span out to the
/// content width, so both are stripped here: what these tests care about is
/// *which entries* contributed and in what order.
fn visible_text(lines: &[ratatui::text::Line<'_>]) -> Vec<String> {
    lines
        .iter()
        .map(line_text)
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .collect()
}

#[rstest::rstest]
fn render_preview_keeps_only_the_last_max_lines() {
    // Given a history whose last 5 entries render to more than 20 lines.
    let entries = preview_entries(50);

    // When rendering a preview that keeps at most 5 lines.
    let lines = render_preview(&entries, &preview_ctx(120), 5, 5);

    // Then the result is exactly the trailing 5 lines, in order.
    let rendered: Vec<String> = lines.iter().map(line_text).collect();
    assert_eq!(rendered.len(), 5, "expected 5 lines, got {rendered:?}");
    let visible = visible_text(&lines);
    assert_eq!(
        visible,
        vec!["entry 48", "entry 49"],
        "truncation must drop from the front so the newest text survives"
    );
}

#[rstest::rstest]
fn render_preview_reads_at_most_max_entries() {
    // Given a history of 50 entries.
    let entries = preview_entries(50);

    // When rendering a preview that reads at most 5 entries.
    let lines = render_preview(&entries, &preview_ctx(120), 5, 1000);

    // Then nothing from before the 5th-from-last entry contributed.
    assert_eq!(
        visible_text(&lines),
        vec!["entry 45", "entry 46", "entry 47", "entry 48", "entry 49"],
        "a preview is bounded to its trailing entries, not the whole history"
    );
}
