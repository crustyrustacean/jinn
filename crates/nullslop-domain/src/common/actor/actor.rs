//! The actor trait for building nullslop actors.
//!
//! Actor authors implement [`Actor`] with async `handle`, `on_shutdown`, and
//! `shutdown` methods. The host creates channels and `ActorRef`s first, then
//! injects them into [`ActorContext`] during activation. After activation, the
//! host spawns a tokio task running the actor's message loop.
//!
//! # Shutdown lifecycle
//!
//! When the application shuts down:
//! 1. The run loop intercepts `ApplicationShuttingDown`
//! 2. `on_shutdown()` is called — override for cleanup (flush to disk, etc.)
//! 3. The run loop auto-announces `ActorShutdownCompleted`
//! 4. Remaining channel messages are drained and discarded
//! 5. `shutdown()` is called after the loop exits

use std::future::Future;

use super::context::ActorContext;
use super::envelope::ActorEnvelope;

/// Trait for implementing a nullslop actor.
///
/// Actors are activated with a two-phase startup:
/// 1. The host creates `ActorRef` channels for all actors.
/// 2. Each actor's [`activate`](Actor::activate) is called with an [`ActorContext`]
///    pre-loaded with peer `ActorRef` handles.
///
/// After activation, the actor receives all messages — bus events, bus commands,
/// direct messages from other actors, and shutdown — through a single
/// [`ActorEnvelope`] in the [`handle`](Actor::handle) method.
pub trait Actor {
    /// The direct message type this actor accepts from other actors.
    type Message: Send + 'static;

    /// Activates the actor. Use `ctx` to subscribe to events/commands
    /// and extract peer `ActorRef` handles.
    ///
    /// This is an associated function (not a method) — it returns `Self`,
    /// constructing the actor during activation.
    fn activate(ctx: &mut ActorContext) -> Self;

    /// Handles an incoming message (event, command, direct, or system).
    fn handle(
        &mut self,
        msg: ActorEnvelope<Self::Message>,
        ctx: &ActorContext,
    ) -> impl Future<Output = ()> + Send;

    /// Called when the application is shutting down.
    ///
    /// The run loop calls this automatically when it receives
    /// `ApplicationShuttingDown`, before auto-announcing shutdown completion.
    /// Override for cleanup (flush to disk, cancel in-flight requests, etc.).
    ///
    /// After this returns, the run loop:
    /// 1. Calls `ctx.announce_shutdown_completed()` (auto-announce)
    /// 2. Drains and discards remaining channel messages
    /// 3. Breaks the loop
    /// 4. Calls `shutdown()`
    ///
    /// Default is no-op.
    fn on_shutdown(&mut self, _ctx: &ActorContext) -> impl Future<Output = ()> + Send {
        std::future::ready(())
    }

    /// Shuts down the actor. Called after the run loop exits.
    ///
    /// Default implementation is a no-op. Override for cleanup logic.
    fn shutdown(self) -> impl Future<Output = ()> + Send
    where
        Self: Sized,
    {
        let _ = self;
        std::future::ready(())
    }
}
