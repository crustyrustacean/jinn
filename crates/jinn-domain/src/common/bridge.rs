//! Kanal closure bridge — connects the sync TUI thread to the async kameo bus.
//!
//! The TUI's intent handler is synchronous and cannot `.await`. This bridge
//! accepts typed message closures through a kanal channel (sync send), then
//! an async drain task calls each closure with a reference to the
//! [`MessageBus`](kameo_actors::message_bus::MessageBus) actor ref.

use kameo::prelude::ActorRef;
use kameo_actors::message_bus::MessageBus;

use crate::common::actor::message_sink::MessageSink;
use crate::common::actor::actor_ref::{ActorSendError, SendResult};
use error_stack::Report;
use crate::protocol::{Command, Event};

/// A closure that publishes a typed message to the bus.
pub type BridgeClosure = Box<dyn FnOnce(&ActorRef<MessageBus>) + Send + 'static>;

/// Bridge between the sync TUI thread and the async kameo message bus.
///
/// The TUI sends [`BridgeClosure`]s via the sync [`kanal::Sender`].
/// A background async task drains them and calls each closure with the bus ref.
#[derive(Debug, Clone)]
pub struct Bridge {
    sender: kanal::Sender<BridgeClosure>,
}

impl Bridge {
    /// Creates a new bridge that drains closures to the given bus.
    ///
    /// Spawns a background tokio task that loops on `receiver.to_async().recv()`
    /// and calls each closure with the bus actor ref.
    pub fn new(bus: ActorRef<MessageBus>) -> Self {
        Self::with_handle(bus, tokio::runtime::Handle::current())
    }

    /// Creates a new bridge using a specific runtime handle.
    ///
    /// Use this when constructing from outside a tokio async context
    /// (e.g., from sync test code using a shared test runtime).
    pub fn with_handle(bus: ActorRef<MessageBus>, handle: tokio::runtime::Handle) -> Self {
        let (sender, receiver) = kanal::unbounded::<BridgeClosure>();
        let async_rx = receiver.to_async();

        handle.spawn(async move {
            while let Ok(closure) = async_rx.recv().await {
                closure(&bus);
            }
        });

        Self { sender }
    }

    /// Sends a closure through the bridge (synchronous, non-blocking).
    ///
    /// The closure will be called by the async drain task with a reference
    /// to the message bus.
    pub fn send(&self, msg: BridgeClosure) -> Result<(), kanal::SendError> {
        self.sender.send(msg)
    }

    /// Wraps a typed message into a bridge closure that publishes it to the bus.
    ///
    /// The returned closure captures the message and spawns a tokio task
    /// to call `bus.tell(Publish(msg)).await`.
    pub fn publish_closure<M>(msg: M) -> BridgeClosure
    where
        M: Clone + Send + 'static,
    {
        Box::new(move |bus| {
            let bus = bus.clone();
            tokio::spawn(async move {
                let _ = bus
                    .tell(kameo_actors::message_bus::Publish(msg))
                    .await;
            });
        })
    }
}

/// A [`MessageSink`] adapter that publishes commands/events via the [`Bridge`].
///
/// Used during migration to allow code that depends on `MessageSink` to
/// publish through the kameo bus without changes.
pub struct BridgeSink {
    bridge: Bridge,
}

impl BridgeSink {
    /// Creates a new sink wrapping the given bridge.
    #[must_use]
    pub fn new(bridge: Bridge) -> Self {
        Self { bridge }
    }
}

impl MessageSink for BridgeSink {
    fn name(&self) -> &'static str {
        "bridge_sink"
    }

    fn send_command(&self, command: Command) -> SendResult {
        let closure = Self::command_to_closure(command);
        self.bridge.send(closure).map_err(|_e| Report::new(ActorSendError))?;
        Ok(())
    }

    fn send_event(&self, event: Event) -> SendResult {
        let closure = Self::event_to_closure(event);
        self.bridge.send(closure).map_err(|_e| Report::new(ActorSendError))?;
        Ok(())
    }
}

impl BridgeSink {
    fn command_to_closure(
        command: crate::protocol::app_msg::command::Command,
    ) -> BridgeClosure {
        Box::new(move |bus| {
            let bus = bus.clone();
            tokio::spawn(async move {
                let _ = bus
                    .tell(kameo_actors::message_bus::Publish(command))
                    .await;
            });
        })
    }

    fn event_to_closure(
        event: crate::protocol::app_msg::event::Event,
    ) -> BridgeClosure {
        Box::new(move |bus| {
            let bus = bus.clone();
            tokio::spawn(async move {
                let _ = bus
                    .tell(kameo_actors::message_bus::Publish(event))
                    .await;
            });
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::*;
    use kameo::prelude::*;
    use kameo_actors::message_bus::{Publish, Register};
    use kameo_actors::DeliveryStrategy;
    use std::sync::{Arc, Mutex};

    #[derive(Actor)]
    struct RecorderActor<T: Send + 'static> {
        received: Arc<Mutex<Vec<T>>>,
    }

    impl<T: Send + 'static> RecorderActor<T> {
        fn new(buffer: Arc<Mutex<Vec<T>>>) -> Self {
            Self { received: buffer }
        }
    }

    impl<T: Clone + Send + 'static> Message<T> for RecorderActor<T> {
        type Reply = ();

        async fn handle(
            &mut self,
            msg: T,
            _ctx: &mut Context<Self, Self::Reply>,
        ) {
            self.received.lock().unwrap().push(msg);
        }
    }

    /// A simple message type for testing.
    #[derive(Clone, Debug, PartialEq)]
    struct TestMsg {
        value: u32,
    }

    impl crate::common::bus::BusMessage for TestMsg {}

    fn test_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn spawn_bus() -> ActorRef<MessageBus> {
        MessageBus::spawn(MessageBus::new(DeliveryStrategy::BestEffort))
    }

    fn spawn_recorder<T: Clone + Send + 'static>() -> (
        ActorRef<RecorderActor<T>>,
        Arc<Mutex<Vec<T>>>,
    ) {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let actor = RecorderActor::spawn(RecorderActor::new(buffer.clone()));
        (actor, buffer)
    }

    #[test]
    fn bus_delivers_published_message_to_registered_recipient() {
        let rt = test_runtime();
        rt.block_on(async {
            // Given a message bus and a registered actor.
            let bus = spawn_bus();
            let (actor, buffer) = spawn_recorder::<TestMsg>();
            bus.tell(Register(actor.recipient::<TestMsg>()))
                .await
                .unwrap();

            // When publishing a message.
            bus.tell(Publish(TestMsg { value: 42 }))
                .await
                .unwrap();

            // Then the registered actor receives it.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let received = buffer.lock().unwrap();
            assert_eq!(received.len(), 1);
            assert_eq!(received[0].value, 42);
        });
    }

    #[test]
    fn bridge_closure_publishes_to_bus_and_actor_receives() {
        let rt = test_runtime();
        rt.block_on(async {
            // Given a bus with a registered actor and a bridge.
            let bus = spawn_bus();
            let (actor, buffer) = spawn_recorder::<TestMsg>();
            bus.tell(Register(actor.recipient::<TestMsg>()))
                .await
                .unwrap();

            let bridge = Bridge::new(bus.clone());

            // When sending a closure through the bridge.
            let closure = Bridge::publish_closure(TestMsg { value: 99 });
            bridge.send(closure).unwrap();

            // Then the actor eventually receives the message.
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let received = buffer.lock().unwrap();
            assert_eq!(received.len(), 1);
            assert_eq!(received[0].value, 99);
        });
    }
}
