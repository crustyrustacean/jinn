//! Actor lifecycle events.
//!
//! The three bus-travelling lifecycle events ([`ActorStarting`],
//! [`ActorStarted`], [`ActorShutdownCompleted`]) are defined in
//! `jinn-slices` ([`jinn_slices::fabric`]) and re-exported here:
//! kernel publishers and every subscriber must share one Rust type.
//! `AllActorsSpawned` lives in `jinn-boot-msg` (its producer is the
//! boot slice's system-ready actor contract surface).

pub use jinn_slices::fabric::ActorShutdownCompleted;
pub use jinn_slices::fabric::ActorStarted;
pub use jinn_slices::fabric::ActorStarting;
