//! Session management slice — session creation, model refresh, and prompt template rescan.
//!
//! Handles creating new sessions, refreshing the model list from the
//! active provider, and rescanning prompt templates. No element —
//! rendering stays in `nullslop-tui`.

use std::sync::Arc;

use nullslop_actor::{Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink};
use nullslop_actor_host::{ActorSpawnResult, spawn_actor};

pub mod actor;
pub mod entries;
pub mod intent;
pub mod persistence;
pub mod render;
pub mod validator;

/// Spawns the session persistence actor on the given tokio runtime handle.
///
/// Creates the actor's channel, injects `state` and `session_store` as context
/// data, activates the actor, and returns the [`ActorRef`] for sending direct
/// messages and the [`ActorSpawnResult`] for routing integration.
///
/// # Panics
///
/// Panics if the actor fails to activate (should never happen with valid injection).
pub fn spawn_session_actor(
    state: nullslop_component::State,
    session_store: persistence::SessionStoreService,
    sink: Arc<dyn MessageSink>,
    handle: &tokio::runtime::Handle,
) -> (
    ActorRef<actor::SessionPersistenceDirectMsg>,
    ActorSpawnResult,
) {
    let (tx, rx) = kanal::unbounded::<ActorEnvelope<actor::SessionPersistenceDirectMsg>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new("session-persistence", sink);
    ctx.set_description("Persists session data to disk");
    ctx.set_data(state);
    ctx.set_data(session_store);
    let actor = actor::SessionPersistenceActor::activate(&mut ctx);
    let result = spawn_actor("session-persistence", actor, &actor_ref, rx, ctx, handle);
    (actor_ref, result)
}
