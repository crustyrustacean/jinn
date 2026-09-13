//! Crossing contracts: the messages the dashboard consumes across the
//! fabric boundary.
//!
//! [`ServiceStatusUpdate`] is the kernel-surface vocabulary every feature
//! publishes (it lives in `jinn-slices`, re-exported here). The lifecycle
//! events are shared Rust types from `jinn_slices::fabric`, re-exported by
//! [`crate::fabric_events`]: kameo bus dispatch is by `TypeId`, so
//! wire-shape mirrors would silently drop every event — one type,
//! imported by both sides.

pub use jinn_slices::ServiceStatusUpdate;
