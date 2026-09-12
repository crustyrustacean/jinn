//! Crossing contracts: the messages the dashboard consumes across the
//! fabric boundary.
//!
//! [`ServiceStatusUpdate`] is the kernel-surface vocabulary every feature
//! publishes (it lives in `jinn-slices`, re-exported here). The lifecycle
//! events are jinn-domain-owned; per the migration contract (bridge
//! routing is schema-id based) they are mirrored here as wire-shape-
//! compatible structs with identical schema descriptors — the conformance
//! test in `tests.rs` pins the two definitions together.

pub use jinn_slices::ServiceStatusUpdate;
