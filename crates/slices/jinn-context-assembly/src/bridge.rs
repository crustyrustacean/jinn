//! The context-assembly slice's bridge drain: forward routes and topic.
//!
//! Topics are named here (the slice consumes them); the relays
//! themselves are kernel bridge actors spawned by this drain — the
//! slice-local drain convention (the slice owns the message types it
//! names, the kernel must not depend on slice crates).

use jinn_domain::Services;

/// The context-assembly slice's crossing topic (`jinn.context-assembly`):
/// kernel context-affecting events forward onto it for the size actor.
#[must_use]
pub fn context_assembly_topic() -> trouper::topics::Topic {
    trouper::topics::Topic::new("jinn.context-assembly")
}

/// Drains the context-assembly slice's forward routes into bridge
/// routes: the kernel events the size actor folds (kernel topic →
/// `jinn.context-assembly`).
///
/// Slice-local drain, called by composition after activation. Relays
/// register on the bus in their own `on_start`, so drain ordering
/// relative to publishers is free.
pub fn install_topic_routes(services: &Services) {
    services
        .bus
        .route_topic::<jinn_session_history_msg::HistoryAppended>(context_assembly_topic());
    services
        .bus
        .route_topic::<jinn_domain::feat::context::protocol::event::ContextOverrideChanged>(
            context_assembly_topic(),
        );
    services
        .bus
        .route_topic::<jinn_domain::protocol::system::ActiveSessionChanged>(
            context_assembly_topic(),
        );
    services
        .bus
        .route_topic::<jinn_session_history_msg::ChatEntryPinChanged>(context_assembly_topic());
    services
        .bus
        .route_topic::<jinn_domain::feat::session::protocol::session_load_completed::SessionLoadCompleted>(
            context_assembly_topic(),
        );
}
