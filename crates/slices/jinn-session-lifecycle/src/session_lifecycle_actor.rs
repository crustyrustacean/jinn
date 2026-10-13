//! Session lifecycle actor — scripted setup, teardown, close, and cwd changes.
//!
//! The actor owns the lifecycle state machine and one cancellable
//! child-process handle per session: sessions are independent, so any
//! number of lifecycle commands may run at once, never two for the same
//! session. Persistence and archiving are published as commands so the
//! session-store actor remains the single owner of storage and session-map
//! removal.

mod handlers;

use std::collections::HashMap;

use jinn_core_types::SessionId;
use jinn_kernel::Services;
use jinn_kernel::common::actor_deps::BusPublish;
use jinn_kernel::common::services::BusService;
use jinn_kernel::common::state::State;
use jinn_session_lifecycle_msg::builtin::BuiltinRegistry;
use jinn_session_lifecycle_msg::{
    CancelLifecycleCommand, CloseSession, FinishSessionSetup, FinishSessionTeardown,
    RunSessionSetup, RunSessionTeardown, SetSessionCwd, TeardownSessionTree,
};
use trouper::actor::{ActorPath, MsgHandler, ServiceActor};
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;
use trouper::system::ActorSystem;

use crate::command_runner::LifecycleCancelHandle;

/// The session lifecycle actor's static trouper path.
pub const SESSION_LIFECYCLE_PATH: &str = "session-lifecycle";

/// The session lifecycle actor's mailbox capacity.
pub const SESSION_LIFECYCLE_MAILBOX_CAPACITY: usize = 1_024;

/// Dependencies for [`SessionLifecycleActor`].
#[derive(Clone)]
pub struct SessionLifecycleActorDeps {
    /// Shared application state.
    pub state: State,
    /// Application-wide services, including the bus and session store summaries.
    pub services: Services,
    /// Registry of compiled lifecycle handlers.
    pub builtin_registry: BuiltinRegistry,
    /// Shell captured at startup for scripted lifecycle commands (bash
    /// semantics; see `jinn_common::bash_shell`).
    pub shell: String,
}

/// Actor that owns session setup, teardown, close, cancellation, and cwd changes.
pub struct SessionLifecycleActor {
    state: State,
    services: Services,
    builtin_registry: BuiltinRegistry,
    shell: String,
    /// The in-flight lifecycle command per session, keyed by session id.
    ///
    /// One command per session at a time, any number of sessions at once.
    /// Every finish and every cancel removes exactly its own session's
    /// entry, so one session's completion can never release another's
    /// child and a cancel kills only the named session's command.
    lifecycle_children: HashMap<SessionId, LifecycleCancelHandle>,
}

impl BusPublish for SessionLifecycleActor {
    fn bus(&self) -> &BusService {
        &self.services.bus
    }
}

impl ServiceActor for SessionLifecycleActor {
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "trait contract: start is never called (spawn uses start_with)"
    )]
    async fn start(
        _args: &trouper::json::Json,
    ) -> Result<Self, error_stack::Report<RegistryError>> {
        Err(
            error_stack::IntoReport::into_report(RegistryError::InvalidSpec)
                .attach("SessionLifecycleActor is spawned via start_with"),
        )
    }
}

impl SessionLifecycleActor {
    /// Spawns the lifecycle actor and installs its eight typed subscriptions.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "port convention: spawn takes owned deps and clones into start_with"
    )]
    pub fn spawn(system: &ActorSystem, deps: SessionLifecycleActorDeps) -> ActorPath {
        let path = ActorPath::new(SESSION_LIFECYCLE_PATH);
        trouper::builder::spawn_service_builder::<Self>(system)
            .at(path.clone())
            .start_with({
                let deps = deps.clone();
                move || {
                    let deps = deps.clone();
                    Box::pin(async move {
                        Ok(Self {
                            state: deps.state,
                            services: deps.services,
                            builtin_registry: deps.builtin_registry,
                            shell: deps.shell,
                            lifecycle_children: HashMap::new(),
                        })
                    })
                }
            })
            .handles::<RunSessionSetup>()
            .handles::<RunSessionTeardown>()
            .handles::<FinishSessionSetup>()
            .handles::<FinishSessionTeardown>()
            .handles::<CancelLifecycleCommand>()
            .handles::<SetSessionCwd>()
            .handles::<CloseSession>()
            .handles::<TeardownSessionTree>()
            .mailbox(
                SESSION_LIFECYCLE_MAILBOX_CAPACITY,
                trouper::inbox::OverloadPolicy::Block,
            )
            .start();
        path
    }
}

macro_rules! forward_lifecycle_message {
    ($message:ty, $handler:ident) => {
        impl MsgHandler<$message> for SessionLifecycleActor {
            async fn handle(&mut self, msg: &$message, _ctx: &mut MsgCtx<'_>) {
                self.$handler(msg).await;
            }
        }
    };
}

forward_lifecycle_message!(RunSessionSetup, handle_run_session_setup);
forward_lifecycle_message!(RunSessionTeardown, handle_run_session_teardown);
forward_lifecycle_message!(FinishSessionSetup, handle_finish_session_setup);
forward_lifecycle_message!(FinishSessionTeardown, handle_finish_session_teardown);
impl MsgHandler<CancelLifecycleCommand> for SessionLifecycleActor {
    async fn handle(&mut self, msg: &CancelLifecycleCommand, _ctx: &mut MsgCtx<'_>) {
        self.handle_cancel_lifecycle_command(msg);
    }
}

forward_lifecycle_message!(SetSessionCwd, handle_set_session_cwd);
forward_lifecycle_message!(CloseSession, handle_close_session);
forward_lifecycle_message!(TeardownSessionTree, handle_teardown_session_tree);

impl SessionLifecycleActor {
    /// Whether a lifecycle command is already in flight for the session.
    pub(super) fn has_in_flight(&self, session_id: &SessionId) -> bool {
        self.lifecycle_children.contains_key(session_id)
    }

    /// Records a spawned command's cancel handle for its session.
    pub(super) fn insert_in_flight(
        &mut self,
        session_id: SessionId,
        handle: LifecycleCancelHandle,
    ) {
        self.lifecycle_children.insert(session_id, handle);
    }

    /// Removes and returns the session's cancel handle, if one is recorded.
    pub(super) fn take_in_flight(
        &mut self,
        session_id: &SessionId,
    ) -> Option<LifecycleCancelHandle> {
        self.lifecycle_children.remove(session_id)
    }
}
