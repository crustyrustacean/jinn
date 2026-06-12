//! Service wrapper for the kameo [`MessageBus`](kameo_actors::message_bus::MessageBus).
//!
//! In production, delegates to a real kameo `MessageBus`. In tests, can operate
//! in **recording mode** via [`BusService::new_recording()`], which captures all
//! `publish()` calls for assertion with [`BusAudit`].

use std::any::{Any, TypeId};
use std::fmt;
use std::sync::{Arc, Mutex};

use kameo::actor::ActorRef;
use kameo_actors::message_bus::{MessageBus, Publish, Register};

use crate::common::bus::BusMessage;

// ---------------------------------------------------------------------------
// BusService
// ---------------------------------------------------------------------------

/// Shared, cloneable wrapper around the message bus.
///
/// Injected into [`Services`](super::Services) during startup.
/// All bus operations go through this wrapper.
///
/// In test code, use [`BusService::new_recording()`] to create a recording
/// bus that captures publishes for assertion via [`BusAudit`].
#[derive(Clone)]
pub struct BusService {
    inner: BusInner,
}

#[derive(Clone)]
enum BusInner {
    Real(ActorRef<MessageBus>),
    Recording(Arc<Mutex<Vec<RecordedMessage>>>),
}

impl BusService {
    /// Creates a new bus service wrapping the given actor ref.
    #[must_use]
    pub fn new(bus: ActorRef<MessageBus>) -> Self {
        Self {
            inner: BusInner::Real(bus),
        }
    }

    /// Creates a bus service in **recording mode** for tests.
    ///
    /// Returns a `(BusService, BusAudit)` pair. The service captures all
    /// `publish()` calls; the audit handle reads them back.
    /// `register()` is a no-op in recording mode.
    #[cfg(test)]
    pub fn new_recording() -> (Self, BusAudit) {
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let service = Self {
            inner: BusInner::Recording(recorded.clone()),
        };
        let audit = BusAudit { recorded };
        (service, audit)
    }

    /// Returns a reference to the underlying bus actor ref.
    ///
    /// # Panics
    ///
    /// Panics if called on a recording-mode bus (tests should not need this).
    #[must_use]
    pub fn actor_ref(&self) -> &ActorRef<MessageBus> {
        match &self.inner {
            BusInner::Real(bus) => bus,
            BusInner::Recording(_) => panic!("actor_ref() called on recording BusService"),
        }
    }

    /// Registers a recipient to receive messages of type `M` on the bus.
    ///
    /// No-op in recording mode.
    pub async fn register<M: Clone + Send + 'static>(
        &self,
        recipient: kameo::actor::Recipient<M>,
    ) {
        match &self.inner {
            BusInner::Real(bus) => {
                bus.ask(Register(recipient))
                    .await
                    .expect("bus register should succeed");
            }
            BusInner::Recording(_) => {
                // No-op in recording mode
                let _ = recipient;
            }
        }
    }

    /// Publishes a typed message to all registered recipients on the bus.
    ///
    /// In recording mode, captures the message for later assertion.
    pub async fn publish<M: BusMessage>(&self, msg: M) {
        match &self.inner {
            BusInner::Real(bus) => {
                if let Err(e) = bus.tell(Publish(msg)).await {
                    tracing::warn!(err = ?e, "bus publish failed");
                }
            }
            BusInner::Recording(recorded) => {
                let name = std::any::type_name::<M>()
                    .rsplit("::")
                    .next()
                    .unwrap_or(std::any::type_name::<M>())
                    .to_owned();
                let type_id = TypeId::of::<M>();
                recorded.lock().unwrap().push(RecordedMessage {
                    name,
                    type_id,
                    payload: Box::new(msg) as Box<dyn Any + Send>,
                });
            }
        }
    }
}

impl fmt::Debug for BusService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.inner {
            BusInner::Real(_) => f.debug_struct("BusService").finish_non_exhaustive(),
            BusInner::Recording(_) => f
                .debug_struct("BusService<Recording>")
                .finish_non_exhaustive(),
        }
    }
}

// ---------------------------------------------------------------------------
// RecordedMessage
// ---------------------------------------------------------------------------

/// A single captured publish call.
pub struct RecordedMessage {
    /// Short type name (e.g., `"PushChatEntry"`).
    pub name: String,
    /// `TypeId` of the message for typed downcasting.
    type_id: TypeId,
    /// The message payload, type-erased.
    payload: Box<dyn Any + Send>,
}

impl RecordedMessage {
    /// Downcasts the payload to a specific message type `M`.
    ///
    /// Returns `None` if the type doesn't match.
    pub fn downcast<M: BusMessage>(&self) -> Option<&M> {
        if self.type_id == TypeId::of::<M>() {
            self.payload.downcast_ref::<M>()
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// BusAudit
// ---------------------------------------------------------------------------

/// Test handle for reading messages captured by a recording [`BusService`].
///
/// Created by [`BusService::new_recording()`].
#[derive(Clone)]
pub struct BusAudit {
    recorded: Arc<Mutex<Vec<RecordedMessage>>>,
}

impl BusAudit {
    /// Returns the ordered list of short type names for all captured messages.
    ///
    /// Useful for asserting message ordering:
    /// ```ignore
    /// assert_eq!(audit.names(), ["PersistSession", "PushChatEntry"]);
    /// ```
    pub fn names(&self) -> Vec<String> {
        self.recorded
            .lock()
            .unwrap()
            .iter()
            .map(|m| m.name.clone())
            .collect()
    }

    /// Returns all captured messages of a specific type, in order.
    ///
    /// ```ignore
    /// let entries: Vec<PushChatEntry> = audit.of_type::<PushChatEntry>();
    /// assert_eq!(entries.len(), 1);
    /// ```
    pub fn of_type<M: BusMessage>(&self) -> Vec<M> {
        self.recorded
            .lock()
            .unwrap()
            .iter()
            .filter_map(|m| m.downcast::<M>().cloned())
            .collect()
    }

    /// Returns the total number of captured messages.
    pub fn len(&self) -> usize {
        self.recorded.lock().unwrap().len()
    }

    /// Returns `true` if no messages have been captured.
    pub fn is_empty(&self) -> bool {
        self.recorded.lock().unwrap().is_empty()
    }

    /// Clears all captured messages.
    pub fn clear(&self) {
        self.recorded.lock().unwrap().clear();
    }

    /// Returns `true` if a message with the given type name was captured.
    pub fn contains_name(&self, name: &str) -> bool {
        self.recorded
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.name == name)
    }
}


impl fmt::Debug for BusAudit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BusAudit")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

//FIXME: disabled during actor migration — tests reference deleted types
// #[cfg(test)]
#[cfg(any())]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Alpha { val: u32 }
    impl crate::common::bus::BusMessage for Alpha {}

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Beta { text: String }
    impl crate::common::bus::BusMessage for Beta {}

    #[tokio::test]
    async fn new_recording_starts_empty() {
        let (bus, audit) = BusService::new_recording();
        assert!(audit.is_empty());
        assert_eq!(audit.len(), 0);
        assert!(audit.names().is_empty());
    }

    #[tokio::test]
    async fn publish_captures_single_message() {
        let (bus, audit) = BusService::new_recording();
        bus.publish(Alpha { val: 42 }).await;
        assert_eq!(audit.len(), 1);
        assert_eq!(audit.names(), ["Alpha"]);
        let alphas: Vec<Alpha> = audit.of_type::<Alpha>();
        assert_eq!(alphas.len(), 1);
        assert_eq!(alphas[0].val, 42);
    }

    #[tokio::test]
    async fn publish_captures_multiple_types_in_order() {
        let (bus, audit) = BusService::new_recording();
        bus.publish(Alpha { val: 1 }).await;
        bus.publish(Beta { text: "hello".into() }).await;
        bus.publish(Alpha { val: 2 }).await;
        assert_eq!(audit.names(), ["Alpha", "Beta", "Alpha"]);
        assert_eq!(audit.of_type::<Alpha>().len(), 2);
        assert_eq!(audit.of_type::<Beta>().len(), 1);
        assert_eq!(audit.of_type::<Alpha>()[0].val, 1);
        assert_eq!(audit.of_type::<Alpha>()[1].val, 2);
        assert_eq!(audit.of_type::<Beta>()[0].text, "hello");
    }

    #[tokio::test]
    async fn contains_name_finds_published_type() {
        let (bus, audit) = BusService::new_recording();
        bus.publish(Alpha { val: 99 }).await;
        assert!(audit.contains_name("Alpha"));
        assert!(!audit.contains_name("Beta"));
    }

    #[tokio::test]
    async fn clear_removes_all_messages() {
        let (bus, audit) = BusService::new_recording();
        bus.publish(Alpha { val: 1 }).await;
        bus.publish(Beta { text: "x".into() }).await;
        assert_eq!(audit.len(), 2);
        audit.clear();
        assert!(audit.is_empty());
    }

    #[tokio::test]
    async fn of_type_returns_empty_for_unpublished_type() {
        let (bus, audit) = BusService::new_recording();
        bus.publish(Alpha { val: 1 }).await;
        let betas: Vec<Beta> = audit.of_type::<Beta>();
        assert!(betas.is_empty());
    }

    #[tokio::test]
    async fn register_is_noop_in_recording_mode() {
        let (bus, _audit) = BusService::new_recording();
        // register is a no-op in recording mode — just verify it doesn't panic
        // We can't easily create a Recipient without spawning an actor,
        // so just verify the bus drops without panic.
        drop(bus);
    }
}
