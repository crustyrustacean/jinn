//! Shared fabric lifecycle events: the kernel's actor announcements.
//!
//! These types are published by kernel wiring ([`crate::BusMessage`]
//! publishes on the kameo bus) and consumed by the dashboard slice —
//! they live in this crate because **kameo bus dispatch is by
//! [`TypeId`]**: a subscriber and a publisher must name the *same Rust
//! type*, not merely schema-equal ones. Schema-id-equal mirror structs
//! silently drop every event, which is exactly the failure mode this
//! co-location prevents. Same precedent as [`crate::ServiceStatusUpdate`].

use serde::Deserialize;
use serde::Serialize;

/// An actor is starting up.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActorStarting {
    /// The actor's name.
    pub name: String,
    /// A short human-readable description of what the actor does.
    pub description: Option<String>,
}

/// An actor has finished starting up.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActorStarted {
    /// The actor's name.
    pub name: String,
    /// A short human-readable description of what the actor does.
    pub description: Option<String>,
}

/// An actor has completed shutdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActorShutdownCompleted {
    /// The actor's name.
    pub name: String,
}

impl crate::BusMessage for ActorStarting {}

crate::crossing_schema!(ActorStarting, "ActorStarting", trouper::schema::SchemaKind::Event,
    description: "An actor is starting up.",
    fields: ["name" => trouper::schema::FieldTy::Str]);

impl crate::BusMessage for ActorStarted {}

crate::crossing_schema!(ActorStarted, "ActorStarted", trouper::schema::SchemaKind::Event,
    description: "An actor has finished starting up.",
    fields: ["name" => trouper::schema::FieldTy::Str]);

impl crate::BusMessage for ActorShutdownCompleted {}

crate::crossing_schema!(ActorShutdownCompleted, "ActorShutdownCompleted", trouper::schema::SchemaKind::Event,
    description: "An actor has completed shutdown.",
    fields: ["name" => trouper::schema::FieldTy::Str]);
