//! Actor lifecycle events.

use serde::{Deserialize, Serialize};

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

/// All actors have been spawned.
///
/// Emitted after the wiring code finishes spawning every actor.
/// The system-ready actor waits for this event before checking whether
/// its running count of `ActorStarted` events matches the total.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllActorsSpawned;

impl crate::common::bus::BusMessage for AllActorsSpawned {}

impl crate::common::bus::BusMessage for ActorStarting {}

impl crate::common::bus::BusMessage for ActorStarted {}

impl crate::common::bus::BusMessage for ActorShutdownCompleted {}

jinn_slices::crossing_schema!(ActorStarting, "ActorStarting", trouper::schema::SchemaKind::Event,
    description: "An actor is starting up.",
    fields: ["name" => trouper::schema::FieldTy::Str]);

jinn_slices::crossing_schema!(ActorStarted, "ActorStarted", trouper::schema::SchemaKind::Event,
    description: "An actor has finished starting up.",
    fields: ["name" => trouper::schema::FieldTy::Str]);

jinn_slices::crossing_schema!(ActorShutdownCompleted, "ActorShutdownCompleted", trouper::schema::SchemaKind::Event,
    description: "An actor has completed shutdown.",
    fields: ["name" => trouper::schema::FieldTy::Str]);
