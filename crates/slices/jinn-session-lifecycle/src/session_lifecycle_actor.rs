//! Session lifecycle actor — scripted setup, teardown, close, and cwd changes.
//!
//! The actor owns the lifecycle state machine and its cancellable child-process
//! handle. Persistence and archiving are published as commands so the
//! session-store actor remains the single owner of storage and session-map
//! removal.

mod handlers;

use jinn_domain::Services;
use jinn_domain::common::actor_deps::BusPublish;
use jinn_domain::common::services::BusService;
use jinn_domain::common::state::State;
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
    /// Shell captured at startup for scripted lifecycle commands.
    pub shell: String,
}

/// Actor that owns session setup, teardown, close, cancellation, and cwd changes.
pub struct SessionLifecycleActor {
    state: State,
    services: Services,
    builtin_registry: BuiltinRegistry,
    shell: String,
    lifecycle_child: Option<LifecycleCancelHandle>,
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
                            lifecycle_child: None,
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
