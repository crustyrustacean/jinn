//! The inference slice — the actor that drives LLM provider streams.
//!
//! Hosts the trouper [`ServiceActor`] inference actor (converted from the
//! kernel `LlmActor`). It consumes the slice-owned dispatch commands
//! ([`SendToLlmProvider`], [`CancelStream`]) and its own [`StreamCompleted`]
//! echo, builds/opens the provider
//! stream via the `LlmService` factory in `Services`, and republishes stream
//! facts (`StreamToken`, `StreamCompleted`, tool-stream events, error/cancel
//! entries) on the fabric — the single write point the session actor's
//! folds already consume.
//!
//! Streaming runs as plain tokio tasks *outside* the actor loop; the actor
//! loop only sees the three crossing messages (plus tombstone bookkeeping).
//!
//! Kernel dependency (see Cargo.toml): the actor publishes on the fabric
//! and resolves LLM factories through `Services`, granted at activation.

mod session;

pub mod inference_actor;

use jinn_slices::SliceHost;

pub use jinn_inference_msg::CancelStream;
pub use jinn_inference_msg::SendToLlmProvider;
pub use jinn_inference_msg::StreamCompleted;

/// Activates the slice: spawns the inference actor on trouper (its
/// `.subscribe` declarations are the readiness point). The dispatch
/// commands and the actor's own `StreamCompleted` echo arrive by
/// schema broadcast (the actor publishes completion on the fabric and
/// re-consumes it to finalize per-session tracking).
pub fn activate(
    host: &mut SliceHost<'_, jinn_slices::RenderFacts>,
    services: jinn_domain::Services,
) {
    let _path = inference_actor::InferenceActor::spawn(host.system(), services);
}
