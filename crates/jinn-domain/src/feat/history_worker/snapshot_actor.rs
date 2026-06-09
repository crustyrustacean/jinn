//! Snapshot actor — the single point that clones history into an `Arc`.
//!
//! Subscribes to [`HistoryAppended`] events. On each event, acquires a brief
//! read lock on shared state, clones the session history into `Arc<[ChatEntry]>`,
//! and emits [`HistorySnapshotReady`]. All history workers subscribe to that
//! event instead of `HistoryAppended`, sharing a single allocation.

use std::sync::Arc;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::state::State;
use crate::feat::session::chat_entry::ChatEntry;
use crate::feat::session::protocol::history_appended::HistoryAppended;
use crate::feat::session::protocol::history_snapshot_ready::HistorySnapshotReady;
use crate::protocol::Event;

/// Actor that creates shared history snapshots for workers.
///
/// This is the **only** place that calls `session.history().to_vec()`.
/// All workers receive the resulting `Arc<[ChatEntry]>` via the
/// [`HistorySnapshotReady`] event.
pub struct HistorySnapshotActor {
    state: State,
}

/// Dependencies for [`HistorySnapshotActor`].
pub struct HistorySnapshotActorDeps {
    /// Shared application state (for reading session history).
    pub state: State,
}

impl Actor for HistorySnapshotActor {
    type Message = NoDirectMsg;
    type Deps = HistorySnapshotActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.set_description("history-snapshot");
        ctx.subscribe_event::<HistoryAppended>();

        Self { state: deps.state }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(Event::HistoryAppended(ref payload)) => {
                self.handle_history_appended(payload, ctx);
            }
            ActorEnvelope::Command(_)
            | ActorEnvelope::Event(_)
            | ActorEnvelope::System(_)
            | ActorEnvelope::Direct(_) => {}
        }
    }
}

impl HistorySnapshotActor {
    fn handle_history_appended(&self, event: &HistoryAppended, ctx: &ActorContext) {
        tracing::debug!(
            session_id = %event.session_id,
            "HistoryAppended received, creating snapshot"
        );

        // Brief read lock → clone history into Arc → drop lock.
        let history: Arc<[ChatEntry]> = {
            let state = self.state.read();
            let Some(session) = state.session.get(&event.session_id) else {
                tracing::debug!(
                    session_id = %event.session_id,
                    "session not found, skipping snapshot"
                );
                return;
            };
            Arc::from(session.history().to_vec())
        };

        tracing::debug!(
            session_id = %event.session_id,
            entries = history.len(),
            "history snapshot created"
        );

        let snapshot = HistorySnapshotReady {
            session_id: event.session_id.clone(),
            history,
        };

        if let Err(e) = ctx.send_event(Event::HistorySnapshotReady(snapshot)) {
            tracing::warn!(
                session_id = %event.session_id,
                err = ?e,
                "failed to send HistorySnapshotReady event"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unreachable, clippy::indexing_slicing, reason = "test code")]

    use std::sync::Arc;

    use crate::common::actor::{Actor, ActorContext, RecordingSink};
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::session::chat_entry::ChatEntry;
    use crate::feat::session::protocol::history_appended::HistoryAppended;
    use crate::protocol::{Event, SessionId};

    use super::{HistorySnapshotActor, HistorySnapshotActorDeps};

    fn test_state_with_session(entries: Vec<ChatEntry>) -> (State, SessionId) {
        let mut session = crate::feat::session::chat_session::ChatSessionState::new();
        for entry in entries {
            session.push_entry(entry);
        }
        let id = session.session_id().clone();
        let state = State::new(AppState::default());
        {
            let mut app = state.write();
            app.session.insert(session);
        }
        (state, id)
    }

    fn make_actor(state: State) -> HistorySnapshotActor {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("test-snapshot", sink);
        HistorySnapshotActor::activate(HistorySnapshotActorDeps { state }, &mut ctx)
    }

    fn test_ctx() -> (Arc<RecordingSink>, ActorContext) {
        let sink = Arc::new(RecordingSink::new());
        let ctx = ActorContext::new("test-snapshot", sink.clone());
        (sink, ctx)
    }

    #[test]
    fn snapshot_contains_all_entries() {
        let entries: Vec<ChatEntry> = (0..5)
            .map(|i| ChatEntry::user(format!("msg {i}")))
            .collect();
        let (state, session_id) = test_state_with_session(entries);
        let actor = make_actor(state);
        let (sink, ctx) = test_ctx();

        let event = HistoryAppended {
            session_id: session_id.clone(),
        };
        actor.handle_history_appended(&event, &ctx);

        let events = sink.take_events();
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::HistorySnapshotReady(snapshot) => {
                assert_eq!(snapshot.session_id, session_id);
                assert_eq!(snapshot.history.len(), 5);
            }
            other => panic!("expected HistorySnapshotReady, got {other:?}"),
        }
    }

    #[test]
    fn nonexistent_session_produces_no_event() {
        let state = State::new(AppState::default());
        let actor = make_actor(state);
        let (sink, ctx) = test_ctx();

        let event = HistoryAppended {
            session_id: SessionId::new(),
        };
        actor.handle_history_appended(&event, &ctx);

        let events = sink.take_events();
        assert!(events.is_empty());
    }

    #[test]
    fn empty_history_produces_empty_snapshot() {
        let (state, session_id) = test_state_with_session(vec![]);
        let actor = make_actor(state);
        let (sink, ctx) = test_ctx();

        let event = HistoryAppended {
            session_id: session_id.clone(),
        };
        actor.handle_history_appended(&event, &ctx);

        let events = sink.take_events();
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::HistorySnapshotReady(snapshot) => {
                assert_eq!(snapshot.session_id, session_id);
                assert_eq!(snapshot.history.len(), 0);
            }
            other => panic!("expected HistorySnapshotReady, got {other:?}"),
        }
    }
}
