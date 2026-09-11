//! Kameo → trouper bridge: per-route forward relays.
//!
//! A slice declares its forward routes at activation
//! ([`SliceHost::forward`]); the drain wiring spawns one relay per
//! route (see [`kameo_to_trouper`]). There is no central route table:
//! routes live in the slices that own their messages, and a message
//! crosses only because its owning slice staged it.
//!
//! Topic names crossing messages are published onto live in this
//! module; the routes themselves are staged per-slice at activation.

pub mod kameo_to_trouper;
pub mod trouper_to_kameo;

pub use kameo_to_trouper::spawn_one;
pub use trouper_to_kameo::spawn_reverse_relay;

/// Drains the dashboard slice's staged forward routes into per-route
/// relays. Typed here because the relays are kameo actors (kernel
/// fabric); the staged descriptors come from the dashboard crate.
pub async fn drain_dashboard_routes(services: &Services) {
    use jinn_dashboard::bridge;
    use jinn_dashboard::fabric_events::{ActorShutdownCompleted, ActorStarted, ActorStarting};
    use jinn_dashboard::nav::DashboardNav;
    use jinn_slices::ServiceStatusUpdate;

    spawn_one::<ActorStarting>(
        services,
        &entry(
            bridge::fabric_topic(),
            <ActorStarting as trouper::schema::Schema>::schema_id(),
        ),
    )
    .await;
    spawn_one::<ActorStarted>(
        services,
        &entry(
            bridge::fabric_topic(),
            <ActorStarted as trouper::schema::Schema>::schema_id(),
        ),
    )
    .await;
    spawn_one::<ActorShutdownCompleted>(
        services,
        &entry(
            bridge::fabric_topic(),
            <ActorShutdownCompleted as trouper::schema::Schema>::schema_id(),
        ),
    )
    .await;
    spawn_one::<ServiceStatusUpdate>(
        services,
        &entry(
            bridge::fabric_topic(),
            <ServiceStatusUpdate as trouper::schema::Schema>::schema_id(),
        ),
    )
    .await;
    spawn_one::<DashboardNav>(
        services,
        &entry(
            bridge::dashboard_topic(),
            <DashboardNav as trouper::schema::Schema>::schema_id(),
        ),
    )
    .await;
}

/// Builds the route entry a dashboard drain spawns a relay from.
fn entry(
    topic: trouper::types::Topic,
    schema_id: trouper::types::SchemaId,
) -> jinn_slices::host::RouteEntry {
    jinn_slices::host::RouteEntry {
        schema_id,
        name: "dashboard",
        topic,
        direction: jinn_slices::host::Direction::Forward,
    }
}

use crate::Services;
use trouper::types::Topic;

/// Trouper topic names crossing messages publish onto.
///
/// Constants live with the bridge (both fabrics read them); the
/// staging call sites are the slices.
pub mod topics {
    /// Actor lifecycle + cross-actor status events (dashboard input).
    pub const FABRIC: &str = "jinn.fabric";
    /// Dashboard keyboard navigation.
    pub const DASHBOARD: &str = "jinn.dashboard";
    /// Quake bar submit commands.
    pub const QUAKE_BAR: &str = "jinn.quake-bar";
}

/// The fabric topic (`jinn.fabric`) as a [`Topic`].
#[must_use]
pub fn fabric_topic() -> Topic {
    Topic::new(topics::FABRIC)
}

/// The dashboard topic (`jinn.dashboard`) as a [`Topic`].
#[must_use]
pub fn dashboard_topic() -> Topic {
    Topic::new(topics::DASHBOARD)
}

/// The quake-bar topic (`jinn.quake-bar`) as a [`Topic`].
#[must_use]
pub fn quake_bar_topic() -> Topic {
    Topic::new(topics::QUAKE_BAR)
}
