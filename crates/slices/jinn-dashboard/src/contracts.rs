//! Crossing contracts: the messages the dashboard consumes across the
//! fabric boundary.
//!
//! Two sources, deliberately asymmetric:
//!
//! - [`trouper::ActorLifecycle`] is the runtime's own announcement of
//!   every actor spawn and stop. It is not a jinn type and nothing in
//!   jinn publishes it — the dashboard folds it to decide which rows
//!   exist and whether they are alive.
//! - [`ServiceStatusUpdate`] is jinn's vocabulary, published by a
//!   feature that wants to add a description and a status message to
//!   its own row.
//!
//! A feature cannot bring an actor into being by publishing, and cannot
//! contradict the runtime's verdict on whether it is alive.

pub use jinn_slices::ServiceStatusUpdate;
