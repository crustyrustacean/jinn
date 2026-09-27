//! Generic actor lifecycle phase, applicable to every actor in the system.
//!
//! A pure value type: the dashboard folds
//! [`trouper::ActorLifecycle`](https://docs.rs/trouper) — the runtime's
//! spawn and stop announcement — into it, and any consumer of actor
//! status can compare against it without depending on `jinn-kernel`.

/// The lifecycle phase of an actor.
///
/// Driven by the runtime's own actor announcements: a spawn makes the
/// actor `Running`, a stop makes it `Dead` or `Idle`.
///
/// `Starting` is a jinn-side projection rather than a runtime-reported
/// state. A runtime spawn announcement means the actor is already live,
/// so the runtime never reports `Starting`. A feature that expects its
/// own actor to appear may declare it `Starting` up front via
/// `ServiceStatusUpdate`, and the runtime's announcement promotes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ActorLifecycle {
    /// The actor has been announced by a feature but not yet by the
    /// runtime.
    Starting,
    /// The actor has spawned and is ready.
    Running,
    /// The actor is dormant: the runtime evicted it for idleness, and it
    /// will re-spawn on the next send to its path.
    ///
    /// Distinct from [`ActorLifecycle::Dead`] because the actor is not
    /// gone. Partition-set entities (per-session workers, for instance)
    /// passivate on an idle window and come straight back, so reporting
    /// them as dead describes a failure that did not happen.
    Idle,
    /// The actor has stopped and will not come back on its own.
    Dead,
}
