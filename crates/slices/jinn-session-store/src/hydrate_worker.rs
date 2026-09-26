//! [`HydrateWorkerActor`] — reads a session's history off the store actor's mailbox.
//!
//! The store actor handles one message to completion, so a handler that awaits
//! a loop of database reads blocks every other message behind the whole
//! history. Startup hydration is exactly that loop: one read per unarchived
//! session, and another per missing tree member, each pulling every entry row
//! and blob.
//!
//! A pool of these workers does the reading instead. The startup handler sends
//! one job per session and returns; the mailbox is free the moment it does.
//! A worker reads its session and publishes the result back for the store actor
//! to apply.
//!
//! Three workers, matching the layout pool: reads, not CPU, are the
//! bottleneck, and the SQLite pool is four connections wide — more workers
//! would queue on the database rather than going faster.

use error_stack::Report;
use jinn_core_types::SessionId;
use jinn_domain::feat::session::SessionStoreService;
use jinn_session_state::SessionSnapshot;
use trouper::actor::{ActorPath, MsgHandler, ServiceActor};
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;

use crate::hydrate::{HydrateCompleted, HydrateSession};

/// Static pool size (one pool per process).
pub const HYDRATE_WORKER_POOL_SIZE: usize = 3;

/// The pool's path prefix; worker `n` spawns at `jinn.session_store.hydrate_worker.{n}`.
pub const HYDRATE_WORKER_PATH_PREFIX: &str = "jinn.session_store.hydrate_worker.";

/// The path the hydration worker at `index` spawns at.
#[must_use]
pub fn hydrate_worker_path(index: usize) -> ActorPath {
    ActorPath::new(format!("{HYDRATE_WORKER_PATH_PREFIX}{index}"))
}

/// Dependencies for [`HydrateWorkerActor`].
#[derive(Clone)]
pub struct HydrateWorkerActorDeps {
    /// The store the worker reads sessions from.
    pub session_store: SessionStoreService,
}

/// Reads one session's history and publishes it back.
pub struct HydrateWorkerActor {
    /// The store the worker reads sessions from.
    session_store: SessionStoreService,
}

impl ServiceActor for HydrateWorkerActor {
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "ServiceActor::start is async by trait contract"
    )]
    async fn start(_args: &trouper::json::Json) -> Result<Self, Report<RegistryError>> {
        // Never called: spawned via `spawn`'s start_with (typed deps can't
        // ride the JSON args).
        Err(Report::new(RegistryError::InvalidSpec)
            .attach("HydrateWorkerActor spawns via start_with"))
    }
}

impl HydrateWorkerActor {
    /// Spawns one worker of the hydration pool at `index`.
    ///
    /// Every worker declares the same work message, which is what lets
    /// `send_to_any` distribute a job across the pool.
    ///
    /// # Panics
    ///
    /// Panics if the actor's path is already taken — a wiring bug.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "port convention: spawn takes owned deps and clones into start_with"
    )]
    pub fn spawn(
        system: &trouper::system::ActorSystem,
        index: usize,
        deps: HydrateWorkerActorDeps,
    ) -> ActorPath {
        let path = hydrate_worker_path(index);
        trouper::builder::spawn_service_builder::<Self>(system)
            .at(path.clone())
            .start_with({
                let deps = deps.clone();
                move || {
                    let deps = deps.clone();
                    Box::pin(async move {
                        Ok(Self {
                            session_store: deps.session_store,
                        })
                    })
                }
            })
            .handles::<HydrateSession>()
            // The result leaves through ctx.publish; the flush gate drops any
            // outbound type not declared here, and a silently dropped completion
            // would leave the sidebar's hydration indicator up forever.
            .emits::<HydrateCompleted>()
            .mailbox(64, trouper::inbox::OverloadPolicy::Block)
            .start();
        path
    }

    /// Reads one session, collapsing every failure into `None`.
    ///
    /// A missing session and a failed read are the same outcome to the caller:
    /// no session to apply. Both are logged — at the level the caller used when
    /// this read happened inline, since a session missing from the store is
    /// expected for a tree member and worth noticing for a startup session —
    /// and reported as an absent snapshot, which leaves exactly one path to the
    /// completion the store actor waits on.
    async fn read_session(&self, session_id: &SessionId, frozen: bool) -> Option<SessionSnapshot> {
        match self.session_store.load_session(session_id).await {
            Ok(Some(snapshot)) => Some(snapshot),
            Ok(None) => {
                if frozen {
                    tracing::debug!(session_id = %session_id, "session in tree not found in store, skipping frozen node");
                } else {
                    tracing::warn!(session_id = %session_id, "session snapshot missing during hydration");
                }
                None
            }
            Err(error) => {
                tracing::warn!(
                    ?error,
                    session_id = %session_id,
                    "failed to load session snapshot during hydration"
                );
                None
            }
        }
    }
}

impl MsgHandler<HydrateSession> for HydrateWorkerActor {
    async fn handle(&mut self, msg: &HydrateSession, ctx: &mut MsgCtx<'_>) {
        let snapshot = self.read_session(&msg.session_id, msg.frozen).await;
        // Published on every path. The store actor counts completions to decide
        // when hydration has finished, so a return without publishing here
        // would strand that count and the hydration indicator with it.
        ctx.publish(HydrateCompleted {
            session_id: msg.session_id.clone(),
            frozen: msg.frozen,
            snapshot,
        });
    }
}
