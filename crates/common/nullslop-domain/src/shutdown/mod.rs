//! Shutdown tracking — state and actor for shutdown coordination.
//!
//! Provides [`ShutdownTrackerState`] for bookkeeping which actors are still
//! running during a shutdown sequence, and [`ShutdownTrackerActor`] that
//! subscribes to lifecycle events and coordinates the shutdown sequence.

use std::collections::HashSet;
use std::sync::Arc;

use crate::actor::{Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink, SystemMessage};
use crate::actor_host::{ActorSpawnResult, spawn_actor};
use crate::component::State;
use crate::services::Services;
use nullslop_protocol::actor::{ActorShutdownCompleted, ActorStarting, ProceedWithShutdown};
use nullslop_protocol::{Command, Event};

/// Tracks which actors are still active during a shutdown.
#[derive(Debug, Clone, Default)]
pub struct ShutdownTrackerState {
    /// Actors that are currently running.
    pending: HashSet<String>,
    /// Whether the application has begun shutting down.
    shutdown_active: bool,
}

impl ShutdownTrackerState {
    /// Create a tracker with no actors and shutdown inactive.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Signal that the application has started shutting down.
    pub fn begin_shutdown(&mut self) {
        self.shutdown_active = true;
    }

    /// Record that an actor has started.
    pub fn track<S>(&mut self, name: S)
    where
        S: AsRef<str>,
    {
        self.pending.insert(name.as_ref().to_owned());
    }

    /// Record that an actor has finished shutting down.
    ///
    /// Returns `true` if this actor was known to be running.
    pub fn complete<S>(&mut self, name: S) -> bool
    where
        S: AsRef<str>,
    {
        self.pending.remove(name.as_ref())
    }

    /// Returns `true` when shutdown is in progress and every actor has finished.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.shutdown_active && self.pending.is_empty()
    }

    /// Returns the names of actors that are still running.
    #[must_use]
    pub fn pending_names(&self) -> Vec<String> {
        self.pending.iter().cloned().collect()
    }
}

/// Direct message type for the shutdown tracker actor (unused).
pub enum ShutdownTrackerDirectMsg {}

/// Spawns the shutdown tracker actor.
///
/// Creates the actor's channel, context, and run loop. Returns the
/// `ActorRef` for sending direct messages and the `ActorSpawnResult`
/// containing the routing entry and join handle.
pub fn spawn(
    state: State,
    services: Services,
    sink: Arc<dyn MessageSink>,
    handle: &tokio::runtime::Handle,
) -> (ActorRef<ShutdownTrackerDirectMsg>, ActorSpawnResult) {
    let (tx, rx) = kanal::unbounded::<ActorEnvelope<ShutdownTrackerDirectMsg>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new("shutdown-tracker", sink);
    ctx.set_description("Tracks actor lifecycle for shutdown coordination");
    ctx.set_data(state);
    ctx.set_data(services);
    let actor = ShutdownTrackerActor::activate(&mut ctx);
    let result = spawn_actor("shutdown-tracker", actor, &actor_ref, rx, ctx, handle);
    (actor_ref, result)
}

/// Actor that coordinates startup tracking and shutdown sequencing.
///
/// On `ActorStarting`, the actor name is added to the pending set.
/// On `ActorShutdownCompleted`, the name is removed. When all tracked actors
/// have completed and shutdown is active, a `ProceedWithShutdown` command
/// is emitted. When `ProceedWithShutdown` is received, `should_quit` is set
/// on `AppState`.
pub struct ShutdownTrackerActor {
    /// Shared application state.
    state: State,
    /// Optional services container for shutdown notifications.
    services: Option<Services>,
}

impl Actor for ShutdownTrackerActor {
    type Message = ShutdownTrackerDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<ActorStarting>();
        ctx.subscribe_event::<nullslop_protocol::actor::ActorStarted>();
        ctx.subscribe_event::<ActorShutdownCompleted>();
        ctx.subscribe_command::<ProceedWithShutdown>();

        #[expect(clippy::expect_used, reason = "State is always injected at startup")]
        let state = ctx.take_data::<State>().expect("State must be injected");

        // Services is optional — not all test contexts provide it.
        let services = ctx.take_data::<Services>();

        Self { state, services }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::System(SystemMessage::ApplicationReady) => {
                ctx.announce_started();
            }
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::Event(event) => {
                self.handle_event(&event, ctx);
            }
            ActorEnvelope::Command(command) => {
                self.handle_command(&command);
            }
            ActorEnvelope::Direct(_) | ActorEnvelope::Shutdown => {}
        }
    }

    async fn shutdown(self) {}
}

impl ShutdownTrackerActor {
    /// Dispatches incoming events to the appropriate handler.
    fn handle_event(&mut self, event: &Event, ctx: &ActorContext) {
        match event {
            Event::ActorStarting { payload } => {
                self.on_actor_starting(&payload.name);
            }
            Event::ActorShutdownCompleted { payload } => {
                self.on_actor_shutdown_completed(&payload.name, ctx);
            }
            _ => {}
        }
    }

    /// Dispatches incoming commands to the appropriate handler.
    fn handle_command(&mut self, command: &Command) {
        match command {
            Command::ProceedWithShutdown { .. } => {
                self.on_proceed_with_shutdown();
            }
            _ => {}
        }
    }

    /// Records that an actor is starting up.
    fn on_actor_starting(&mut self, name: &str) {
        self.state.write().shutdown.shutdown_tracker.track(name);
    }

    /// Records that an actor has completed shutdown.
    ///
    /// If all tracked actors have completed, emits `ProceedWithShutdown`
    /// with the full list of completed names and no timed-out names.
    fn on_actor_shutdown_completed(&mut self, name: &str, ctx: &ActorContext) {
        let should_emit = {
            let mut state = self.state.write();
            state.shutdown.shutdown_tracker.complete(name);
            state.shutdown.shutdown_tracker.is_complete()
        };

        if !should_emit {
            return;
        }

        // Lock-free emission
        if let Err(e) = ctx.send_command(Command::ProceedWithShutdown {
            payload: ProceedWithShutdown {
                completed: vec![],
                timed_out: vec![],
            },
        }) {
            tracing::error!(err = ?e, "failed to emit ProceedWithShutdown");
        }
    }

    /// Sets `should_quit` on `AppState` when `ProceedWithShutdown` is received.
    fn on_proceed_with_shutdown(&mut self) {
        self.state.write().frontend.should_quit = true;

        // Notify the core that shutdown is complete.
        if let Some(ref services) = self.services {
            services
                .core_channel
                .send(crate::services::CoreNotification::ShutdownComplete);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::component::AppState;

    use super::*;

    fn fresh_state() -> State {
        State::new(AppState::default())
    }

    #[rstest::rstest]
    fn actor_starting_tracks_actor() {
        // Given a shutdown tracker actor with fresh state.
        let state = fresh_state();
        let mut actor = ShutdownTrackerActor {
            state,
            services: None,
        };

        // When processing an ActorStarting event.
        actor.on_actor_starting("echo-actor");

        // Then the actor name is in the pending set.
        let pending = actor.state.read().shutdown.shutdown_tracker.pending_names();
        assert_eq!(pending, vec!["echo-actor"]);
    }

    #[rstest::rstest]
    fn actor_shutdown_completed_removes_actor() {
        // Given a tracker with one actor tracked and shutdown active.
        let state = fresh_state();
        state.write().shutdown.shutdown_tracker.track("echo-actor");
        state.write().shutdown.shutdown_tracker.begin_shutdown();
        let mut actor = ShutdownTrackerActor {
            state: state.clone(),
            services: None,
        };

        // When the actor completes shutdown.
        actor.on_actor_shutdown_completed("echo-actor", &create_noop_ctx());

        // Then shutdown is complete.
        assert!(actor.state.read().shutdown.shutdown_tracker.is_complete());
    }

    #[rstest::rstest]
    fn shutdown_not_complete_while_actors_pending() {
        // Given a tracker with two actors and shutdown active.
        let state = fresh_state();
        state.write().shutdown.shutdown_tracker.track("actor-a");
        state.write().shutdown.shutdown_tracker.track("actor-b");
        state.write().shutdown.shutdown_tracker.begin_shutdown();
        let mut actor = ShutdownTrackerActor {
            state: state.clone(),
            services: None,
        };

        // When only one actor completes shutdown.
        actor.on_actor_shutdown_completed("actor-a", &create_noop_ctx());

        // Then shutdown is NOT complete — actor-b is still pending.
        assert!(!actor.state.read().shutdown.shutdown_tracker.is_complete());
    }

    #[rstest::rstest]
    fn shutdown_complete_when_all_actors_done() {
        // Given a tracker with two actors and shutdown active.
        let state = fresh_state();
        state.write().shutdown.shutdown_tracker.track("actor-a");
        state.write().shutdown.shutdown_tracker.track("actor-b");
        state.write().shutdown.shutdown_tracker.begin_shutdown();
        let mut actor = ShutdownTrackerActor {
            state: state.clone(),
            services: None,
        };

        // When both actors complete shutdown.
        actor.on_actor_shutdown_completed("actor-a", &create_noop_ctx());
        actor.on_actor_shutdown_completed("actor-b", &create_noop_ctx());

        // Then shutdown is complete.
        assert!(actor.state.read().shutdown.shutdown_tracker.is_complete());
    }

    #[rstest::rstest]
    fn proceed_with_shutdown_sets_should_quit() {
        // Given a tracker with fresh state.
        let state = fresh_state();
        let mut actor = ShutdownTrackerActor {
            state,
            services: None,
        };

        // When receiving ProceedWithShutdown.
        actor.on_proceed_with_shutdown();

        // Then should_quit is set.
        assert!(actor.state.read().frontend.should_quit);
    }

    /// Create a no-op ActorContext for testing (no message sink).
    /// Tests that check emission behavior need a real sink.
    fn create_noop_ctx() -> ActorContext {
        use crate::actor::SendResult;
        struct NoopSink;
        impl crate::actor::MessageSink for NoopSink {
            fn send_command(&self, _command: nullslop_protocol::Command) -> SendResult {
                Ok(())
            }
            fn send_event(&self, _event: nullslop_protocol::Event) -> SendResult {
                Ok(())
            }
        }
        ActorContext::new("test", std::sync::Arc::new(NoopSink))
    }
}
