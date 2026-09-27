//! The context-assembly slice — a stateless assembly service.
//!
//! Assembling a system prompt + conversation messages is a PURE
//! function of the caller-provided [`jinn_context_assembly_msg::AssemblyInputs`]: the
//! service never reads `AppState`. The kernel's queue/session dispatch
//! paths snapshot the session state they can see, send an
//! `AssembleContext` message to the `context-assembly` trouper actor,
//! and receive an `AssembledResponse` reply.

//! # Assembly Pipeline
//!
//! Chat history is assembled into LLM-ready messages through these stages:
//!
//! 1. **Pin splitting** - entries are separated into TOP pins, BOTTOM pins,
//!    and working history based on pin position.
//! 2. **Compaction** - if the working history exceeds the session's token budget,
//!    entries are trimmed newest-to-oldest (preserving pinned entries); compaction
//!    summaries ride in the working history as ordinary messages.
//! 3. **System prompt construction** - the system prompt is composed from
//!    dedicated per-section builders in fixed order:
//!    - Persona body
//!    - Project context files
//!    - Tool context block
//!    - Skills block (`<available_skills>` XML catalog)
//!    - Current date
//!    - Working directory
//!
//!    Sections with no content are omitted entirely.
//! 4. **Message ordering** - the final array is pure conversation history:
//!    `[TOP pins] -> [compacted working history] -> [BOTTOM pins] -> [last message]`.
//!    The system prompt travels separately from the assembled messages.

#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::panic,
        reason = "test assertions on infallible registration"
    )
)]

pub mod assemble;

#[cfg(test)]
pub(crate) mod assembly_test_bridge;
pub mod inputs;
pub mod inputs_snapshot;
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
