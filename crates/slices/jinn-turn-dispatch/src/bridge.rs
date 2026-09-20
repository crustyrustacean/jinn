//! The turn-dispatch slice's bridge drain: forward routes and topic.
//!
//! Topics are named here (the turn-dispatch slice consumes them); the
//! relays themselves are kernel bridge actors spawned by this drain —
//! the slice-local drain convention (the slice owns the message types it
//! names, the kernel must not depend on slice crates).

use jinn_domain::Services;

use crate::turn_dispatch_topic;

/// Drains the turn-dispatch slice's forward routes into bridge relays:
/// the kernel `SessionPhaseChanged` event and the slice-owned
/// `DispatchTurn` command (kernel topic → `jinn.turn-dispatch`).
///
/// Slice-local drain, called by composition after activation. Relays
/// register on the bus in their own `on_start`, so drain ordering
/// relative to publishers is free.
pub fn install_topic_routes(services: &Services) {
    services.bus.route_topic::<jinn_domain::feat::session::protocol::session_phase_changed::SessionPhaseChanged>(turn_dispatch_topic());
    services
        .bus
        .route_topic::<jinn_turn_dispatch_msg::DispatchTurn>(turn_dispatch_topic());
}
