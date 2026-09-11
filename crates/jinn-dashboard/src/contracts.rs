//! Crossing contracts: the messages the dashboard consumes across the
//! fabric boundary.
//!
//! [`ServiceStatusUpdate`] is dashboard-owned (the projection every
//! feature publishes). The lifecycle events are jinn-domain-owned; per
//! the migration contract (bridge routing is schema-id based) they are
//! mirrored here as wire-shape-compatible structs with identical
//! schema descriptors — the conformance test in `tests.rs` pins the
//! two definitions together.

use serde::Deserialize;
use serde::Serialize;

use jinn_core_types::ActorLifecycle;

/// A service's status for the dashboard, published by the owning feature.
///
/// Generic projection onto a dashboard row: `lifecycle: None` leaves the
/// row's lifecycle untouched (it is driven by the actor-lifecycle events),
/// and `description: None` preserves any existing description. Features
/// translate their service-specific state into this event so the dashboard
/// never needs to know a feature exists.
///
/// This is a bridge-crossing type: the forward relay serializes it onto
/// `jinn.fabric` under its [`jinn_slices::crossing_schema`] contract, so
/// the canvas actor's topic subscription can decode it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceStatusUpdate {
    /// The dashboard row name (the key the owning feature publishes under,
    /// e.g. its `spawn_tracked!`/actor name).
    pub name: String,
    /// New row description; `None` preserves the existing one.
    pub description: Option<String>,
    /// New lifecycle; `None` leaves the row's lifecycle untouched.
    pub lifecycle: Option<ActorLifecycle>,
    /// Free-form status message for the third column.
    pub status_message: Option<String>,
}

impl jinn_slices::BusMessage for ServiceStatusUpdate {}

jinn_slices::crossing_schema!(ServiceStatusUpdate, "ServiceStatusUpdate", trouper::schema::SchemaKind::Event,
    description: "A feature's projection onto its dashboard row (optional lifecycle, description, status message).",
    fields: ["name" => trouper::schema::FieldTy::Str]);
