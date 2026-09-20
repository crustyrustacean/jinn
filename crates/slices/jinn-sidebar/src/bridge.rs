//! The sidebar slice's bridge drain: forward routes and topic.
//!
//! Topics are named here (the sidebar consumes them); the relays
//! themselves are kernel bridge actors spawned by this drain — the
//! slice-local drain convention (the slice owns the message types it
//! names, the kernel must not depend on slice crates).

use jinn_domain::Services;

use crate::sections::sidebar_state_actor::sidebar_topic;

/// Drains the sidebar slice's forward routes into bridge relays: the
/// kernel session events the slice's actors fold (kernel topic →
/// `jinn.sidebar`).
///
/// Slice-local drain, called by composition after activation. Relays
/// register on the bus in their own `on_start`, so drain ordering
/// relative to publishers is free.
pub fn install_topic_routes(services: &Services) {
    services
        .bus
        .route_topic::<jinn_domain::feat::session::protocol::session_closed::SessionClosed>(
            sidebar_topic(),
        );
}
