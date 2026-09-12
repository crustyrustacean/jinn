//! The fabric lifecycle events the dashboard consumes.
//!
//! The canonical types live in `jinn-slices` ([`jinn_slices::fabric`])
//! beside [`crate::contracts::ServiceStatusUpdate`]: kameo bus dispatch
//! is by `TypeId`, so the kernel publishers (`spawn_tracked!`) and the
//! dashboard's relays must name the *same* Rust type — schema-id-equal
//! mirrors silently drop every event. This module is the dashboard's
//! import surface for them.
//!
//! [`crate::contracts::ServiceStatusUpdate`]: jinn_slices::ServiceStatusUpdate

pub use jinn_slices::fabric::ActorShutdownCompleted;
pub use jinn_slices::fabric::ActorStarted;
pub use jinn_slices::fabric::ActorStarting;
