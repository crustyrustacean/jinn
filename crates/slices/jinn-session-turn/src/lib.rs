//! The session-turn slice.
//!
//! This slice owns the one coordinated reducer that advances session turns:
//! enqueue, streaming, tool continuation, retry, context mutation, and
//! turn-path persistence. During the crate move, [`activate`] forwards to the
//! existing kernel actor so composition can migrate before handler ownership
//! moves. The forwarding seam is transitional and is not a second owner.

pub mod session_actor;

use session_actor::{SessionPersistenceActor, SessionPersistenceActorDeps};
use trouper::actor::ActorPath;
use trouper::system::ActorSystem;

/// Activates the session-turn reducer at the shared static actor path.
///
/// # Panics
///
/// Panics if the reducer path is already occupied or its subscriptions fail.
/// Either condition is a composition error and must abort launch.
pub fn activate(system: &ActorSystem, deps: SessionPersistenceActorDeps) -> ActorPath {
    SessionPersistenceActor::spawn(system, deps)
}
