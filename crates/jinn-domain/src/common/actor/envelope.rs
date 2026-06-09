//! Actor envelope wrapping all message types into a single channel.
//!
//! Every message an actor processes arrives inside an [`ActorEnvelope`] -
//! whether it originated as a bus event, a bus command, a direct typed message
//! from another actor, or a system lifecycle message.
//!
//! Note: [`SystemMessage::ApplicationShuttingDown`] is intercepted by the
//! actor run loop - actors never see it in their `handle()` method.
//! See [`Actor::on_shutdown`](crate::Actor::on_shutdown) for the shutdown hook.

/// System-level lifecycle messages delivered to every actor.
///
/// These messages bypass the event bus - the actor host sends them directly
/// to all actors regardless of subscriptions.
///
/// Note: `ApplicationShuttingDown` is intercepted by the run loop and never
/// reaches the actor's `handle()` method. The run loop calls
/// [`Actor::on_shutdown()`](crate::Actor::on_shutdown) instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemMessage {
    /// The application is shutting down.
    ApplicationShuttingDown,
}

/// Wrapper for all messages an actor can receive.
///
/// The type parameter `M` is the actor's direct message type (e.g.
/// `LlmPipeDirectMsg`). Each actor reads `ActorEnvelope<M>` from a single
/// kanal channel, giving it one unified match block for all incoming messages.
#[expect(
    clippy::large_enum_variant,
    reason = "boxing would cascade through all match arms"
)]
pub enum ActorEnvelope<M> {
    /// A bus event this actor subscribed to during activation.
    Event(crate::protocol::Event),
    /// A bus command this actor registered for during activation.
    Command(crate::protocol::Command),
    /// A direct typed message from another actor.
    Direct(M),
    /// A system lifecycle message (delivered to all actors, no subscription needed).
    System(SystemMessage),
}
