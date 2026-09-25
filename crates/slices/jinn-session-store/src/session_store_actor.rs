//! Session-store actor — persistence, loading, startup hydration, and archiving.
//!
//! This actor owns the store-facing half of session management. It mutates
//! session state only through the same narrow capabilities and tcaps as the
//! kernel actor, then releases the state lock before publishing or persisting.

mod handlers;

use jinn_boot_msg::EnvironmentLoaded;
use jinn_domain::Services;
use jinn_domain::common::actor_deps::BusPublish;
use jinn_domain::common::services::BusService;
use jinn_domain::common::state::State;
use jinn_domain::common::tcaps::frontend::FrontendCap;
use jinn_domain::common::tcaps::session::SessionCap;
use jinn_session_store_msg::{ArchiveSession, ArchiveSessionTree};
use jinn_session_store_msg::{LoadSessionPickerEntries, SessionForkRequested};
use jinn_session_store_msg::SessionLoadRequested;
use jinn_session_store_msg::PersistSession;
use trouper::actor::{ActorPath, MsgHandler, ServiceActor};
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;
use trouper::system::ActorSystem;

/// The session-store actor's static trouper path.
pub const SESSION_STORE_PATH: &str = "session-store";

/// The session-store actor's mailbox capacity.
///
/// Store operations can touch the full session snapshot and SQLite. Blocking
/// delivery prevents a large history from being dropped during a burst of
/// explicit persistence requests.
pub const SESSION_STORE_MAILBOX_CAPACITY: usize = 65_536;

/// Dependencies for [`SessionStoreActor`].
#[derive(Clone)]
pub struct SessionStoreActorDeps {
    /// Application-wide services containing the session store and bus.
    pub services: Services,
    /// Shared application state.
    pub state: State,
    /// Capability for session-map writes.
    pub session_cap: SessionCap,
    /// Capability for picker and frontend reconciliation writes.
    pub frontend_cap: FrontendCap,
}

/// Actor that owns session persistence, loading, hydration, and archiving.
pub struct SessionStoreActor {
    services: Services,
    state: State,
    session_cap: SessionCap,
    frontend_cap: FrontendCap,
}

impl BusPublish for SessionStoreActor {
    fn bus(&self) -> &BusService {
        &self.services.bus
    }
}

impl ServiceActor for SessionStoreActor {
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "trait contract: start is never called (spawn uses start_with)"
    )]
    async fn start(
        _args: &trouper::json::Json,
    ) -> Result<Self, error_stack::Report<RegistryError>> {
        Err(
            error_stack::IntoReport::into_report(RegistryError::InvalidSpec)
                .attach("SessionStoreActor is spawned via start_with"),
        )
    }
}

impl SessionStoreActor {
    /// Spawns the store actor and installs its seven typed subscriptions.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "port convention: spawn takes owned deps and clones into start_with"
    )]
    pub fn spawn(system: &ActorSystem, deps: SessionStoreActorDeps) -> ActorPath {
        let path = ActorPath::new(SESSION_STORE_PATH);
        trouper::builder::spawn_service_builder::<Self>(system)
            .at(path.clone())
            .start_with({
                let deps = deps.clone();
                move || {
                    let deps = deps.clone();
                    Box::pin(async move {
                        Ok(Self {
                            services: deps.services,
                            state: deps.state,
                            session_cap: deps.session_cap,
                            frontend_cap: deps.frontend_cap,
                        })
                    })
                }
            })
            .handles::<SessionLoadRequested>()
            .handles::<LoadSessionPickerEntries>()
            .handles::<SessionForkRequested>()
            .handles::<PersistSession>()
            .handles::<ArchiveSession>()
            .handles::<ArchiveSessionTree>()
            .handles::<EnvironmentLoaded>()
            .mailbox(
                SESSION_STORE_MAILBOX_CAPACITY,
                trouper::inbox::OverloadPolicy::Block,
            )
            .start();
        path
    }
}

impl MsgHandler<SessionLoadRequested> for SessionStoreActor {
    async fn handle(&mut self, msg: &SessionLoadRequested, _ctx: &mut MsgCtx<'_>) {
        self.on_load_requested(msg).await;
    }
}

impl MsgHandler<LoadSessionPickerEntries> for SessionStoreActor {
    async fn handle(&mut self, msg: &LoadSessionPickerEntries, _ctx: &mut MsgCtx<'_>) {
        self.handle_load_session_picker_entries(msg).await;
    }
}

impl MsgHandler<SessionForkRequested> for SessionStoreActor {
    async fn handle(&mut self, msg: &SessionForkRequested, _ctx: &mut MsgCtx<'_>) {
        self.on_session_fork_requested(msg).await;
    }
}

impl MsgHandler<PersistSession> for SessionStoreActor {
    async fn handle(&mut self, msg: &PersistSession, _ctx: &mut MsgCtx<'_>) {
        self.handle_persist_session(msg).await;
    }
}

impl MsgHandler<ArchiveSession> for SessionStoreActor {
    async fn handle(&mut self, msg: &ArchiveSession, _ctx: &mut MsgCtx<'_>) {
        self.handle_archive_session(msg).await;
    }
}

impl MsgHandler<ArchiveSessionTree> for SessionStoreActor {
    async fn handle(&mut self, msg: &ArchiveSessionTree, _ctx: &mut MsgCtx<'_>) {
        self.handle_archive_session_tree(msg).await;
    }
}

impl MsgHandler<EnvironmentLoaded> for SessionStoreActor {
    async fn handle(&mut self, msg: &EnvironmentLoaded, _ctx: &mut MsgCtx<'_>) {
        self.on_environment_loaded(&msg.config).await;
    }
}
