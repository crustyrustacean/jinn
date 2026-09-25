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
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "An actor is starting up.")]
pub struct ActorStarting {
    /// The actor's name.
    pub name: String,
    /// A short human-readable description of what the actor does.
    pub description: Option<String>,
}

/// An actor has finished starting up.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "An actor has finished starting up.")]
pub struct ActorStarted {
    /// The actor's name.
    pub name: String,
    /// A short human-readable description of what the actor does.
    pub description: Option<String>,
}

/// An actor has completed shutdown.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "An actor has completed shutdown.")]
pub struct ActorShutdownCompleted {
    /// The actor's name.
    pub name: String,
}

impl crate::BusMessage for ActorStarting {}

impl crate::BusMessage for ActorStarted {}

impl crate::BusMessage for ActorShutdownCompleted {}
