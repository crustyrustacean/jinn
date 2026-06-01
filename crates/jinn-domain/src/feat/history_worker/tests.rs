//! Integration tests for the history worker pipeline.
//!
//! Tests cover the full lifecycle: worker evaluates history snapshot → produces
//! mutations → actor submits them via command bus.

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

use std::sync::Arc;

use crate::common::actor::actor::Actor;
use crate::common::actor::{ActorContext, RecordingSink};
use crate::feat::history_worker::actor::{HistoryWorkerActor, HistoryWorkerActorDeps};
use crate::feat::history_worker::worker_trait::HistoryWorker;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind, ContextOverride};
use crate::feat::session::history_mutation::HistoryMutation;
use crate::feat::session::protocol::history_snapshot_ready::HistorySnapshotReady;
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

    async fn evaluate(
        &self,
        _session_id: &SessionId,
        history: Arc<[ChatEntry]>,
    ) -> Vec<HistoryMutation> {
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

    async fn evaluate(
        &self,
        _session_id: &SessionId,
        _history: Arc<[ChatEntry]>,
    ) -> Vec<HistoryMutation> {
        vec![]
    }
}

// ── Test helpers ───────────────────────────────────────────────────

fn make_actor<H: HistoryWorker + Clone>(worker: H) -> HistoryWorkerActor<H> {
    let sink = Arc::new(RecordingSink::new());
    let mut ctx = ActorContext::new("test-worker", sink);
    HistoryWorkerActor::activate(HistoryWorkerActorDeps { worker }, &mut ctx)
}

fn test_ctx() -> (Arc<RecordingSink>, ActorContext) {
    let sink = Arc::new(RecordingSink::new());
    let ctx = ActorContext::new("test-worker", sink.clone());
    (sink, ctx)
}

fn snapshot_event(session_id: SessionId, entries: Vec<ChatEntry>) -> HistorySnapshotReady {
    HistorySnapshotReady {
        session_id,
        history: Arc::from(entries),
    }
}

// ── Tests ──────────────────────────────────────────────────────────

#[test]
fn worker_produces_mutations_for_long_history() {
    let entries: Vec<ChatEntry> = (0..5)
        .map(|i| ChatEntry::user(format!("msg {i}")))
        .collect();
    let worker = TruncateOldUserEntries;
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let mutations =
        rt.block_on(async { worker.evaluate(&SessionId::new(), Arc::from(entries)).await }); // 5 entries - 3 kept = 2 excluded
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
    let mutations =
        rt.block_on(async { worker.evaluate(&SessionId::new(), Arc::from(entries)).await });
    assert!(mutations.is_empty());
}

#[tokio::test]
async fn actor_emits_submit_command_for_long_history() {
    let entries: Vec<ChatEntry> = (0..5)
        .map(|i| ChatEntry::user(format!("msg {i}")))
        .collect();
    let session_id = SessionId::new();
    let mut actor = make_actor(TruncateOldUserEntries);
    let (sink, ctx) = test_ctx();

    let event = snapshot_event(session_id.clone(), entries);
    actor.handle_snapshot_ready(&event, &ctx).await;

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
    let session_id = SessionId::new();
    let mut actor = make_actor(TruncateOldUserEntries);
    let (sink, ctx) = test_ctx();

    let event = snapshot_event(session_id, entries);
    actor.handle_snapshot_ready(&event, &ctx).await;

    let commands = sink.take_commands();
    assert!(commands.is_empty());
}

#[tokio::test]
async fn noop_worker_never_produces_mutations() {
    let entries: Vec<ChatEntry> = (0..10)
        .map(|i| ChatEntry::user(format!("msg {i}")))
        .collect();
    let session_id = SessionId::new();
    let mut actor = make_actor(NoOpWorker);
    let (sink, ctx) = test_ctx();

    let event = snapshot_event(session_id, entries);
    actor.handle_snapshot_ready(&event, &ctx).await;

    let commands = sink.take_commands();
    assert!(commands.is_empty());
}
