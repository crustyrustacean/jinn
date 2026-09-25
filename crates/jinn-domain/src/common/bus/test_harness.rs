//! Test harness for bus-based actor tests.
//!
//! Provides a [`TestHarness`] over a fresh trouper `ActorSystem` and offers
//! convenience methods for spawning actors, recorders, and publishing
//! messages — eliminating boilerplate from individual test functions.
#![allow(
    clippy::expect_used,
    clippy::missing_panics_doc,
    reason = "test harness"
)]

use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;

use crate::common::bus::BusMessage;
use crate::common::services::bus_service::BusService;

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

/// A test fixture that manages the message fabric and provides convenience
/// methods for spawning actors and recorders in tests.
pub struct TestHarness {
    bus: BusService,
    system: trouper::system::ActorSystem,
}

impl TestHarness {
    /// Create a new harness with a fresh trouper `ActorSystem`.
    // API symmetry with other harness methods; async for future-proofing.
    pub async fn new() -> Self {
        async {}.await;
        let system = trouper::system::ActorSystem::new(trouper::system::SystemConfig::production());
        let bus = BusService::new_trouper(system.clone());
        Self { bus, system }
    }

    /// Create a new harness like [`Self::new`] (kept for call-site
    /// compatibility; the fabric is trouper-only now).
    pub async fn new_best_effort() -> Self {
        Self::new().await
    }

    /// The wrapped `BusService` — pass to actor deps.
    pub fn bus(&self) -> BusService {
        self.bus.clone()
    }

    /// The harness's trouper system — spawn ported actors against it.
    #[must_use]
    pub const fn system(&self) -> &trouper::system::ActorSystem {
        &self.system
    }

    /// Assembles a harness from pre-built parts (fabric tests that hand-
    /// construct the `BusService` to control its legs).
    #[must_use]
    pub fn from_parts(bus: BusService, system: trouper::system::ActorSystem) -> Self {
        Self { bus, system }
    }

    /// Publish a typed message on the fabric (the `BusService`'s legs).
    pub async fn publish<M>(&self, msg: M)
    where
        M: BusMessage
            + trouper::schema::Schema
            + serde::Serialize
            + Clone
            + Send
            + Sync
            + trouper::envelope::PayloadValue,
    {
        self.bus.publish(msg).await;
    }

    /// Spawn a [`Recorder`] for type `M`: a `harness.publish::<M>()`
    /// round-trips through `BusService` and the schema broadcast before
    /// the recorder sees the decoded message — tests validate the real
    /// delivery path.
    #[expect(
        clippy::unused_async,
        reason = "API symmetry with other async harness methods"
    )]
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "ServiceActor::start is async by trait contract"
    )]
    pub async fn spawn_recorder<M>(&self) -> Recorder<M>
    where
        M: BusMessage
            + trouper::schema::Schema
            + serde::Serialize
            + serde::de::DeserializeOwned
            + Clone
            + Send
            + Sync
            + trouper::envelope::PayloadValue,
    {
        let recorder = Recorder::<M>::default();
        self.spawn_trouper_recorder::<M>(&recorder);
        recorder
    }

    /// Subscribes a trouper tap actor for `M`'s schema traffic, appending
    /// decoded messages into the harness's shared [`Recorder`] buffer.
    ///
    /// Each tap gets a unique path (a process-wide counter) so parallel
    /// tests never share tap state; the buffer is `Arc`-shared between the
    /// tap and the returned handle.
    fn spawn_trouper_recorder<M>(&self, recorder: &Recorder<M>)
    where
        M: BusMessage
            + trouper::schema::Schema
            + serde::Serialize
            + serde::de::DeserializeOwned
            + Clone
            + Send
            + Sync
            + trouper::envelope::PayloadValue,
    {
        use trouper::actor::{ActorPath, MsgHandler, ServiceActor};

        static TAP_SEQ: AtomicU64 = AtomicU64::new(0);

        struct TroupeTap<M> {
            buffer: Arc<Mutex<Vec<M>>>,
            _msg: PhantomData<fn() -> M>,
        }

        impl<M> ServiceActor for TroupeTap<M>
        where
            M: BusMessage
                + trouper::schema::Schema
                + serde::Serialize
                + serde::de::DeserializeOwned,
        {
            #[expect(
                clippy::unused_async_trait_impl,
                reason = "async signature symmetry; body has no await"
            )]
            async fn start(
                _args: &trouper::json::Json,
            ) -> Result<Self, error_stack::Report<trouper::registry::RegistryError>> {
                // Never called: spawned via `spawn_service_builder` + `start_with`
                // (the typed buffer can't ride JSON args).
                Err(error_stack::IntoReport::into_report(
                    trouper::registry::RegistryError::InvalidSpec,
                )
                .attach("TroupeTap is spawned via start_with"))
            }
        }

        impl<M> MsgHandler<M> for TroupeTap<M>
        where
            M: BusMessage
                + trouper::schema::Schema
                + serde::Serialize
                + serde::de::DeserializeOwned
                + Clone
                + Sync,
        {
            async fn handle(&mut self, msg: &M, _ctx: &mut trouper::context::MsgCtx<'_>) {
                self.buffer.lock().push(msg.clone());
            }
        }

        let seq = TAP_SEQ.fetch_add(1, Ordering::SeqCst);
        let path = ActorPath::new(format!(
            "test.tap.{}.{}",
            M::schema_id().name().replace("::", "."),
            seq
        ));
        let buffer = recorder.buffer.clone();
        trouper::builder::spawn_service_builder::<TroupeTap<M>>(&self.system)
            .at(path.clone())
            .start_with(move || {
                let buffer = buffer.clone();
                Box::pin(async move {
                    Ok(TroupeTap {
                        buffer,
                        _msg: PhantomData,
                    })
                })
            })
            .handles::<M>()
            .mailbox(1024, trouper::inbox::OverloadPolicy::Block)
            .start();
    }

    /// Build a [`Services`] with the harness bus wired into a test instance.
    ///
    /// This creates a `Services::new_fake()` and replaces its bus and trouper
    /// system with the harness ones, so actors use the same bus AND the same
    /// fabric the test is publishing on.
    pub async fn services(&self) -> crate::Services {
        let mut services = crate::Services::new_fake().await;
        services.bus = self.bus.clone();
        services.trouper_system = self.system.clone();
        services
    }

    /// Build an [`ActorDeps`] with the harness bus wired into a test [`Services`].
    ///
    /// This creates a `Services::new()` and replaces its bus and trouper system
    /// with the harness ones, so actors use the same bus AND the same fabric
    /// the test is publishing on.
    pub async fn actor_deps(&self) -> crate::common::actor_deps::ActorDeps {
        let mut services = crate::Services::new_fake().await;
        services.bus = self.bus.clone();
        services.trouper_system = self.system.clone();
        crate::common::actor_deps::ActorDeps { services }
    }
}

// ---------------------------------------------------------------------------
// Recorder
// ---------------------------------------------------------------------------

/// A test recorder for messages of type `M`: a shared buffer a trouper
/// tap appends decoded deliveries into. Retrieve them with
/// [`Recorder::drain`] or the [`await_recorded`] helper.
pub struct Recorder<M> {
    buffer: Arc<Mutex<Vec<M>>>,
}

impl<M> Clone for Recorder<M> {
    fn clone(&self) -> Self {
        Self {
            buffer: self.buffer.clone(),
        }
    }
}

impl<M> Default for Recorder<M> {
    fn default() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl<M> Recorder<M> {
    /// Drains all collected messages.
    pub fn drain(&self) -> Vec<M>
    where
        M: Clone + Send + 'static,
    {
        std::mem::take(&mut *self.buffer.lock())
    }

    /// The number of messages collected so far (without draining).
    pub fn len(&self) -> usize {
        self.buffer.lock().len()
    }

    /// Returns `true` if nothing has been collected.
    pub fn is_empty(&self) -> bool {
        self.buffer.lock().is_empty()
    }
}

/// Poll a [`Recorder`] until it has collected at least `min_count` messages, or
/// the timeout expires. Returns whatever has been collected (may be fewer than
/// `min_count` on timeout — the test assertion will then fail with a clear message).
pub async fn await_recorded<M: Clone + Send + 'static>(
    recorder: &Recorder<M>,
    min_count: usize,
    timeout: Duration,
) -> Vec<M> {
    let deadline = tokio::time::Instant::now() + timeout;
    // `drain` empties the buffer, so every poll's messages must be
    // kept: a burst split across polls would otherwise be discarded piecemeal
    // below `min_count`. Accumulate until the minimum is met.
    let mut collected: Vec<M> = Vec::new();
    loop {
        collected.extend(recorder.drain());
        if collected.len() >= min_count {
            return collected;
        }
        if tokio::time::Instant::now() >= deadline {
            return collected;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
