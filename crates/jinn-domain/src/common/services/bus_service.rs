//! Service wrapper for the kameo [`MessageBus`](kameo_actors::message_bus::MessageBus).

use std::fmt;

use kameo::prelude::ActorRef;
use kameo_actors::message_bus::MessageBus;

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
}

impl fmt::Debug for BusService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BusService").finish_non_exhaustive()
    }
}
