//! Actor lifecycle domain: commands and events for actor startup, shutdown coordination.

pub use crate::common::actor::protocol::command::ProceedWithShutdown;
pub use crate::common::actor::protocol::event::{
    ActorShutdownCompleted, ActorStarted, ActorStarting,
};
