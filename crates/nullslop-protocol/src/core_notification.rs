//! Core lifecycle notifications.

/// Lifecycle notifications from the actor system to the core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreNotification {
    /// The actor system has completed shutdown.
    ShutdownComplete,
}
