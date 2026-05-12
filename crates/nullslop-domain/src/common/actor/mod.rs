//! Actor SDK for building nullslop actors.
//!
//! Actor authors implement the [`Actor`] trait. The host module
//! (`actor_host`) manages lifecycle, bus routing, and run loops.
//!
//! # Core types
//!
//! - [`Actor`] — async trait that actor authors implement
//! - [`ActorRef<M>`] — typed, cloneable handle for sending messages to an actor
//! - [`ActorEnvelope<M>`] — wrapper for all messages an actor can receive
//! - [`ActorContext`] — subscriptions, peer refs, and message sink
//! - [`MessageSink`] — trait for sending bus messages from actors to the application

pub mod actor;
pub mod actor_name;
pub mod actor_ref;
pub mod command_msg;
pub mod context;
pub mod envelope;
pub mod event_msg;
pub mod message_sink;
pub mod protocol;

pub use actor::Actor;
pub use actor_name::ActorName;
pub use actor_ref::ActorRef;
pub use actor_ref::{ActorSendError, SendResult};
pub use command_msg::{CommandMsg, CommandName};
pub use context::ActorContext;
pub use envelope::{ActorEnvelope, SystemMessage};
pub use event_msg::{EventMsg, EventTypeName};
pub use message_sink::MessageSink;

/// Shared test utilities.
///
/// Provides a canonical [`RecordingSink`] that replaces local duplicates
/// across actor crates.
pub use message_sink::RecordingSink;

#[cfg(test)]
mod derive_tests;
