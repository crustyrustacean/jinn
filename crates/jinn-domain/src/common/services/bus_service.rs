//! Service wrapper for the kameo [`MessageBus`](kameo_actors::message_bus::MessageBus).

use std::fmt;

use kameo::actor::ActorRef;
use kameo_actors::message_bus::{MessageBus, Publish, Register};

/// Shared, cloneable wrapper around the message bus actor ref.
///
/// Injected into [`Services`](super::Services) during startup.
/// All bus operations go through this wrapper.
#[derive(Clone)]
pub struct BusService {
    bus: ActorRef<MessageBus>,
}

impl BusService {
    /// Creates a new bus service wrapping the given actor ref.
    #[must_use]
    pub fn new(bus: ActorRef<MessageBus>) -> Self {
        Self { bus }
    }

    /// Returns a reference to the underlying bus actor ref.
    #[must_use]
    pub fn actor_ref(&self) -> &ActorRef<MessageBus> {
        &self.bus
    }

    /// Registers a recipient to receive messages of type `M` on the bus.
    ///
    /// Callers typically construct the recipient via `actor_ref.recipient::<M>()`.
    /// The message type `M` is inferred from the recipient.
    ///
    /// ```ignore
    /// let recipient = actor_ref.recipient::<MyMessage>();
    /// args.bus.register(recipient).await;
    /// ```
    pub async fn register<M: Clone + Send + 'static>(
        &self,
        recipient: kameo::actor::Recipient<M>,
    ) {
        self.bus
            .tell(Register(recipient))
            .await
            .expect("bus register should succeed");
    }

    /// Publishes a typed message to all registered recipients on the bus.
    ///
    /// Fire-and-forget: logs a warning on delivery failure but does not propagate.
    pub async fn publish<M: Clone + Send + 'static>(&self, msg: M) {
        if let Err(e) = self.bus.tell(Publish(msg)).await {
            tracing::warn!(err = ?e, "bus publish failed");
        }
    }
}

impl fmt::Debug for BusService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BusService").finish_non_exhaustive()
    }
}
