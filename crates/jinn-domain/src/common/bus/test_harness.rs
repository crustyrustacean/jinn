//! Test harness for bus-based actor tests.
//!
//! Provides a [`TestHarness`] that spawns a `MessageBus` and offers convenience
//! methods for spawning actors, recorders, and publishing messages — eliminating
//! boilerplate from individual test functions.

use std::marker::PhantomData;
use std::time::Duration;

use kameo::actor::{ActorRef, Spawn};
use kameo::prelude::{Context, Message};
use kameo_actors::message_bus::{Publish, Register};

use crate::common::services::bus_service::BusService;

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

/// A test fixture that manages a [`MessageBus`] and provides convenience methods
/// for spawning actors and recorders in tests.
pub struct TestHarness {
    bus: BusService,
    bus_ref: ActorRef<kameo_actors::message_bus::MessageBus>,
}

impl TestHarness {
    /// Create a new harness with a fresh `MessageBus`.
    pub async fn new() -> Self {
        let bus = kameo_actors::message_bus::MessageBus::new(kameo_actors::DeliveryStrategy::BestEffort);
        let bus_ref = Spawn::spawn(bus);
        let bus = BusService::new(bus_ref.clone());
        Self { bus, bus_ref }
    }

    /// The wrapped `BusService` — pass to actor deps.
    pub fn bus(&self) -> BusService {
        self.bus.clone()
    }

    /// Publish a typed message on the bus.
    pub async fn publish<M: Clone + Send + 'static>(&self, msg: M) {
        self.bus_ref
            .tell(Publish(msg))
            .await
            .expect("publish should succeed");
    }

    /// Spawn a kameo actor and wait for startup (bus registration complete).
    pub async fn spawn_actor<A: kameo::Actor>(&self, args: A::Args) -> ActorRef<A> {
        let actor = A::spawn(args);
        actor.wait_for_startup().await;
        actor
    }

    /// Spawn a [`Recorder`] and register it on the bus for type `M`.
    pub async fn spawn_recorder<M: Clone + Send + 'static>(&self) -> ActorRef<Recorder<M>> {
        let recorder = Recorder::<M>::spawn(());
        self.bus_ref
            .tell(Register(recorder.clone().recipient::<M>()))
            .await
            .expect("register recorder");
        recorder
    }
}

// ---------------------------------------------------------------------------
// await_recorded helper
// ---------------------------------------------------------------------------

/// Poll a [`Recorder`] until it has collected at least `min_count` messages, or
/// the timeout expires. Returns whatever has been collected (may be fewer than
/// `min_count` on timeout — the test assertion will then fail with a clear message).
pub async fn await_recorded<M: Clone + Send + 'static>(
    recorder: &ActorRef<Recorder<M>>,
    min_count: usize,
    timeout: Duration,
) -> Vec<M> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let recorded: Vec<M> = recorder.ask(GetRecorded::new()).await.expect("get recorded");
        if recorded.len() >= min_count {
            return recorded;
        }
        if tokio::time::Instant::now() >= deadline {
            return recorder.ask(GetRecorded::new()).await.expect("get recorded");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// ---------------------------------------------------------------------------
// Recorder actor
// ---------------------------------------------------------------------------

/// Query to retrieve collected messages from a [`Recorder`].
pub struct GetRecorded<M> {
    _phantom: PhantomData<M>,
}

impl<M> GetRecorded<M> {
    fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

/// A simple recorder actor that collects messages of type `M`.
/// Retrieve them with `recorder.ask(GetRecorded::<M>::new())`.
pub struct Recorder<M> {
    messages: Vec<M>,
}

impl<M: Send + 'static> kameo::Actor for Recorder<M> {
    type Args = ();
    type Error = kameo::error::Infallible;

    async fn on_start(
        _args: Self::Args,
        _actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            messages: Vec::new(),
        })
    }
}

impl<M: Clone + Send + 'static> Message<M> for Recorder<M> {
    type Reply = ();

    async fn handle(&mut self, msg: M, _ctx: &mut Context<Self, Self::Reply>) {
        self.messages.push(msg);
    }
}

impl<M: Clone + Send + 'static> Message<GetRecorded<M>> for Recorder<M> {
    type Reply = Vec<M>;

    async fn handle(
        &mut self,
        _msg: GetRecorded<M>,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.messages.clone()
    }
}
