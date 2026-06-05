//! Actor that fires plugin lifecycle hooks.
//!
//! Subscribes to [`AllActorsSpawned`] and [`SessionCreated`] events.
//! Fires `on_app_started` and `on_session_created` plugin hooks respectively
//! via [`PluginFire`](crate::feat::workflow::PluginFire).

use crate::common::actor::protocol::event::AllActorsSpawned;
use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::feat::session_lifecycle::protocol::event::SessionCreated;
use crate::protocol::{Command, Event};

/// Actor that bridges domain lifecycle events to plugin hooks.
///
/// Subscribes to:
/// - [`AllActorsSpawned`] → fires `on_app_started`
/// - [`SessionCreated`] → fires `on_session_created`
pub struct PluginLifecycleActor {
    /// Runtime services (for `plugins: PluginFireService`).
    services: Services,
    /// The session ID active at startup (for `on_app_started` ctx).
    startup_session_id: String,
}

/// Dependencies for [`PluginLifecycleActor`].
pub struct PluginLifecycleActorDeps {
    /// Runtime services.
    pub services: Services,
    /// The active session ID at startup.
    pub startup_session_id: String,
}

impl Actor for PluginLifecycleActor {
    type Message = NoDirectMsg;
    type Deps = PluginLifecycleActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<AllActorsSpawned>();
        ctx.subscribe_event::<SessionCreated>();
        ctx.set_description("Fires plugin lifecycle hooks (on_app_started, on_session_created)");

        Self {
            services: deps.services,
            startup_session_id: deps.startup_session_id,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<NoDirectMsg>, _ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(event) => {
                self.handle_event(event).await;
            }
            ActorEnvelope::Command(_) | ActorEnvelope::System(_) => {}
        }
    }
}

impl PluginLifecycleActor {
    async fn handle_event(&self, event: Event) {
        match event {
            Event::AllActorsSpawned(_) => {
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
            Event::SessionCreated(SessionCreated { session_id }) => {
                tracing::debug!(session_id = %session_id, "firing on_session_created plugin hook");
                let ctx = serde_json::json!({
                    "session_id": session_id.to_string(),
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
            _ => {}
        }
    }
}

// Unused but needed for type inference in some contexts.
#[allow(dead_code, reason = "Command handling placeholder")]
fn _command_type_hint(_: Command) {}
