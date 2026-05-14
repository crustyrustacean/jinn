//! Fake actor host for testing.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::common::actor::SystemMessage;
use crate::protocol::{ActorName, Command, Event};
use error_stack::Report;
use parking_lot::Mutex;

use super::actor_host::ActorHost;
use super::actor_host::ActorHostError;

/// A fake actor host that records sent events, commands, and shutdown calls.
///
/// Use in tests to verify routing behavior without spawning real actors.
pub struct FakeActorHost {
    /// Events routed through this host.
    events_sent: Mutex<Vec<Event>>,
    /// Commands routed through this host.
    commands_sent: Mutex<Vec<Command>>,
    /// System messages routed through this host.
    system_sent: Mutex<Vec<SystemMessage>>,
    /// Whether shutdown has been called.
    shutdown_called: AtomicBool,
}

impl FakeActorHost {
    /// Creates a new empty fake host.
    #[must_use]
    pub fn new() -> Self {
        Self {
            events_sent: Mutex::new(Vec::new()),
            commands_sent: Mutex::new(Vec::new()),
            system_sent: Mutex::new(Vec::new()),
            shutdown_called: AtomicBool::new(false),
        }
    }

    /// Returns all events that were routed through this host.
    #[must_use]
    pub fn events_sent(&self) -> Vec<Event> {
        self.events_sent.lock().clone()
    }

    /// Returns all commands that were routed through this host.
    #[must_use]
    pub fn commands_sent(&self) -> Vec<Command> {
        self.commands_sent.lock().clone()
    }

    /// Returns all system messages that were routed through this host.
    #[must_use]
    pub fn system_sent(&self) -> Vec<SystemMessage> {
        self.system_sent.lock().clone()
    }

    /// Returns whether shutdown was called.
    #[must_use]
    pub fn is_shutdown(&self) -> bool {
        self.shutdown_called.load(Ordering::SeqCst)
    }
}

impl Default for FakeActorHost {
    fn default() -> Self {
        Self::new()
    }
}

impl ActorHost for FakeActorHost {
    fn name(&self) -> &'static str {
        "FakeActorHost"
    }

    fn send_event(&self, event: &Event, _source: Option<&ActorName>) {
        self.events_sent.lock().push(event.clone());
    }

    fn send_command(&self, command: &Command, _source: Option<&ActorName>) {
        self.commands_sent.lock().push(command.clone());
    }

    fn send_system(&self, msg: SystemMessage) {
        self.system_sent.lock().push(msg);
    }

    fn begin_shutdown(&self, completion_tx: tokio::sync::oneshot::Sender<()>) {
        // Fake host has no real actors — immediately signal completion.
        let _ = completion_tx.send(());
    }

    fn shutdown(&self) -> Result<(), Report<ActorHostError>> {
        self.shutdown_called.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn fake_host_tracks_events() {
        // Given a fake host.
        let host = FakeActorHost::new();

        // When sending a KeyDown event.
        host.send_event(
            &Event::KeyDown {
                payload: crate::protocol::system::KeyDown {
                    key: crate::protocol::KeyEvent {
                        key: crate::protocol::Key::Enter,
                        modifiers: crate::protocol::Modifiers::none(),
                    },
                },
            },
            None,
        );

        // Then the event is recorded.
        assert_eq!(host.events_sent().len(), 1);
        assert!(matches!(host.events_sent()[0], Event::KeyDown { .. }));
    }

    #[rstest::rstest]
    fn fake_host_tracks_commands() {
        // Given a fake host.
        let host = FakeActorHost::new();

        // When sending a command.
        host.send_command(&Command::RefreshModels, None);

        // Then the command is recorded.
        assert_eq!(host.commands_sent().len(), 1);
        assert!(matches!(host.commands_sent()[0], Command::RefreshModels));
    }

    #[rstest::rstest]
    fn fake_host_tracks_shutdown() {
        // Given a fake host.
        let host = FakeActorHost::new();

        // When calling shutdown.
        host.shutdown().expect("shutdown should succeed");

        // Then shutdown was recorded.
        assert!(host.is_shutdown());
    }

    #[rstest::rstest]
    fn fake_host_tracks_system_messages() {
        // Given a fake host.
        let host = FakeActorHost::new();

        // When sending a system message.
        host.send_system(SystemMessage::ApplicationShuttingDown);

        // Then the system message is recorded.
        assert_eq!(host.system_sent().len(), 1);
        assert!(matches!(
            host.system_sent()[0],
            SystemMessage::ApplicationShuttingDown
        ));
    }

    #[rstest::rstest]
    fn fake_host_name() {
        // Given a fake host.
        let host = FakeActorHost::new();

        // When querying the host name.
        assert_eq!(host.name(), "FakeActorHost");

        // Then name is correct.
    }
}
