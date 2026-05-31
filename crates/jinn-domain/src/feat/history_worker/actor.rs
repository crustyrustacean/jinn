//! Generic `HistoryWorkerActor` - wraps any [`HistoryWorker`] as a bus actor.
//!
//! Subscribes to [`HistoryAppended`] events. On each event:
//! 2. Acquires a brief read lock, clones `history.to_vec()`, drops lock
//! 3. Spawns a tokio task for `worker.evaluate(history_snapshot).await`
//! 4. If mutations are produced, emits [`SubmitHistoryMutations`]

use std::collections::HashSet;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::state::State;
use crate::feat::history_worker::worker_trait::HistoryWorker;
use crate::feat::session::protocol::history_appended::HistoryAppended;
use crate::feat::session::protocol::submit_history_mutations::SubmitHistoryMutations;
use crate::protocol::{Command, Event, SessionId};

/// A generic actor that wraps a [`HistoryWorker`] implementation.
///
/// Each worker instance runs as its own actor with its own tokio task.
/// Workers receive [`HistoryAppended`] events, evaluate their heuristic,
/// and submit mutations via the command bus.
pub struct HistoryWorkerActor<H: HistoryWorker> {
    worker: H,
    state: State,
    /// Sessions currently being evaluated - prevents concurrent compaction.
    in_flight: HashSet<SessionId>,
}

/// Dependencies for [`HistoryWorkerActor`].
pub struct HistoryWorkerActorDeps<H: HistoryWorker> {
    /// The worker heuristic implementation.
    pub worker: H,
    /// Shared application state (for reading session history).
    pub state: State,
}

impl<H: HistoryWorker + Clone> Actor for HistoryWorkerActor<H> {
    type Message = NoDirectMsg;
    type Deps = HistoryWorkerActorDeps<H>;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.set_description(deps.worker.name());
        ctx.subscribe_event::<HistoryAppended>();

        Self {
            worker: deps.worker,
            state: deps.state,
            in_flight: HashSet::new(),
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(Event::HistoryAppended(ref payload)) => {
                self.handle_history_appended(payload, ctx).await;
            }
            ActorEnvelope::Command(_)
            | ActorEnvelope::Event(_)
            | ActorEnvelope::System(_)
            | ActorEnvelope::Direct(_) => {}
        }
    }
}

impl<H: HistoryWorker> HistoryWorkerActor<H> {
    pub(crate) async fn handle_history_appended(
        &mut self,
        event: &HistoryAppended,
        ctx: &ActorContext,
    ) {
        tracing::info!(
            worker = self.worker.name(),
            session_id = %event.session_id,
            "HistoryAppended received"
        );

        // Skip if already evaluating this session.
        if self.in_flight.contains(&event.session_id) {
            tracing::info!(
                session_id = %event.session_id,
                "compaction already in flight, skipping"
            );
            return;
        }

        // Verify session exists.
        {
            let state = self.state.read();
            if state.session.get(&event.session_id).is_none() {
                tracing::info!("session not found, skipping");
                return;
            }
        }
        // Brief read lock → clone history → drop lock.
        let history_snapshot = {
            let state = self.state.read();
            let Some(session) = state.session.get(&event.session_id) else {
                return;
            };
            session.history().to_vec()
        };

        // Mark session as in-flight.
        self.in_flight.insert(event.session_id.clone());

        // Run heuristic evaluation outside any lock (async).
        let mutations = self.worker.evaluate(&event.session_id, history_snapshot).await;

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
