//! Actor that fires plugin lifecycle hooks.
//!
//! Subscribes to [`AllActorsSpawned`] and [`SessionCreated`] events.
//! Fires `on_app_started` and `on_session_created` plugin hooks respectively
//! via [`PluginFire`](crate::feat::plugin_dispatch::PluginFire).

use std::convert::Infallible;

use kameo::prelude::{Actor, ActorRef, Context, Message};

use crate::common::actor::protocol::event::AllActorsSpawned;
use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::services::bus_service::BusService;
use crate::feat::session_lifecycle::protocol::event::SessionCreated;

/// Actor that bridges domain lifecycle events to plugin hooks.
///
/// Subscribes to:
/// - [`AllActorsSpawned`] → fires `on_app_started`
/// - [`SessionCreated`] → fires `on_session_created`
pub struct PluginLifecycleActor {
    /// Runtime services.
    services: crate::common::services::Services,
    /// The session ID active at startup (for `on_app_started` ctx).
    startup_session_id: String,
}

/// Dependencies for [`PluginLifecycleActor`].
pub struct PluginLifecycleActorDeps {
    /// Runtime deps (services + bus).
    pub deps: ActorDeps,
    /// The active session ID at startup.
    pub startup_session_id: String,
}

impl Actor for PluginLifecycleActor {
    type Args = PluginLifecycleActorDeps;
    type Error = Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        args.deps
            .subscribe(actor_ref.clone().recipient::<AllActorsSpawned>())
            .await;
        args.deps
            .subscribe(actor_ref.recipient::<SessionCreated>())
            .await;

        Ok(Self {
            services: args.deps.services,
            startup_session_id: args.startup_session_id,
        })
    }
}

impl BusPublish for PluginLifecycleActor {
    fn bus(&self) -> &BusService {
        &self.services.bus
    }
}

impl Message<AllActorsSpawned> for PluginLifecycleActor {
    type Reply = ();

    async fn handle(&mut self, _msg: AllActorsSpawned, _ctx: &mut Context<Self, Self::Reply>) {
        let ctx = serde_json::json!({
            "session_id": self.startup_session_id,
        });
        if let Err(e) = self
            .services
            .plugins
            .fire_async_json("on_app_started", &ctx)
            .await
        {
            tracing::warn!(err = %e, "on_app_started plugin hook failed");
        }
    }
}

impl Message<SessionCreated> for PluginLifecycleActor {
    type Reply = ();

    async fn handle(&mut self, msg: SessionCreated, _ctx: &mut Context<Self, Self::Reply>) {
        tracing::debug!(session_id = %msg.session_id, "firing on_session_created plugin hook");
        let ctx = serde_json::json!({
            "session_id": msg.session_id.to_string(),
        });
        if let Err(e) = self
            .services
            .plugins
            .fire_async_json("on_session_created", &ctx)
            .await
        {
            tracing::warn!(err = %e, "on_session_created plugin hook failed");
        }
    }
}
