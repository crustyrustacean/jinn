//! Shared fabric lifecycle events: the kernel's actor announcements.
//!
//! These types are published by kernel wiring (through
//! [`crate::BusService`] publishes) and consumed by the dashboard slice
//! — they live in this crate because kernel publishers and slice
//! subscribers must name the *same Rust type*: schema-id-equal mirror
//! structs silently drop every event. Same precedent as
//! [`crate::ServiceStatusUpdate`].

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
