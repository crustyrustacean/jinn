//! The dashboard slice's bridge drain: forward routes and topics.
//!
//! Topics are named here (the dashboard consumes them); the relays
//! themselves spawn in composition's drain wiring.

use trouper::topics::Topic;

use crate::contracts::ServiceStatusUpdate;
use crate::fabric_events::{ActorShutdownCompleted, ActorStarted, ActorStarting};
use crate::nav::DashboardNav;

/// Actor lifecycle + cross-actor status events (dashboard input).
pub const FABRIC: &str = "jinn.fabric";
/// Dashboard keyboard navigation.
pub const DASHBOARD: &str = "jinn.dashboard";

/// The fabric topic (`jinn.fabric`) as a [`Topic`].
#[must_use]
pub fn fabric_topic() -> Topic {
    Topic::new(FABRIC)
}

/// The dashboard topic (`jinn.dashboard`) as a [`Topic`].
#[must_use]
pub fn dashboard_topic() -> Topic {
    Topic::new(DASHBOARD)
}

/// All of the dashboard's staged forward routes, typed.
#[must_use]
pub fn typed_stages() -> Vec<RouteStagingDescriptor> {
    vec![
        RouteStagingDescriptor::of::<ActorStarting>(fabric_topic()),
        RouteStagingDescriptor::of::<ActorStarted>(fabric_topic()),
        RouteStagingDescriptor::of::<ActorShutdownCompleted>(fabric_topic()),
        RouteStagingDescriptor::of::<ServiceStatusUpdate>(fabric_topic()),
        RouteStagingDescriptor::of::<DashboardNav>(dashboard_topic()),
    ]
}

/// A type-erased staging descriptor carrying its schema id — the
/// manifest a composition drain walks.
pub struct RouteStagingDescriptor {
    /// The message's schema id.
    pub schema_id: trouper::schema::SchemaId,
    /// The trouper topic the relay publishes onto.
    pub topic: Topic,
    /// Display name for diagnostics.
    pub name: &'static str,
    /// Travel direction.
    pub direction: jinn_slices::Direction,
}

impl RouteStagingDescriptor {
    /// Stages one route for `M` on `topic`.
    #[must_use]
    pub fn of<M: trouper::schema::Schema>(topic: Topic) -> Self {
        Self {
            schema_id: M::schema_id(),
            topic,
            name: "dashboard",
            direction: jinn_slices::Direction::Forward,
        }
    }
}

/// Drains the dashboard slice's staged forward routes into per-route
/// routes. Typed here because the kernel
/// fabric); the typed descriptors above describe what to spawn.
///
/// Slice → kernel direction: this function is the dashboard's own
/// drain, called by composition after activation. It lives in this
/// crate (not the kernel's bridge module) because it names dashboard
/// types — the kernel must not depend on slice crates.
pub fn install_topic_routes(services: &jinn_domain::Services) {
    // Erased publishes (bridge closures) route natively on trouper: the
    // schema→topic rules mirror the relays below.
    services.bus.route_topic::<ActorStarting>(fabric_topic());
    services.bus.route_topic::<ActorStarted>(fabric_topic());
    services
        .bus
        .route_topic::<ActorShutdownCompleted>(fabric_topic());
    services
        .bus
        .route_topic::<ServiceStatusUpdate>(fabric_topic());
    services.bus.route_topic::<DashboardNav>(dashboard_topic());
}
