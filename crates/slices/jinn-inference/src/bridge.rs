//! The inference slice's bridge drain: forward routes and topic.
//!
//! Topics are named here (the inference slice consumes them); the relays
//! themselves are kernel bridge actors spawned by this drain — the
//! slice-local drain convention (the slice owns the message types it
//! names, the kernel must not depend on slice crates).

use jinn_domain::Services;

use crate::inference_topic;

/// Drains the inference slice's forward routes into bridge relays:
/// `SendToLlmProvider` and `CancelStream` dispatch commands, plus the
/// actor's own `StreamCompleted` echo (all onto `jinn.inference`).
///
/// Slice-local drain, called by composition after activation. Relays
/// register on the bus in their own `on_start`, so drain ordering
/// relative to publishers is free.
pub fn install_topic_routes(services: &Services) {
    services
        .bus
        .route_topic::<jinn_inference_msg::SendToLlmProvider>(inference_topic());
    services
        .bus
        .route_topic::<jinn_inference_msg::CancelStream>(inference_topic());
    services
        .bus
        .route_topic::<jinn_inference_msg::StreamCompleted>(inference_topic());
}
