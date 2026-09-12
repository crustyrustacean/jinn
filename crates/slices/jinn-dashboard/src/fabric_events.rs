//! Wire-shape mirrors of the fabric lifecycle events.
//!
//! The canonical types live in the kernel (they are published by every
//! kernel actor on the kameo bus). The dashboard consumes their
//! trouper-side envelopes, which are schema-id'd JSON — so a local
//! struct with the identical schema descriptor and serde shape decodes
//! them exactly (G4 in the migration spec). The conformance test
//! serializes the kernel type's documented shape and asserts this
//! mirror round-trips it under the same schema id.

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

impl jinn_slices::BusMessage for ActorStarting {}

jinn_slices::crossing_schema!(ActorStarting, "ActorStarting", trouper::schema::SchemaKind::Event,
    description: "An actor is starting up.",
    fields: ["name" => trouper::schema::FieldTy::Str]);

impl jinn_slices::BusMessage for ActorStarted {}

jinn_slices::crossing_schema!(ActorStarted, "ActorStarted", trouper::schema::SchemaKind::Event,
    description: "An actor has finished starting up.",
    fields: ["name" => trouper::schema::FieldTy::Str]);

impl jinn_slices::BusMessage for ActorShutdownCompleted {}

jinn_slices::crossing_schema!(ActorShutdownCompleted, "ActorShutdownCompleted", trouper::schema::SchemaKind::Event,
    description: "An actor has completed shutdown.",
    fields: ["name" => trouper::schema::FieldTy::Str]);
