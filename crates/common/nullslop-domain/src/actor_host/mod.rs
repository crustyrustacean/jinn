//! Host infrastructure for the nullslop actor system.
//!
//! Provides [`InMemoryActorHost`] for managing actor lifecycle — routing bus
//! events/commands into actors via closure-based [`RoutingEntry`], spawning
//! tokio tasks, and shutting down gracefully.

#![allow(
    clippy::panic,
    reason = "startup invariants in routing use intentional panics"
)]

pub mod actor_host;
pub mod fake;
pub mod in_memory;
pub mod routing;

pub use actor_host::ActorHostError;
pub use actor_host::{ActorHost, ActorHostService};
pub use fake::FakeActorHost;
pub use in_memory::{ActorSpawnResult, InMemoryActorHost, spawn_actor};
pub use routing::RoutingEntry;
