//! Plugin actor - bridges domain events to Lua plugin VMs.
//!
//! The [`PluginActor`] subscribes to domain events (starting with
//! [`SessionCreated`]) and dispatches them to Lua plugin VMs via the
//! [`PluginRegistry`]. Because `mlua::Lua` and `PluginRegistry` are `!Send`,
//! the registry is created on and never leaves a dedicated OS thread.
//! The actor itself only holds a channel sender, forwarding events to that thread.

use jinn_domain::common::actor::protocol::event::AllActorsSpawned;
use jinn_domain::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use jinn_domain::feat::session_lifecycle::protocol::event::SessionCreated;
use jinn_domain::protocol::app_msg::Event;
use jinn_plugin::PluginRegistry;

/// Messages sent from the PluginActor (on the tokio runtime) to the dedicated
/// plugin thread (which owns the `!Send` `PluginRegistry`).
enum PluginMsg {
    /// A domain event to dispatch to plugin VMs.
    Event(Box<Event>),
    /// All actors spawned - fire `app::started`.
    AppStarted,
    /// Shut down the plugin thread.
    Shutdown,
}

/// Dependencies injected at activation.
pub struct PluginActorDeps {
    /// Factory that creates the plugin registry on the dedicated thread.
    /// This avoids moving the `!Send` `PluginRegistry` across thread boundaries.
    pub registry_factory: Box<dyn FnOnce() -> PluginRegistry + Send>,
    /// The active session ID at startup (for firing `app::started`).
    pub startup_session_id: String,
}

/// Actor that bridges domain events to Lua plugin VMs.
///
/// On activation, spawns a dedicated OS thread that creates and owns the
/// `PluginRegistry` (which contains `!Send` Lua VMs). The actor forwards
/// subscribed events through a channel to that thread.
pub struct PluginActor {
    /// Sender to the dedicated plugin thread.
    tx: kanal::Sender<PluginMsg>,
}

impl Actor for PluginActor {
    type Message = NoDirectMsg;
    type Deps = PluginActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<SessionCreated>();
        ctx.subscribe_event::<AllActorsSpawned>();

        let (tx, rx) = kanal::unbounded::<PluginMsg>();
        let factory = deps.registry_factory;

        // Spawn the dedicated OS thread that creates and owns the PluginRegistry.
        // The factory closure is `FnOnce` + `Send`, so it can cross the thread
        // boundary. The `!Send` `PluginRegistry` is constructed on this thread.
        let startup_session_id = deps.startup_session_id;
        std::thread::Builder::new()
            .name("plugin-dispatch".to_owned())
            .spawn(move || {
                let registry = factory();
                plugin_thread(rx, registry, startup_session_id);
            })
            .expect("failed to spawn plugin dispatch thread");

        Self { tx }
    }

    async fn handle(&mut self, msg: ActorEnvelope<NoDirectMsg>, _ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(event) => {
                Self::process_event(&event, &self.tx);
            }
            ActorEnvelope::Command(_) | ActorEnvelope::System(_) => {}
        }
    }

    async fn on_shutdown(&mut self, _ctx: &ActorContext) {
        let _ = self.tx.send(PluginMsg::Shutdown);
    }
}

impl PluginActor {
    /// Processes an incoming event, forwarding relevant ones to the plugin thread.
    fn process_event(event: &Event, tx: &kanal::Sender<PluginMsg>) {
        if let Event::SessionCreated(SessionCreated { session_id: _ }) = event {
            let _ = tx.send(PluginMsg::Event(Box::new(event.clone())));
        }
        // AllActorsSpawned triggers app::started on the plugin thread.
        if matches!(event, Event::AllActorsSpawned(_)) {
            let _ = tx.send(PluginMsg::AppStarted);
        }
    }
}

/// Runs on a dedicated OS thread. Owns the `PluginRegistry` and dispatches
/// events to Lua plugin VMs. Never crosses thread boundaries with `!Send` types.
fn plugin_thread(
    rx: kanal::Receiver<PluginMsg>,
    registry: PluginRegistry,
    startup_session_id: String,
) {
    loop {
        match rx.recv() {
            Ok(PluginMsg::Event(event)) => {
                dispatch_event(&event, &registry);
            }
            Ok(PluginMsg::AppStarted) => {
                let ctx = jinn_plugin::ctx::AppStartedCtx {
                    session_id: startup_session_id.clone(),
                };
                jinn_plugin::emit(jinn_plugin::hooks::APP_STARTED, &registry, &ctx);
            }
            Ok(PluginMsg::Shutdown) | Err(_) => {
                tracing::debug!("plugin dispatch thread shutting down");
                break;
            }
        }
    }
}

/// Maps a domain event to a plugin hook name and dispatches it.
fn dispatch_event(event: &Event, registry: &PluginRegistry) {
    if let Event::SessionCreated(SessionCreated { session_id }) = event {
        let ctx = jinn_plugin::ctx::SessionCreatedCtx {
            session_id: session_id.to_string(),
        };
        jinn_plugin::emit(jinn_plugin::hooks::SESSION_CREATED, registry, &ctx);
    }
}
