//! Forward bridge relays — kameo bus → trouper topic, one relay actor
//! per route.
//!
//! A slice declares its forward routes at activation
//! ([`SliceHost::forward`]); composition drains them through
//! [`spawn_forward_relays`] after all slices have activated. There is
//! no central route table: each route is a small kameo actor
//! (`ForwardRelay<M>`) registered on the bus for exactly one message
//! type, republishing onto its topic as a schema-tagged JSON envelope.
//!
//! Delivery semantics: the bus's `BestEffort` strategy drops a publish
//! when the relay's mailbox is full, and a dropped lifecycle event is
//! never redelivered — a dashboard row would freeze mid-lifecycle. The
//! relay therefore spawns with an **unbounded** mailbox: crossing
//! events are accepted unconditionally at the bus hop and any loss
//! stays at the trouper hop, where the retained topic log re-offers
//! past cursors. Fire-and-forget overall: a warn log on unroutable
//! sends, no retry.

use kameo::mailbox;
use trouper::envelope::Event;
use trouper::topics::Topic;

use crate::Services;
use crate::common::bus::BusMessage;
use jinn_slices::host::ForwardMessage;
use jinn_slices::host::RouteEntry;

/// Builds a trouper [`Event`] from a crossing message.
///
/// Serialization cannot fail for these types (plain structs/enums), so a
/// failure degrades to a null payload rather than a panic in an actor
/// handler.
pub(crate) fn event_of<M>(msg: &M) -> Event
where
    M: trouper::schema::Schema + serde::Serialize,
{
    let payload = serde_json::to_value(msg).unwrap_or(serde_json::Value::Null);
    Event::new(M::schema_id(), payload)
}

/// One relay: registered on the bus for `M`, forwards each publish
/// onto `topic`.
struct ForwardRelay<M> {
    system: trouper::system::ActorSystem,
    topic: Topic,
    _marker: std::marker::PhantomData<fn() -> M>,
}

impl<M> kameo::prelude::Actor for ForwardRelay<M>
where
    M: BusMessage + ForwardMessage,
{
    type Args = (trouper::system::ActorSystem, Topic);
    type Error = kameo::error::Infallible;

    async fn on_start(
        args: Self::Args,
        _actor_ref: kameo::prelude::ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            system: args.0,
            topic: args.1,
            _marker: std::marker::PhantomData,
        })
    }
}

impl<M> kameo::prelude::Message<M> for ForwardRelay<M>
where
    M: BusMessage + ForwardMessage,
{
    type Reply = ();

    async fn handle(&mut self, msg: M, _ctx: &mut kameo::prelude::Context<Self, Self::Reply>) {
        let event = event_of(&msg);
        if let Err(_unroutable) = self
            .system
            .send(self.system.envelope_to_topic(event, self.topic.clone()))
            .await
        {
            tracing::warn!(
                schema = %M::schema_id().to_string(),
                "forward relay hit an unroutable trouper topic send"
            );
        }
    }
}

/// Spawns and registers one forward relay: the typed drain step.
///
/// The host stages schema ids (transport metadata); the drain needs
/// the concrete type, so slice drain modules call this per route they
/// staged, naming `M` explicitly.
///
/// # Panics
///
/// Panics when the staged entry's schema id is not `M`'s — draining a
/// route as the wrong type is a wiring bug.
pub async fn spawn_one<M>(services: &Services, route: &RouteEntry)
where
    M: BusMessage + ForwardMessage,
{
    assert_eq!(
        route.schema_id,
        M::schema_id(),
        "forward route staging mismatch: staged {} but draining {} — routes must drain as their declared type",
        route.schema_id,
        M::schema_id()
    );
    let relay = <ForwardRelay<M> as kameo::actor::Spawn>::spawn_with_mailbox(
        (services.trouper_system.clone(), route.topic.clone()),
        // Unbounded: a bounded mailbox would let the bus's BestEffort
        // try_send silently drop crossing events under a startup burst
        // (e.g. the ~84 lifecycle publishes during actor wiring), and
        // the dashboard would freeze those actors at `Starting` forever.
        mailbox::unbounded(),
    );
    relay.wait_for_startup().await;
    services.bus.subscribe::<M, ForwardRelay<M>>(&relay).await;
}
