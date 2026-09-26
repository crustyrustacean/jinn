//! [`LayoutSupervisorActor`] — owns the layout worker pool and its deadline.
//!
//! This actor never sees the layout work itself. Workers are reached with a
//! typed `send_to_any`, which round-robins across them, so the supervisor only
//! has to answer the two ways a measurement can fail to finish: a worker
//! crashes and exhausts its restart budget, or the job simply takes longer than
//! anyone is willing to wait. Both clear the session load guard.
//!
//! The deadline is a recovery valve, not a performance target. Clearing the
//! guard on expiry means the next frame does the layout pass inline — the
//! freeze this whole feature exists to avoid, but briefly, instead of a
//! permanent spinner. The trade is deliberate: a slow session is a nuisance, a
//! stuck session is a dead application.

use std::sync::Arc;
use std::time::Duration;

use error_stack::Report;
use jinn_chat_log_view_msg::{
    ArmLayoutDeadline, ArmPreviewDeadline, Escalated, LayoutDeadlineExpired, PreviewDeadlineExpired,
};
use trouper::actor::{ActorPath, MsgHandler, ServiceActor};
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;
use trouper::supervision::{ActorSpec, Backoff, RestartBudget, RestartPolicy};

use crate::common::state::State;
use crate::feat::ui::chat_log::layout_worker::{
    LAYOUT_WORKER_POOL_SIZE, LayoutWorkerActor, LayoutWorkerActorDeps, layout_worker_path,
};

/// Static path the layout supervisor spawns at (one per process).
pub const LAYOUT_SUPERVISOR_PATH: &str = "jinn.chat_log.layout.supervisor";

/// How long a layout job may run before it is abandoned.
///
/// Long enough that an unusually large session is not cut off mid-measurement,
/// short enough that a genuinely stuck job recovers on its own.
pub const LAYOUT_DEADLINE: Duration = Duration::from_secs(30);

/// Restarts a crashed worker may take before the failure is reported.
const WORKER_RESTART_BUDGET: u32 = 2;

/// The window the restart budget is counted over.
const WORKER_RESTART_WINDOW: Duration = Duration::from_secs(10);

/// Dependencies for [`LayoutSupervisorActor`].
#[derive(Clone)]
pub struct LayoutSupervisorActorDeps {
    /// Shared application state, so the supervisor can clear the load guard.
    pub state: State,
    /// The system the worker pool is spawned on.
    pub system: trouper::system::ActorSystem,
}

/// Owns the layout worker pool and releases the load guard when a job fails.
pub struct LayoutSupervisorActor {
    /// Shared application state.
    state: State,
    /// The system the worker pool is spawned on.
    system: trouper::system::ActorSystem,
}

impl ServiceActor for LayoutSupervisorActor {
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "ServiceActor::start is async by trait contract"
    )]
    async fn start(_args: &trouper::json::Json) -> Result<Self, Report<RegistryError>> {
        // Never called: spawned via `spawn`'s start_with (typed deps can't
        // ride the JSON args).
        Err(Report::new(RegistryError::InvalidSpec)
            .attach("LayoutSupervisorActor spawns via start_with"))
    }
}

impl LayoutSupervisorActor {
    /// Spawns the supervisor and the worker pool it owns.
    ///
    /// The supervisor spawns first so it is already registered as each
    /// worker's escalation target when a worker starts.
    ///
    /// # Panics
    ///
    /// Panics if any path in the pool is already taken — a wiring bug.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "port convention: spawn takes owned deps and clones into start_with"
    )]
    pub fn spawn(
        system: &trouper::system::ActorSystem,
        deps: LayoutSupervisorActorDeps,
    ) -> ActorPath {
        let path = ActorPath::new(LAYOUT_SUPERVISOR_PATH);
        let worker_system = system.clone();
        // Bound before the builder so the returned path is the original, not
        // a clone the closure also captured.
        let bound_path = path.clone();
        trouper::builder::spawn_service_builder::<Self>(system)
            .at(path.clone())
            .start_with({
                let deps = deps.clone();
                move || {
                    let deps = deps.clone();
                    let path = bound_path.clone();
                    Box::pin(async move {
                        let state = deps.state.clone();
                        spawn_worker_pool(&worker_system, &state, &path);
                        Ok(Self {
                            state,
                            system: deps.system.clone(),
                        })
                    })
                }
            })
            .handles::<ArmLayoutDeadline>()
            .handles::<LayoutDeadlineExpired>()
            // The sidebar's preview renders share this pool, so they share its
            // deadline: one watchdog, not two.
            .handles::<ArmPreviewDeadline>()
            .handles::<PreviewDeadlineExpired>()
            .handles::<Escalated>()
            .mailbox(64, trouper::inbox::OverloadPolicy::Block)
            .start();
        path
    }

    /// Builds the supervisor directly, without an actor system.
    ///
    /// Releasing the guard is pure state work, so the deadline and escalation
    /// paths can be exercised without a fabric or a wall-clock wait.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn spawnless(deps: LayoutSupervisorActorDeps) -> Self {
        Self {
            state: deps.state,
            system: deps.system,
        }
    }

    /// Releases the load guard, leaving a warning behind.
    ///
    /// The chat log falls back to measuring inline, so nothing is written
    /// into the conversation: a measurement that could not be taken off-thread
    /// is not a conversation event.
    #[cfg(test)]
    pub(crate) fn release_guard(&self, session_id: &jinn_core_types::SessionId, reason: &str) {
        self.do_release_guard(session_id, reason);
    }

    /// Releases the load guard, leaving a warning behind.
    ///
    /// The chat log falls back to measuring inline, so nothing is written
    /// into the conversation: a measurement that could not be taken off-thread
    /// is not a conversation event.
    fn do_release_guard(&self, session_id: &jinn_core_types::SessionId, reason: &str) {
        let released = self
            .state
            .with_session(|view| view.session.map().clear_load_for(session_id));
        if !released {
            // The guard belongs to a session that is no longer loading — the
            // user switched away before this deadline or escalation landed.
            // Releasing it anyway would strand that other session's spinner
            // and make its next frame measure inline, so this is not an error.
            tracing::debug!(
                session_id = %session_id,
                reason,
                "layout release skipped; a different session holds the load guard"
            );
            return;
        }
        tracing::warn!(
            session_id = %session_id,
            reason,
            "chat log layout abandoned; clearing the load guard"
        );
    }

    /// Stops the sidebar's preview spinner, leaving a warning behind.
    ///
    /// Id-scoped like the guard above: a preview that timed out belongs to the
    /// session it was requested for, and abandoning a preview the cursor has
    /// since left would strand nothing but the log line.
    fn do_abandon_preview(
        &self,
        session_id: &jinn_core_types::SessionId,
        generation: u64,
        reason: &str,
    ) {
        // Generation-scoped, exactly as the guard release above is
        // session-scoped: a request the cursor has already moved past must not
        // stop the spinner belonging to the one that replaced it.
        let abandoned = self
            .state
            .read()
            .frontend
            .update_sections(|s| s.sessions.preview.abandon(session_id, generation));
        if abandoned.unwrap_or(false) {
            tracing::warn!(session_id = %session_id, reason, "session preview abandoned");
        }
    }
}

/// Spawns the layout worker pool, each worker supervised by `supervisor`.
///
/// The pool shares one supervisor rather than one per worker: a retired worker
/// only ever means "the measurement did not finish", and whichever session is
/// loading is released by the supervisor either way.
fn spawn_worker_pool(system: &trouper::system::ActorSystem, state: &State, supervisor: &ActorPath) {
    for index in 0..LAYOUT_WORKER_POOL_SIZE {
        let path = layout_worker_path(index);
        let state = state.clone();
        system.spawn(ActorSpec {
            path,
            parent: Some(supervisor.clone()),
            restart: RestartPolicy::Permanent,
            budget: RestartBudget::per(WORKER_RESTART_BUDGET, WORKER_RESTART_WINDOW),
            backoff: Backoff::default(),
            args: trouper::json!({ "index": index }),
            spawn: Arc::new(move |system: &trouper::system::ActorSystem, _path, _args| {
                LayoutWorkerActor::spawn(
                    system,
                    index,
                    LayoutWorkerActorDeps {
                        state: state.clone(),
                    },
                );
            }),
        });
    }
}

impl MsgHandler<ArmLayoutDeadline> for LayoutSupervisorActor {
    async fn handle(&mut self, msg: &ArmLayoutDeadline, _ctx: &mut MsgCtx<'_>) {
        // Owned copies, because the timer outlives this handler's borrow of
        // the message and of `self`.
        let system = self.system.clone();
        let session_id = msg.session_id.clone();
        let after = msg.after;
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            if let Err(_envelope) = system
                .send_to_any(LayoutDeadlineExpired { session_id })
                .await
            {
                tracing::warn!("no layout supervisor to receive the deadline expiry");
            }
        });
    }
}

impl MsgHandler<LayoutDeadlineExpired> for LayoutSupervisorActor {
    async fn handle(&mut self, msg: &LayoutDeadlineExpired, _ctx: &mut MsgCtx<'_>) {
        self.do_release_guard(&msg.session_id, "layout deadline expired");
    }
}

impl MsgHandler<ArmPreviewDeadline> for LayoutSupervisorActor {
    async fn handle(&mut self, msg: &ArmPreviewDeadline, _ctx: &mut MsgCtx<'_>) {
        // Owned copies, because the timer outlives this handler's borrow of
        // the message and of `self`.
        let system = self.system.clone();
        let session_id = msg.session_id.clone();
        let generation = msg.generation;
        let after = msg.after;
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            if let Err(_envelope) = system
                .send_to_any(PreviewDeadlineExpired {
                    session_id,
                    generation,
                })
                .await
            {
                tracing::warn!("no layout supervisor to receive the preview deadline expiry");
            }
        });
    }
}

impl MsgHandler<PreviewDeadlineExpired> for LayoutSupervisorActor {
    async fn handle(&mut self, msg: &PreviewDeadlineExpired, _ctx: &mut MsgCtx<'_>) {
        // A preview has no inline fallback the way a chat log measurement does:
        // there is nothing to render if the wrap never came back. Abandoning just
        // stops the spinner, so the popup shows nothing until the next cursor
        // move asks again.
        self.do_abandon_preview(&msg.session_id, msg.generation, "preview deadline expired");
    }
}

impl MsgHandler<Escalated> for LayoutSupervisorActor {
    async fn handle(&mut self, msg: &Escalated, _ctx: &mut MsgCtx<'_>) {
        // A failed worker carries no per-job context, so the guard is
        // released for whichever session is loading. A crash with no job in
        // flight is a no-op here.
        let session_id = self
            .state
            .read()
            .session
            .session_load_guard()
            .map(|guard| guard.session_id.clone());
        let Some(session_id) = session_id else {
            tracing::warn!(
                worker = %msg.escalated,
                reason = %msg.reason,
                "chat log layout worker retired with no session loading"
            );
            return;
        };
        self.do_release_guard(&session_id, "layout worker retired");
    }
}
