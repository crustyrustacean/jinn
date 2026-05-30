//! Trait for sending commands and events from actors to the application.
//!
//! [`MessageSink`] abstracts how an actor's output reaches the rest of the
//! application. The actor crate defines the trait; the application provides
//! the implementation (e.g. one that submits `AppMsg` to `AppCore`'s channel).

use crate::protocol::{Command, Event};
use parking_lot::Mutex;

use super::actor_ref::SendResult;

/// Trait for sending bus messages from actors to the application.
///
/// Implemented by the application wiring layer (Phase 3). Actors call
/// `send_command`/`send_event` through their [`ActorContext`](crate::ActorContext),
/// which delegates to the underlying `MessageSink`.
pub trait MessageSink: Send + Sync + 'static {
    /// Returns a human-readable name for this sink, for debugging.
    fn name(&self) -> &'static str;

    /// Sends a command to the bus.
    ///
    /// # Errors
    ///
    /// Returns an error if the command cannot be delivered.
    fn send_command(&self, command: Command) -> SendResult;

    /// Sends an event to the bus.
    ///
    /// # Errors
    ///
    /// Returns an error if the event cannot be delivered.
    fn send_event(&self, event: Event) -> SendResult;
}

/// A message sink for testing that records sent commands and events.
///
/// Shared across the workspace - actor tests, host tests, and integration tests
/// all use this instead of defining local duplicates.
pub struct RecordingSink {
    /// Recorded commands.
    commands: Mutex<Vec<Command>>,
    /// Recorded events.
    events: Mutex<Vec<Event>>,
}

impl Default for RecordingSink {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingSink {
    /// Creates a new empty recording sink.
    pub fn new() -> Self {
        Self {
            commands: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
        }
    }

    /// Returns all commands sent through this sink.
    pub fn commands(&self) -> Vec<Command> {
        self.commands.lock().clone()
    }

    /// Returns all events sent through this sink.
    pub fn events(&self) -> Vec<Event> {
        self.events.lock().clone()
    }

    /// Drains and returns all events, leaving the sink empty.
    pub fn take_events(&self) -> Vec<Event> {
        let mut guard = self.events.lock();
        std::mem::take(&mut guard)
    }

    /// Drains and returns all commands, leaving the sink empty.
    pub fn take_commands(&self) -> Vec<Command> {
        let mut guard = self.commands.lock();
        std::mem::take(&mut guard)
    }

    /// Clears all recorded commands and events.
    pub fn clear(&self) {
        self.commands.lock().clear();
        self.events.lock().clear();
    }
}

impl MessageSink for RecordingSink {
    fn name(&self) -> &'static str {
        "recording_sink"
    }

    fn send_command(&self, command: Command) -> SendResult {
        self.commands.lock().push(command);
        Ok(())
    }

    fn send_event(&self, event: Event) -> SendResult {
        self.events.lock().push(event);
        Ok(())
    }
}
