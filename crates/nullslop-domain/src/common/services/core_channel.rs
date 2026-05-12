//! Core channel service — signals the core about lifecycle events.
//!
//! Wraps a [`kanal::Sender<CoreNotification>`] so actors (or any holder of
//! [`Services`]) can notify the core about shutdown, settlement, etc.
//!
//! This replaces sleep-polling hacks in coordinated shutdown and the headless runner.

use derive_more::Debug;

// Re-export CoreNotification from nullslop-protocol for backward compatibility.
pub use crate::protocol::CoreNotification;

/// Service for sending lifecycle notifications to the core.
///
/// Wraps the actor→core channel. Clone is cheap (`kanal::Sender` is already `Clone`).
#[derive(Debug, Clone)]
pub struct CoreChannelService {
    #[debug("CoreChannelService")]
    /// The sender half of the core notification channel.
    sender: kanal::Sender<CoreNotification>,
}

impl CoreChannelService {
    /// Creates a new core channel service from the given sender.
    #[must_use]
    pub fn new(sender: kanal::Sender<CoreNotification>) -> Self {
        Self { sender }
    }

    /// Sends a notification to the core.
    pub fn send(&self, notification: CoreNotification) {
        let _ = self.sender.send(notification);
    }

    /// Returns the service name for debugging.
    #[must_use]
    pub fn name(&self) -> &'static str {
        "core-channel"
    }
}
