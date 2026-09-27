//! Generic actor lifecycle phase, applicable to every actor in the system.
//!
//! A pure value type: the dashboard folds
//! [`trouper::ActorLifecycle`](https://docs.rs/trouper) — the runtime's
//! spawn and stop announcement — into it, and any consumer of actor
//! status can compare against it without depending on `jinn-kernel`.

/// The lifecycle phase of an actor, as the runtime reports it.
///
/// Every variant mirrors a state the runtime can actually announce; there
/// is no jinn-side projection variant. A feature cannot write this value:
/// the dashboard's fold is its only writer, so the enum is the runtime's
/// vocabulary rather than a shared opinion about it.
///
/// The two sanctioned stops are absent by design. A runtime `Normal` or
/// `Shutdown` stop means the actor finished or was torn down deliberately,
/// so the dashboard removes its row rather than keeping a row that reports
/// a clean ending as a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ActorLifecycle {
    /// The actor has spawned and is ready.
    Running,
    /// The actor is dormant: the runtime evicted it for idleness, and it
    /// will re-spawn on the next send to its path.
    ///
    /// Distinct from a failure because the actor is not gone.
    /// Partition-set entities (per-session workers, for instance)
    /// passivate on an idle window and come straight back.
    Idle,
    /// The restart budget was exhausted and the failure was escalated to
    /// the parent — the worst outcome the runtime can report.
    Escalated,
    /// A handler panicked and the supervisor declined to restart the actor.
    Crashed,
}
