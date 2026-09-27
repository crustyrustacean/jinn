//! The citations slice — shape-based web-source detection from tool
//! traffic.
//!
//! Hosts the trouper [`ServiceActor`] citations actor (converted from the
//! dormant `url-citations` plugin). It consumes the tool-loop facts
//! ([`ToolCallReceived`], [`ToolExecutionCompleted`]) and the stream
//! terminal ([`StreamCompleted`]), detects citable URLs by shape
//! ([`detect`]) with no I/O, and publishes one [`CitationsReceived`]
//! flush per turn that reaches a genuine finish — the session actor's
//! existing fold renders it as the Sources annotation. Aborted turns
//! retain the buffer so a later successful turn still surfaces the
//! sources.
//!
//! Kernel dependency (see Cargo.toml): publishes through `Services`'
//! bus, granted at slice activation.

pub mod citations_actor;
pub mod detect;

use jinn_kernel::Services;
use jinn_slices::RenderFacts;
use jinn_slices::SliceHost;

/// Activates the slice: spawns the citations actor on trouper (its
/// `.subscribe` declarations are the readiness point). Citations have
/// no config knobs — the detection rules are fixed by shape.
pub fn activate(host: &mut SliceHost<'_, RenderFacts>, services: Services) {
    let _ = citations_actor::CitationsActor::spawn(
        host.system(),
        citations_actor::CitationsActorDeps { services },
    );
}
