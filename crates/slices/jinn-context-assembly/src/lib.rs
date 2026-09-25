//! The context-assembly slice — a stateless assembly service.
//!
//! Assembling a system prompt + conversation messages is a PURE
//! function of the caller-provided [`jinn_context_assembly_msg::AssemblyInputs`]: the
//! service never reads `AppState`. The kernel's queue/session dispatch
//! paths snapshot the session state they can see, send an
//! `AssembleContext` message to the `context-assembly` trouper actor,
//! and receive an `AssembledResponse` reply.

#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::panic,
        reason = "test assertions on infallible registration"
    )
)]

pub mod assemble;
pub mod inputs;
pub mod service;
pub mod size_actor;

/// Installs the slice's actors on a trouper system: spawns the stateless
/// assembly service and the context-size actor (whose `.subscribe`
/// declarations are the readiness point).
///
/// Split from composition so tests (and any composition that owns a bare
/// [`trouper::system::ActorSystem`]) can wire the actor fabric without
/// the kernel's `Services`.
///
/// # Panics
///
/// Panics if the size actor cannot spawn — a broken actor
/// system, not a caller bug.
pub fn install_actors(
    system: &trouper::system::ActorSystem,
    state: jinn_domain::common::state::State,
    services: &jinn_domain::Services,
) {
    let _ = service::spawn(system);
    let _path = size_actor::ContextSizeActor::spawn(system, state, services.clone());
}
