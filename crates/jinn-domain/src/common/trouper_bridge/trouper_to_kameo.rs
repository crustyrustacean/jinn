//! Trouper → kameo reverse relays: one relay actor per route.
//!
//! A slice declares a reverse route at activation
//! ([`SliceHost::reverse`]) when a trouper-side event must reach
//! pre-port kameo consumers. There is no central table: each route is
//! a [`ServiceActor`] at path `trouper-to-kameo/<schema-name>`, whose
//! handler republishes onto the kameo bus. Topic subscription happens
//! at spawn — subscribe is the readiness point, so publishes after
//! [`spawn_reverse_relay`] returns cannot be missed. The relay's inbox
//! is deep with backpressure (see the spawn site) so a retained-topic
//! pump pass never strands events behind a full inbox.

use trouper::actor::ActorPath;
use trouper::actor::MsgHandler;
use trouper::actor::ServiceActor;
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;
use trouper::system::ActorSystem;

use crate::common::services::bus_service::BusService;
use jinn_slices::host::ReverseMessage;

/// One reverse relay for message `M` on one topic.
pub struct ReverseRelay<M> {
    bus: BusService,
    _marker: std::marker::PhantomData<fn() -> M>,
}

impl<M> ServiceActor for ReverseRelay<M>
where
    M: ReverseMessage + crate::common::bus::BusMessage,
{
    async fn start(_args: &serde_json::Value) -> Result<Self, error_stack::Report<RegistryError>> {
        Err(
            error_stack::IntoReport::into_report(RegistryError::InvalidSpec)
                .attach("ReverseRelay spawns via start_with; start requires the bus handle"),
        )
    }
}

impl<M> MsgHandler<M> for ReverseRelay<M>
where
    M: ReverseMessage + crate::common::bus::BusMessage,
{
    async fn handle(&mut self, msg: M, _ctx: &mut MsgCtx<'_>) {
        self.bus.publish(msg).await;
    }
}

/// Spawns the relay actor for `M` at `trouper-to-kameo/<name>` and
/// subscribes it to its schema-named topic. Live on return.
///
/// # Panics
///
/// Panics when the topic subscription fails — a broken trouper
/// registry must abort wiring, not run unrouted.
#[expect(
    clippy::expect_used,
    reason = "a failed reverse-route subscription is a wiring bug that must abort launch"
)]
pub fn spawn_reverse_relay<M>(system: &ActorSystem, bus: BusService) -> ActorPath
where
    M: ReverseMessage + crate::common::bus::BusMessage,
{
    let name = M::schema_def().name;
    let path = ActorPath::new(format!("trouper-to-kameo/{name}"));
    let spawned = trouper::builder::spawn_service_builder::<ReverseRelay<M>>(system)
        .at(path.clone())
        // Large-inbox + Block: the topic pump offers retained entries with
        // try_deliver, so a full (default-64) inbox would leave events
        // stranded in the topic log until the next pump pass. A deep inbox
        // with backpressure drains the startup burst in one pass; true
        // redelivery stays available via cursor reset.
        .mailbox(64 * 1024, trouper::inbox::OverloadPolicy::Block)
        .start_with(move || {
            Box::pin(async move {
                Ok(ReverseRelay::<M> {
                    bus,
                    _marker: std::marker::PhantomData,
                })
            })
        })
        .handles::<M>()
        .start();
    let topic = trouper::topics::Topic::new(name.as_str());
    system
        .subscribe(&spawned, &topic, None)
        .expect("reverse relay subscribes its own schema topic");
    spawned
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unwrap_used,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use trouper::actor::ActorPath;
    use trouper::actor::MsgHandler;
    use trouper::context::MsgCtx;
    use trouper::envelope::Event;
    use trouper::registry::RegistryError;
    use trouper::schema::Schema;

    use super::spawn_reverse_relay;
    use crate::common::bus::test_harness::{TestHarness, await_recorded};

    /// Test-only topic: a topic with no reverse route — the negative case.
    const UNROUTED_TOPIC: &str = "jinn.test.reverse-bridge-unrouted";

    fn unrouted_topic() -> trouper::topics::Topic {
        trouper::topics::Topic::new(UNROUTED_TOPIC)
    }

    /// Test-only probe message — the stand-in for the first real
    /// trouper-emitting slice's event.
    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    struct ProbeEvent {
        note: String,
    }

    impl crate::common::bus::BusMessage for ProbeEvent {}

    jinn_slices::crossing_schema!(ProbeEvent, "ProbeEvent", trouper::schema::SchemaKind::Event,
        description: "Test-only probe for the reverse bridge.",
        fields: ["note" => trouper::schema::FieldTy::Str]);

    fn probe_envelope() -> Event {
        Event::new(
            ProbeEvent::schema_id(),
            serde_json::json!({ "note": "hello" }),
        )
    }

    /// A ProbeEvent published on its schema-named topic arrives at a
    /// kameo subscriber, typed.
    #[rstest::rstest]
    #[tokio::test]
    async fn reverse_route_forwards_topic_message_to_kameo() {
        // Given a reverse relay routed for ProbeEvent and a kameo
        // recorder registered for it.
        let harness = TestHarness::new().await;
        let services = harness.services().await;
        spawn_reverse_relay::<ProbeEvent>(&services.trouper_system, services.bus.clone());
        let recorder = harness.spawn_recorder::<ProbeEvent>().await;

        // When publishing a ProbeEvent envelope on its topic
        // immediately after the spawn helper returns.
        services
            .trouper_system
            .send(
                services
                    .trouper_system
                    .envelope_to_topic(probe_envelope(), probe_topic()),
            )
            .await
            .expect("topic send resolves");

        // Then the kameo recorder receives exactly the typed message —
        // subscribe is the readiness point, so nothing is missed.
        let msgs = await_recorded(&recorder, 1, Duration::from_secs(2)).await;
        assert_eq!(
            msgs,
            vec![ProbeEvent {
                note: "hello".to_owned()
            }]
        );
    }

    /// A message published on a topic without a relay never reaches
    /// kameo.
    #[rstest::rstest]
    #[tokio::test]
    async fn reverse_relay_ignores_unregistered_topics() {
        // Given a reverse relay routed for ProbeEvent and a kameo
        // recorder registered for it.
        let harness = TestHarness::new().await;
        let services = harness.services().await;
        spawn_reverse_relay::<ProbeEvent>(&services.trouper_system, services.bus.clone());
        let recorder = harness.spawn_recorder::<ProbeEvent>().await;

        // When publishing a ProbeEvent envelope on an unrouted topic.
        services
            .trouper_system
            .send(
                services
                    .trouper_system
                    .envelope_to_topic(probe_envelope(), unrouted_topic()),
            )
            .await
            .expect("topic send resolves");

        // Then nothing reaches the kameo recorder within the wait window.
        let msgs = await_recorded(&recorder, 1, Duration::from_millis(400)).await;
        assert!(
            msgs.is_empty(),
            "unrouted topic leaked into kameo: {msgs:?}"
        );
    }

    /// The relay lives at its well-known path derived from the schema
    /// name.
    #[rstest::rstest]
    #[tokio::test]
    async fn reverse_relay_spawns_at_schema_derived_path() {
        // Given a trouper system and a bus.
        let services = crate::Services::new_fake().await;

        // When spawning the relay.
        let path =
            spawn_reverse_relay::<ProbeEvent>(&services.trouper_system, services.bus.clone());

        // Then the path is `trouper-to-kameo/<schema-name>`.
        assert_eq!(path.to_string(), "trouper-to-kameo/ProbeEvent");
    }

    fn probe_topic() -> trouper::topics::Topic {
        trouper::topics::Topic::new("ProbeEvent")
    }

    /// A probe trouper actor counting the envelopes it receives on a
    /// topic.
    struct TopicProbe {
        hits: Arc<AtomicUsize>,
    }

    impl trouper::actor::ServiceActor for TopicProbe {
        async fn start(
            _args: &serde_json::Value,
        ) -> Result<Self, error_stack::Report<RegistryError>> {
            Err(
                error_stack::IntoReport::into_report(RegistryError::InvalidSpec)
                    .attach("spawned via start_with; start is never called"),
            )
        }
    }

    impl MsgHandler<crate::common::actor::protocol::event::ActorStarted> for TopicProbe {
        async fn handle(
            &mut self,
            _msg: crate::common::actor::protocol::event::ActorStarted,
            _ctx: &mut MsgCtx<'_>,
        ) {
            self.hits.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Spawning a forward relay makes bus publishes of `M` land on its
    /// trouper topic.
    #[rstest::rstest]
    #[tokio::test]
    async fn forward_relay_translates_bus_publish_to_topic_envelope() {
        // Given a trouper system with a probe subscribed to the fabric
        // topic and a forward relay for ActorStarted draining into it.
        let services = crate::Services::new_fake().await;
        let hits = Arc::new(AtomicUsize::new(0));
        let system = services.trouper_system.clone();
        trouper::builder::spawn_service_builder::<TopicProbe>(&system)
            .at(ActorPath::new("topic-probe"))
            .start_with({
                let hits = hits.clone();
                move || Box::pin(async move { Ok(TopicProbe { hits }) })
            })
            .handles::<crate::common::actor::protocol::event::ActorStarted>()
            .start();
        system
            .subscribe(
                &ActorPath::new("topic-probe"),
                &crate::common::trouper_bridge::fabric_topic(),
                None,
            )
            .expect("probe subscribes to the fabric topic");
        super::super::spawn_one::<crate::common::actor::protocol::event::ActorStarted>(
            &services,
            &jinn_slices::host::RouteEntry {
                schema_id:
                    <crate::common::actor::protocol::event::ActorStarted as Schema>::schema_id(),
                name: "test",
                topic: crate::common::trouper_bridge::fabric_topic(),
                direction: jinn_slices::host::Direction::Forward,
            },
        )
        .await;

        // When publishing ActorStarted on the kameo bus.
        services
            .bus
            .publish(crate::common::actor::protocol::event::ActorStarted {
                name: "llm".to_owned(),
                description: None,
            })
            .await;

        // Then the topic probe receives exactly one envelope.
        for _ in 0..200 {
            if hits.load(Ordering::SeqCst) > 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("topic probe never received the forwarded envelope");
    }

    /// Dual-direction registration panics at staging time — the loop
    /// guard is the registry's insert, not a post-hoc debug assert.
    #[rstest::rstest]
    #[test]
    #[should_panic(expected = "already registered")]
    fn dual_direction_registration_panics() {
        // Given a registry with ProbeEvent staged forward.
        let mut registry = jinn_slices::host::host_routes::RouteRegistry::default();
        registry.forward::<ProbeEvent, _>(probe_topic(), ProbeEvent::schema_def);

        // When staging the same schema in reverse.
        registry.reverse::<ProbeEvent, _>(probe_topic(), ProbeEvent::schema_def);

        // Then registration panics — a feedback loop by construction.
    }

    /// The typed drain rejects a staged entry drained as the wrong
    /// message type.
    #[rstest::rstest]
    #[tokio::test]
    #[should_panic(expected = "forward route staging mismatch")]
    async fn drain_rejects_type_mismatch() {
        // Given a staged route declared for ProbeEvent.
        let services = crate::Services::new_fake().await;
        let staged = jinn_slices::host::RouteEntry {
            schema_id: ProbeEvent::schema_id(),
            name: "test",
            topic: probe_topic(),
            direction: jinn_slices::host::Direction::Forward,
        };

        // When draining it as ActorStarted.
        super::super::spawn_one::<crate::common::actor::protocol::event::ActorStarted>(
            &services, &staged,
        )
        .await;

        // Then the drain panics (attribute).
    }
}
