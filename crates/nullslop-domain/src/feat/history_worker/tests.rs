//! Integration tests for the history worker pipeline.
//!
//! Tests cover the full lifecycle: worker evaluates history → produces
//! mutations → actor submits them via command bus → session queues
//! and later applies them at safe drain points.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

use std::sync::Arc;

use crate::common::actor::actor::Actor;
use crate::common::actor::{ActorContext, RecordingSink};
use crate::common::app_state::AppState;
use crate::common::state::State;
use crate::feat::history_worker::actor::{HistoryWorkerActor, HistoryWorkerActorDeps};
use crate::feat::history_worker::worker_trait::HistoryWorker;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind, ContextOverride};
use crate::feat::session::history_mutation::HistoryMutation;
use crate::feat::session::protocol::history_appended::HistoryAppended;
use crate::protocol::{Command, SessionId};

// ── Test workers ───────────────────────────────────────────────────

/// A test worker that marks all User entries beyond the first 3 as excluded.
#[derive(Clone)]
struct TruncateOldUserEntries;

#[async_trait::async_trait]
impl HistoryWorker for TruncateOldUserEntries {
    fn name(&self) -> &'static str {
        "test-truncate-old-user"
    }

    async fn evaluate(&self, history: Vec<ChatEntry>) -> Vec<HistoryMutation> {
        let user_entries: Vec<_> = history
            .iter()
            .filter(|e| matches!(e.kind, ChatEntryKind::User { .. }))
            .collect();

        if user_entries.len() <= 3 {
            return vec![];
        }

        // Mark all but last 3 user entries as excluded.
        let to_exclude = user_entries.len() - 3;
        user_entries[..to_exclude]
            .iter()
            .map(|e| HistoryMutation::SetContextOverride {
                entry_id: e.id.clone(),
                value: ContextOverride::ForcedExclude,
            })
            .collect()
    }
}

/// A test worker that always produces an empty result.
#[derive(Clone)]
struct NoOpWorker;

#[async_trait::async_trait]
impl HistoryWorker for NoOpWorker {
    fn name(&self) -> &'static str {
        "test-noop"
    }

    async fn evaluate(&self, _history: Vec<ChatEntry>) -> Vec<HistoryMutation> {
        vec![]
    }
}

// ── Test helpers ───────────────────��───────────────────────────────

fn test_state_with_session(entries: Vec<ChatEntry>) -> (State, SessionId) {
    let mut session = crate::feat::session::chat_session::ChatSessionState::new();
    for entry in entries {
        session.push_entry(entry);
    }
    let id = session.session_id().clone();
    let state = State::new(AppState::default());
    // Replace the default session.
    {
        let mut app = state.write();
        app.session.insert(session);
    }
    (state, id)
}

fn make_actor<H: HistoryWorker + Clone>(worker: H, state: State) -> HistoryWorkerActor<H> {
    let sink = Arc::new(RecordingSink::new());
    let mut ctx = ActorContext::new("test-worker", sink);
    HistoryWorkerActor::activate(HistoryWorkerActorDeps { worker, state }, &mut ctx)
}

fn test_ctx() -> (Arc<RecordingSink>, ActorContext) {
    let sink = Arc::new(RecordingSink::new());
    let ctx = ActorContext::new("test-worker", sink.clone());
    (sink, ctx)
}

// ── Tests ──────────────────────────────────────────────────────────

#[test]
fn worker_produces_mutations_for_long_history() {
    let entries: Vec<ChatEntry> = (0..5)
        .map(|i| ChatEntry::user(format!("msg {i}")))
        .collect();
    let worker = TruncateOldUserEntries;
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let mutations = rt.block_on(async { worker.evaluate(entries).await });
    assert_eq!(mutations.len(), 2); // 5 entries - 3 kept = 2 excluded
    for m in &mutations {
        if let HistoryMutation::SetContextOverride { value, .. } = m {
            assert!(matches!(value, ContextOverride::ForcedExclude));
        } else {
            panic!("expected SetContextOverride mutation");
        }
    }
}

#[test]
fn worker_produces_no_mutations_for_short_history() {
    let entries: Vec<ChatEntry> = (0..3)
        .map(|i| ChatEntry::user(format!("msg {i}")))
        .collect();
    let worker = TruncateOldUserEntries;
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let mutations = rt.block_on(async { worker.evaluate(entries).await });
    assert!(mutations.is_empty());
}

#[tokio::test]
async fn actor_emits_submit_command_for_long_history() {
    let entries: Vec<ChatEntry> = (0..5)
        .map(|i| ChatEntry::user(format!("msg {i}")))
        .collect();
    let (state, session_id) = test_state_with_session(entries);
    let actor = make_actor(TruncateOldUserEntries, state);
    let (sink, ctx) = test_ctx();

    let event = HistoryAppended {
        session_id: session_id.clone(),
    };
    actor.handle_history_appended(&event, &ctx).await;

    let commands = sink.take_commands();
    assert_eq!(commands.len(), 1);
    match &commands[0] {
        Command::SubmitHistoryMutations(cmd) => {
            assert_eq!(cmd.session_id, session_id);
            assert_eq!(cmd.mutations.len(), 2);
        }
        other => panic!("expected SubmitHistoryMutations, got {other:?}"),
    }
}

#[tokio::test]
async fn actor_emits_nothing_for_short_history() {
    let entries: Vec<ChatEntry> = (0..2)
        .map(|i| ChatEntry::user(format!("msg {i}")))
        .collect();
    let (state, session_id) = test_state_with_session(entries);
    let actor = make_actor(TruncateOldUserEntries, state);
    let (sink, ctx) = test_ctx();

    let event = HistoryAppended { session_id };
    actor.handle_history_appended(&event, &ctx).await;

    let commands = sink.take_commands();
    assert!(commands.is_empty());
}

#[tokio::test]
async fn actor_skips_nonexistent_session() {
    let state = State::new(AppState::default());
    let actor = make_actor(TruncateOldUserEntries, state);
    let (sink, ctx) = test_ctx();

    let event = HistoryAppended {
        session_id: SessionId::new(),
    };
    actor.handle_history_appended(&event, &ctx).await;

    let commands = sink.take_commands();
    assert!(commands.is_empty());
}

#[tokio::test]
async fn actor_skips_judge_session() {
    // Given a session that is a judge session with enough entries to trigger mutations.
    let entries: Vec<ChatEntry> = (0..5)
        .map(|i| ChatEntry::user(format!("msg {i}")))
        .collect();
    let state = State::new(AppState::default());
    let session_id = {
        let mut session = crate::feat::session::chat_session::ChatSessionState::new();
        for entry in entries {
            session.push_entry(entry);
        }
        // Make it a judge session.
        session.set_judge(crate::feat::judge::JudgeMeta {
            origin_session: SessionId::new(),
            is_attached: true,
            judge_name: "test-judge".to_owned(),
            auto_reset: None,
        });
        let id = session.session_id().clone();
        let mut app = state.write();
        app.session.insert(session);
        id
    };
    let actor = make_actor(TruncateOldUserEntries, state);
    let (sink, ctx) = test_ctx();

    let event = HistoryAppended { session_id };
    actor.handle_history_appended(&event, &ctx).await;

    // Then no commands were emitted — judge session is skipped.
    let commands = sink.take_commands();
    assert!(commands.is_empty());
}

#[tokio::test]
async fn noop_worker_never_produces_mutations() {
    let entries: Vec<ChatEntry> = (0..10)
        .map(|i| ChatEntry::user(format!("msg {i}")))
        .collect();
    let (state, session_id) = test_state_with_session(entries);
    let actor = make_actor(NoOpWorker, state);
    let (sink, ctx) = test_ctx();

    let event = HistoryAppended { session_id };
    actor.handle_history_appended(&event, &ctx).await;

    let commands = sink.take_commands();
    assert!(commands.is_empty());
}
