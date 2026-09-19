//! The preferences slice's bridge drain: forward routes and topic.
//!
//! Topics are named here (the slice consumes them); the relays
//! themselves are kernel bridge actors spawned by this drain — the
//! slice-local drain convention (the slice owns the message types it
//! names, the kernel must not depend on slice crates).

use jinn_domain::Services;

/// The preferences slice's crossing topic (`jinn.preferences`): the
/// `UpdatePreferences`/`UpdateAppState` persistence commands forward
/// onto it for the two actors.
#[must_use]
pub fn preferences_topic() -> trouper::topics::Topic {
    trouper::topics::Topic::new("jinn.preferences")
}

/// Drains the preferences slice's forward routes into bridge relays:
/// the persistence commands forward onto the preferences topic
/// (kernel topic → `jinn.preferences`).
///
/// Slice-local drain, called by composition after activation. Relays
/// register on the bus in their own `on_start`, so drain ordering
/// relative to publishers is free — the actors' own subscribes (the
/// readiness point) happen in `activate`, before any publish.
pub fn install_topic_routes(services: &Services) {
    services.bus.route_topic::<jinn_preferences_config::protocol::command::UpdatePreferences>(preferences_topic());
    services.bus.route_topic::<jinn_preferences_config::protocol::app_state_command::UpdateAppState>(preferences_topic());
}

