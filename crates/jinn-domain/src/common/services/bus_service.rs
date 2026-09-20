//! Service wrapper for the message fabric.
//!
//! Production publishes onto the **trouper actor system**: every message is
//! wrapped as a schema-tagged [`trouper::envelope::Event`] and sent onto a
//! topic — the `jinn.domain` topic for kernel-domain traffic, or a slice's
//! own topic when a route registers one. Actors subscribe their topic
//! directly.
//!
//! In tests, [`BusService`] can operate in **recording mode** via
//! [`BusService::new_recording()`], which captures all `publish()` calls
//! for assertion with [`BusAudit`].

use std::any::{Any, TypeId};
use std::fmt;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::common::bus::BusMessage;

/// The trouper topic kernel-domain messages publish onto.
///
/// One shared topic suffices during the fabric swap: trouper dispatches by
/// schema id at the typed adapter, so distinct messages never collide even
/// on one topic. Slices that already own a topic keep it — a route entry
/// overrides the default for its message.
pub const JINN_DOMAIN_TOPIC: &str = "jinn.domain";

/// The kernel-domain topic as a [`Topic`](trouper::topics::Topic).
#[must_use]
pub fn jinn_domain_topic() -> trouper::topics::Topic {
    trouper::topics::Topic::new(JINN_DOMAIN_TOPIC)
}

// ---------------------------------------------------------------------------
// BusService
// ---------------------------------------------------------------------------

/// Shared, cloneable wrapper around the message fabric.
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
    /// Trouper-native fabric: publishes go onto schema-routed trouper
    /// topics.
    Troupe {
        system: trouper::system::ActorSystem,
        routes: Arc<Mutex<Vec<RouteRule>>>,
    },
    #[cfg_attr(
        not(any(test, feature = "test-harness")),
        expect(
            dead_code,
            reason = "recording mode is test-only but lives in the shared bus type"
        )
    )]
    Recording(Arc<Mutex<Vec<RecordedMessage>>>),
}

/// One schema→topic routing rule.
struct RouteRule {
    schema_id: trouper::schema::SchemaId,
    topic: trouper::topics::Topic,
}

impl RouteRule {
    fn matches(&self, schema_id: &trouper::schema::SchemaId) -> bool {
        &self.schema_id == schema_id
    }
}

impl BusService {
    /// Creates a bus service backed by the trouper fabric.
    #[must_use]
    pub fn new_trouper(system: trouper::system::ActorSystem) -> Self {
        Self {
            inner: BusInner::Troupe {
                system,
                routes: Arc::new(Mutex::new(Vec::new())),
            },
        }
    }

    /// The trouper system this service publishes onto, when the fabric is
    /// trouper-backed. The seam the bridge drains closures against.
    ///
    /// # Panics
    ///
    /// Panics when called on a recording-mode bus (test-only): there is no
    /// system to return.
    #[expect(
        clippy::panic,
        reason = "invariant: recording variant is test-only; calling system_ref on it is programmer misuse"
    )]
    #[must_use]
    pub fn system_ref(&self) -> &trouper::system::ActorSystem {
        match &self.inner {
            BusInner::Troupe { system, .. } => system,
            BusInner::Recording(_) => {
                panic!("system_ref() called on a recording bus (test-only)")
            }
        }
    }

    /// Creates a bus service in **recording mode** for tests.
    ///
    /// Returns a `(BusService, BusAudit)` pair. The service captures all
    /// `publish()` calls; the audit handle reads them back.
    /// `register()` is a no-op in recording mode.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn new_recording() -> (Self, BusAudit) {
        let messages = Arc::new(Mutex::new(Vec::new()));
        let service = Self {
            inner: BusInner::Recording(messages.clone()),
        };
        let audit = BusAudit { messages };
        (service, audit)
    }

    /// Returns `true` if this bus is in recording mode (test-only).
    #[must_use]
    pub fn is_recording(&self) -> bool {
        matches!(&self.inner, BusInner::Recording(_))
    }

    /// Routes one message schema onto `topic`: every future publish of a
    /// message with this schema id lands on the topic instead of the
    /// default `jinn.domain` topic.
    ///
    /// Registered by slice drains for messages whose slice already owns a
    /// trouper topic (the message is a `ForwardMessage` there).
    pub fn route_topic<M: trouper::schema::Schema>(&self, topic: trouper::topics::Topic) {
        if let BusInner::Troupe { routes, .. } = &self.inner {
            routes.lock().push(RouteRule {
                schema_id: M::schema_id(),
                topic,
            });
        }
    }

    /// Subscribes a trouper actor path to `topic` for message type `M`.
    ///
    /// Runtime-spawned actors (task listeners, MCP servers) use this to
    /// join the fabric with the routed topic resolved from the schema —
    /// the same path composition's `system.subscribe` calls take, exposed
    /// through the bus so the topic constant lives in one place.
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "async signature symmetry; body has no await"
    )]
    #[expect(
        clippy::unused_async,
        reason = "async signature symmetry; body has no await"
    )]
    #[expect(
        clippy::expect_used,
        reason = "a failed topic subscription is a wiring bug that must abort spawn"
    )]
    /// # Panics
    ///
    /// Panics if the target path has no slot — a runtime-spawned actor
    /// must exist (spawned) before this call.
    pub async fn subscribe_topic<M: trouper::schema::Schema>(
        &self,
        path: &trouper::actor::ActorPath,
        topic: &trouper::topics::Topic,
    ) {
        if let BusInner::Troupe { system, .. } = &self.inner {
            system
                .subscribe(path, topic, None)
                .expect("runtime actor subscribes its topic");
        }
    }

    /// The topic a publish of `M` currently rides (the route resolution
    /// `publish` uses) — inspection for test harnesses.
    #[cfg(any(test, feature = "test-harness"))]
    #[must_use]
    pub fn routed_topic<M: trouper::schema::Schema>(&self) -> trouper::topics::Topic {
        match &self.inner {
            BusInner::Troupe { routes, .. } => Self::topic_for(routes, &M::schema_id()),
            _ => jinn_domain_topic(),
        }
    }

    /// Every topic routed for `schema_id` (all registrations, deduped,
    /// preserving registration order) — the fan-out a publish performs.
    fn topics_for(
        routes: &Mutex<Vec<RouteRule>>,
        schema_id: &trouper::schema::SchemaId,
    ) -> Vec<trouper::topics::Topic> {
        let routes = routes.lock();
        let mut topics: Vec<trouper::topics::Topic> = Vec::new();
        for rule in routes.iter().filter(|rule| rule.matches(schema_id)) {
            if !topics.contains(&rule.topic) {
                topics.push(rule.topic.clone());
            }
        }
        if topics.is_empty() {
            topics.push(jinn_domain_topic());
        }
        topics
    }

    /// The topic a publish of `M` currently rides: the last-registered
    /// route for its schema id, else the shared `jinn.domain` topic.
    /// Single-topic view for ask seams and test taps; `publish` itself
    /// fans out over [`Self::topics_for`].
    #[cfg_attr(
        not(any(test, feature = "test-harness")),
        expect(
            dead_code,
            reason = "single-topic view is consumed by the test-harness probe"
        )
    )]
    fn topic_for(
        routes: &Mutex<Vec<RouteRule>>,
        schema_id: &trouper::schema::SchemaId,
    ) -> trouper::topics::Topic {
        let routes = routes.lock();
        routes
            .iter()
            .rev()
            .find(|rule| rule.matches(schema_id))
            .map_or_else(jinn_domain_topic, |rule| rule.topic.clone())
    }

    /// Publishes a typed message onto the fabric.
    ///
    /// On the trouper fabric the message is wrapped as a schema-tagged
    /// event and sent onto every topic routed for its schema id (the
    /// relay era fanned one publish out to each registered route's
    /// topic; multicast preserves that). With no route the shared
    /// `jinn.domain` topic receives the event. In recording mode,
    /// captures the message for later assertion.
    pub async fn publish<M: BusMessage + trouper::schema::Schema + serde::Serialize>(
        &self,
        msg: M,
    ) {
        match &self.inner {
            BusInner::Troupe { system, routes } => {
                let name = message_name::<M>();
                let payload = serde_json::to_value(&msg).unwrap_or(serde_json::Value::Null);
                let event = trouper::envelope::Event::new(M::schema_id(), payload);
                for topic in Self::topics_for(routes, &M::schema_id()) {
                    tracing::debug!(message = name, topic = %topic, "trouper: {name} published");
                    let _ = system
                        .send(system.envelope_to_topic(event.clone(), topic))
                        .await;
                }
            }
            BusInner::Recording(recorded) => {
                let type_id = TypeId::of::<M>();
                recorded.lock().push(RecordedMessage {
                    name: message_name::<M>().to_owned(),
                    type_id,
                    payload: Box::new(msg) as Box<dyn Any + Send>,
                });
            }
        }
    }

    /// Publishes a pre-built schema-tagged event onto the fabric.
    ///
    /// The closure bridge's erased publish path: the message arrives as a
    /// schema id + JSON payload (the schema table supplies the routed
    /// topics), so the delivery matches [`Self::publish`] exactly — one
    /// copy per routed topic, the shared `jinn.domain` topic when
    /// unrouted. Not recorded — recording-mode tests publish typed
    /// messages directly.
    pub async fn publish_event(&self, event: trouper::envelope::Event) {
        if let BusInner::Troupe { system, routes } = &self.inner {
            for topic in Self::topics_for(routes, &event.schema) {
                tracing::debug!(schema = %event.schema, topic = %topic, "trouper: event published");
                let _ = system
                    .send(system.envelope_to_topic(event.clone(), topic))
                    .await;
            }
        }
    }
}

/// The short type name of a bus message (e.g. `"PushChatEntry"`).
fn message_name<M: BusMessage>() -> &'static str {
    std::any::type_name::<M>()
        .rsplit("::")
        .next()
        .unwrap_or(std::any::type_name::<M>())
}

/// Test-only probe of the fabric's routing decisions.
///
/// Asks the bus directly which topic a schema currently routes onto — the
/// same resolution `publish` performs — so fabric tests can assert
/// schema→topic routing without inspecting the trouper system.
#[cfg(any(test, feature = "test-harness"))]
pub struct RouteTestProbe {
    bus: BusService,
}

#[cfg(any(test, feature = "test-harness"))]
impl RouteTestProbe {
    /// Attaches a probe to the given bus.
    #[must_use]
    pub fn attach(bus: &BusService) -> Self {
        Self { bus: bus.clone() }
    }

    /// The topic a publish of `M` currently rides.
    #[must_use]
    pub fn topic_for<M: trouper::schema::Schema>(&self) -> Option<String> {
        match &self.bus.inner {
            BusInner::Troupe { routes, .. } => Some(
                BusService::topic_for(routes, &M::schema_id())
                    .as_str()
                    .to_owned(),
            ),
            _ => None,
        }
    }
}

impl fmt::Debug for BusService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.inner {
            BusInner::Troupe { .. } => f.debug_struct("BusService<Troupe>").finish_non_exhaustive(),
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
    pub type_id: TypeId,
    /// The message payload, type-erased.
    pub payload: Box<dyn Any + Send>,
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
    messages: Arc<Mutex<Vec<RecordedMessage>>>,
}

impl BusAudit {
    /// Returns the ordered list of short type names for all captured messages.
    ///
    /// Useful for asserting message ordering:
    /// ```ignore
    /// assert_eq!(audit.names(), ["PersistSession", "PushChatEntry"]);
    /// ```
    pub fn names(&self) -> Vec<String> {
        self.messages
            .lock()
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
        self.messages
            .lock()
            .iter()
            .filter_map(|m| m.downcast::<M>().cloned())
            .collect()
    }

    /// Returns the total number of captured messages.
    pub fn len(&self) -> usize {
        self.messages.lock().len()
    }

    /// Returns `true` if no messages have been captured.
    pub fn is_empty(&self) -> bool {
        self.messages.lock().is_empty()
    }

    /// Clears all captured messages.
    pub fn clear(&self) {
        self.messages.lock().clear();
    }

    /// Returns `true` if a message with the given type name was captured.
    pub fn contains_name(&self, name: &str) -> bool {
        self.messages.lock().iter().any(|m| m.name == name)
    }
}

impl fmt::Debug for BusAudit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BusAudit")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    struct Alpha {
        val: u32,
    }
    impl crate::common::bus::BusMessage for Alpha {}

    jinn_slices::crossing_schema!(Alpha, "Alpha",
    trouper::schema::SchemaKind::Event,
    description: "Bus test message alpha.",
    fields: ["val" => trouper::schema::FieldTy::Int]);

    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    struct Beta {
        text: String,
    }
    impl crate::common::bus::BusMessage for Beta {}

    jinn_slices::crossing_schema!(Beta, "Beta",
    trouper::schema::SchemaKind::Event,
    description: "Bus test message beta.",
    fields: ["text" => trouper::schema::FieldTy::Str]);

    #[rstest::rstest]
    #[tokio::test]
    async fn new_recording_starts_empty() {
        let (_bus, audit) = BusService::new_recording();
        assert!(audit.is_empty());
        assert_eq!(audit.len(), 0);
        assert!(audit.names().is_empty());
    }

    #[rstest::rstest]
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

    #[rstest::rstest]
    #[tokio::test]
    async fn publish_captures_multiple_types_in_order() {
        let (bus, audit) = BusService::new_recording();
        bus.publish(Alpha { val: 1 }).await;
        bus.publish(Beta {
            text: "hello".into(),
        })
        .await;
        bus.publish(Alpha { val: 2 }).await;
        assert_eq!(audit.names(), ["Alpha", "Beta", "Alpha"]);
        assert_eq!(audit.of_type::<Alpha>().len(), 2);
        assert_eq!(audit.of_type::<Beta>().len(), 1);
        assert_eq!(audit.of_type::<Alpha>()[0].val, 1);
        assert_eq!(audit.of_type::<Alpha>()[1].val, 2);
        assert_eq!(audit.of_type::<Beta>()[0].text, "hello");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn contains_name_finds_published_type() {
        let (bus, audit) = BusService::new_recording();
        bus.publish(Alpha { val: 99 }).await;
        assert!(audit.contains_name("Alpha"));
        assert!(!audit.contains_name("Beta"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn clear_removes_all_messages() {
        let (bus, audit) = BusService::new_recording();
        bus.publish(Alpha { val: 1 }).await;
        bus.publish(Beta { text: "x".into() }).await;
        assert_eq!(audit.len(), 2);
        audit.clear();
        assert!(audit.is_empty());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn of_type_returns_empty_for_unpublished_type() {
        let (bus, audit) = BusService::new_recording();
        bus.publish(Alpha { val: 1 }).await;
        let betas: Vec<Beta> = audit.of_type::<Beta>();
        assert!(betas.is_empty());
    }

    /// A `MakeWriter` capturing formatted log output for assertions.
    #[derive(Clone, Default)]
    struct CapturingWriter(Arc<std::sync::Mutex<Vec<u8>>>);

    impl CapturingWriter {
        fn contents(&self) -> String {
            String::from_utf8(self.0.lock().expect("poisoned").clone())
                .expect("captured output is utf-8")
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturingWriter {
        type Writer = CapturingSink;

        fn make_writer(&'a self) -> Self::Writer {
            CapturingSink(self.0.clone())
        }
    }

    struct CapturingSink(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for CapturingSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let mut sink = self.0.lock().map_err(|err| {
                std::io::Error::other(format!("capture buffer mutex poisoned: {err}"))
            })?;
            sink.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn publish_on_trouper_bus_logs_published_line() {
        use tracing_subscriber::Layer;
        use tracing_subscriber::layer::SubscriberExt;

        // Given a trouper-backed BusService and a subscriber capturing
        // debug events.
        let system = trouper::system::ActorSystem::new(trouper::system::SystemConfig::production());
        let bus = BusService::new_trouper(system);

        let capture = CapturingWriter::default();
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_writer(capture.clone())
                .with_ansi(false)
                .with_filter(tracing_subscriber::EnvFilter::new("jinn_domain=debug")),
        );
        let _guard = tracing::subscriber::set_default(subscriber);

        // When publishing a message.
        bus.publish(Alpha { val: 7 }).await;

        // Then a debug line names the type as published with its topic.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if capture.contents().contains("trouper: Alpha published") {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "expected 'trouper: Alpha published' in captured output, got: {}",
                capture.contents()
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}
