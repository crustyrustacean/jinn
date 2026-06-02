//! Generic `HistoryWorkerActor` - wraps any [`HistoryWorker`] as a bus actor.
//!
//! Subscribes to [`HistorySnapshotReady`] events. On each event:
//! 2. Receives shared `Arc<[ChatEntry]>` (O(1) clone)
//! 3. Spawns a tokio task for `worker.evaluate(history_snapshot).await`
//! 4. If mutations are produced, emits [`SubmitHistoryMutations`]
use std::collections::HashSet;
use std::sync::Arc;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::feat::history_worker::worker_trait::HistoryWorker;
use crate::feat::session::chat_entry::ChatEntry;
use crate::feat::session::protocol::history_snapshot_ready::HistorySnapshotReady;
use crate::feat::session::protocol::submit_history_mutations::SubmitHistoryMutations;
use crate::protocol::{Command, Event, SessionId};

/// A generic actor that wraps a [`HistoryWorker`] implementation.
///
/// Each worker instance runs as its own actor with its own tokio task.
/// Workers receive [`HistorySnapshotReady`] events, evaluate their heuristic,
/// and submit mutations via the command bus.
pub struct HistoryWorkerActor<H: HistoryWorker> {
    worker: H,
    /// Sessions currently being evaluated - prevents concurrent compaction.
    in_flight: HashSet<SessionId>,
}

/// Dependencies for [`HistoryWorkerActor`].
pub struct HistoryWorkerActorDeps<H: HistoryWorker> {
    /// The worker heuristic implementation.
    pub worker: H,
}

impl<H: HistoryWorker + Clone> Actor for HistoryWorkerActor<H> {
    type Message = NoDirectMsg;
    type Deps = HistoryWorkerActorDeps<H>;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.set_description(deps.worker.name());
        ctx.subscribe_event::<HistorySnapshotReady>();

        Self {
            worker: deps.worker,
            in_flight: HashSet::new(),
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(Event::HistorySnapshotReady(ref payload)) => {
                self.handle_snapshot_ready(payload, ctx).await;
            }
            ActorEnvelope::Command(_)
            | ActorEnvelope::Event(_)
            | ActorEnvelope::System(_)
            | ActorEnvelope::Direct(_) => {}
        }
    }
}

impl<H: HistoryWorker + Clone> HistoryWorkerActor<H> {
    pub(crate) async fn handle_snapshot_ready(
        &mut self,
        event: &HistorySnapshotReady,
        ctx: &ActorContext,
    ) {
        tracing::info!(
            worker = self.worker.name(),
            session_id = %event.session_id,
            "HistorySnapshotReady received"
        );

        // Skip if already evaluating this session.
        if self.in_flight.contains(&event.session_id) {
            tracing::info!(
                session_id = %event.session_id,
                "compaction already in flight, skipping"
            );
            return;
        }

        // O(1) Arc clone — shared with all other workers.
        let history_snapshot: Arc<[ChatEntry]> = event.history.clone();

        // Mark session as in-flight.
        self.in_flight.insert(event.session_id.clone());

        // Run heuristic evaluation outside any lock (async).
        let mutations = self
            .worker
            .evaluate(&event.session_id, history_snapshot)
            .await;

        // Clear in-flight marker.
        self.in_flight.remove(&event.session_id);

        if mutations.is_empty() {
            return;
        }

        tracing::debug!(
            worker = self.worker.name(),
            session_id = %event.session_id,
            count = mutations.len(),
            "history worker produced mutations"
        );

        // Submit mutations via command bus.
        let cmd = Command::SubmitHistoryMutations(SubmitHistoryMutations {
            session_id: event.session_id.clone(),
            mutations,
        });
        if let Err(e) = ctx.send_command(cmd) {
            tracing::warn!(
                worker = self.worker.name(),
                err = ?e,
                "failed to submit history mutations"
            );
        }
    }
}
