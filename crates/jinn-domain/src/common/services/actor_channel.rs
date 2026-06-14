//! Actor channel service - sends typed message closures into the actor system.
//!
//! Wraps a [`kanal::Sender<AppMsg>`] so any holder of [`Services`] can submit
//! message closures to the actor system without needing direct access to [`AppCore`].

use crate::protocol::AppMsg;
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

    /// Sends a raw [`AppMsg`] (bridge closure) into the actor system.
    pub fn send(&self, msg: AppMsg) {
        let _ = self.sender.send(msg);
    }

    /// Sends a typed bus message as a closure into the actor system.
    pub fn send_message<M: crate::common::bus::BusMessage>(&self, msg: M) {
        let closure = crate::common::bridge::Bridge::publish_closure(msg);
        self.send(closure);
    }

    /// Returns the service name for debugging.
    #[must_use]
    pub fn name(&self) -> &'static str {
        "actor-channel"
    }
}
