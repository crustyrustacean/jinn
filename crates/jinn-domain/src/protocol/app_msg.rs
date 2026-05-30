//! Application message type for the processing loop.

pub mod command;
pub mod event;

pub use command::Command;
pub use command::DynamicCommand;
pub use event::DynamicEvent;
pub use event::Event;

use crate::protocol::ActorName;

/// An application message for the core processing loop.
#[allow(
    clippy::large_enum_variant,
    reason = "boxing would cascade through all match arms"
)]
#[derive(Debug)]
pub enum AppMsg {
    /// A command to be routed through the bus.
    Command {
        /// The command payload.
        command: Command,
        /// The actor that submitted this command, if any.
        source: Option<ActorName>,
    },
    /// An event from an actor (routed through the bus).
    Event {
        /// The event payload.
        event: Event,
        /// The actor that submitted this event, if any.
        source: Option<ActorName>,
    },
}
