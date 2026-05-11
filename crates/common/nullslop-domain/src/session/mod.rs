//! Session management — session lifecycle, persistence, and loading.
//!
//! Provides persistence types ([`PersistedSession`], [`SessionStore`], etc.)
//! used by the session actor, services container, and component crate.
//! Also contains the session actor, intent handlers, validators, entry loaders,
//! and picker rendering.

mod persisted_session;
pub mod session_store;

pub mod actor;
pub mod entries;
pub mod intent;
pub mod render;
pub mod validator;

pub use persisted_session::{BLOB_STRATEGY_STATE, PersistedSession, SessionSummary};
pub use session_store::{JsonlSessionStore, SessionStore, SessionStoreError, SessionStoreService};

use std::sync::Arc;

use crate::actor::{Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink};
use crate::actor_host::{ActorSpawnResult, spawn_actor};

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
    state: crate::component::State,
    session_store: SessionStoreService,
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
