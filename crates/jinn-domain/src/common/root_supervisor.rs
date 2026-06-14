//! Root supervisor actor — anchors the actor supervision tree.
//!
//! Spawned first during app startup. Every other actor is spawned as a supervised
//! child of this root via [`Actor::supervise`] with [`RestartPolicy::Never`]. When
//! the root is stopped gracefully (on app exit), kameo's lifecycle code cascades
//! shutdown to all children and waits for them to close
//! ([`Links::send_children_shutdown`] + [`Links::wait_children_closed`], kameo
//! `spawn.rs:216-227`).
//!
//! The root itself does no work; it exists solely to anchor the supervision tree
//! so that a single `stop_gracefully()` + `wait_for_shutdown()` (raced against a
//! timeout in `run.rs`) coordinates shutdown of the entire actor system.
//!
//! [`Links::send_children_shutdown`]: kameo::links::Links
//! [`Links::wait_children_closed`]: kameo::links::Links

use kameo::Actor; // bring derive-able Actor trait into scope
use kameo::actor::Spawn; // brings spawn/wait_for_startup into scope
use kameo::prelude::ActorRef;

/// The root of the actor supervision tree.
///
/// Spawn via [`RootSupervisor::spawn_root`], then supervise every other actor
/// against the returned [`ActorRef`].
#[derive(Actor)]
pub struct RootSupervisor;

impl RootSupervisor {
    /// Spawns the root supervisor and waits for its startup to complete.
    ///
    /// Returns a ready-to-use [`ActorRef`] that can be passed to
    /// `Actor::supervise(&root, args)` for every other actor.
    ///
    /// # Panics
    ///
    /// Panics if the actor fails to start (should not happen for a no-op actor).
    pub async fn spawn_root() -> ActorRef<Self> {
        let root = Self::spawn(RootSupervisor);
        root.wait_for_startup().await;
        root
    }
}

/// Type alias for the root supervisor's actor ref.
pub type RootSupervisorRef = ActorRef<RootSupervisor>;
