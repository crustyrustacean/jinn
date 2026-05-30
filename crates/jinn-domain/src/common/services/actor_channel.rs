//! Actor channel service — sends commands and events into the actor system.
//!
//! Wraps a [`kanal::Sender<AppMsg>`] so any holder of [`Services`] can submit
//! messages to the actor system without needing direct access to [`AppCore`].

use crate::protocol::{AppMsg, Command, Event};
use derive_more::Debug;

/// Service for sending messages into the actor system.
///
/// Wraps the core→actor channel. Clone is cheap (`kanal::Sender` is already `Clone`).
#[derive(Debug, Clone)]
pub struct ActorChannelService {
    #[debug("ActorChannelService")]
    /// The sender half of the actor message channel.
    sender: kanal::Sender<AppMsg>,
}

impl ActorChannelService {
    /// Creates a new actor channel service from the given sender.
    #[must_use]
    pub fn new(sender: kanal::Sender<AppMsg>) -> Self {
        Self { sender }
    }

    /// Sends a command into the actor system (no source actor).
    pub fn send_command(&self, command: Command) {
        let _ = self.sender.send(AppMsg::Command {
            command,
            source: None,
        });
    }

    /// Sends an event into the actor system (no source actor).
    pub fn send_event(&self, event: Event) {
        let _ = self.sender.send(AppMsg::Event {
            event,
            source: None,
        });
    }

    /// Sends a raw [`AppMsg`] into the actor system.
    pub fn send(&self, msg: AppMsg) {
        let _ = self.sender.send(msg);
    }

    /// Returns the service name for debugging.
    #[must_use]
    pub fn name(&self) -> &'static str {
        "actor-channel"
    }
}
