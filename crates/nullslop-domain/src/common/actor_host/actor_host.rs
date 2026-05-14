//! Actor host trait and service wrapper.

use std::sync::Arc;

use crate::common::actor::SystemMessage;
use crate::protocol::{ActorName, Command, Event};
use error_stack::Report;
use wherror::Error;

/// Error type for actor host operations.
#[derive(Debug, Error)]
#[error(debug)]
pub struct ActorHostError;

/// Trait for managing actors.
///
/// Implemented by [`InMemoryActorHost`](crate::InMemoryActorHost) for production
/// and [`FakeActorHost`](crate::FakeActorHost) for testing. Provides routing
/// of events/commands to actors and graceful shutdown.
pub trait ActorHost: Send + Sync + 'static {
    /// Returns the host's name.
    fn name(&self) -> &'static str;

    /// Routes an event to subscribed actors, skipping the source.
    fn send_event(&self, event: &Event, source: Option<&ActorName>);

    /// Routes a command to registered actors, skipping the source.
    fn send_command(&self, command: &Command, source: Option<&ActorName>);

    /// Sends a system message to all actors (no subscription needed).
    fn send_system(&self, msg: SystemMessage);

    /// Initiates coordinated shutdown tracking.
    ///
    /// Populates the shutdown tracker with all known actor names and stores
    /// the oneshot sender. When all actors complete their shutdown, the
    /// sender fires. Callers should call this before `send_system(ApplicationShuttingDown)`.
    fn begin_shutdown(&self, completion_tx: tokio::sync::oneshot::Sender<()>);

    /// Shuts down all actors gracefully.
    ///
    /// # Errors
    ///
    /// Returns an error if any actors fail to shut down within the timeout.
    fn shutdown(&self) -> Result<(), Report<ActorHostError>>;
}

/// Service wrapper for the actor host.
///
/// Wraps `Arc<dyn ActorHost>` for shared ownership across the application.
/// Follows the service wrapper pattern from the project style guide.
#[derive(Clone)]
pub struct ActorHostService {
    /// The underlying actor host implementation.
    svc: Arc<dyn ActorHost>,
}

impl std::fmt::Debug for ActorHostService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActorHostService")
            .field("name", &self.svc.name())
            .finish()
    }
}

impl ActorHostService {
    /// Creates a new actor host service wrapping the given host.
    #[must_use]
    pub fn new(host: Arc<dyn ActorHost>) -> Self {
        Self { svc: host }
    }

    /// Returns a reference to the underlying host trait object.
    #[must_use]
    pub fn backend(&self) -> &dyn ActorHost {
        self.svc.as_ref()
    }

    /// Routes an event to subscribed actors via the backend.
    pub fn send_event(&self, event: &Event, source: Option<&ActorName>) {
        self.svc.send_event(event, source);
    }

    /// Routes a command to registered actors via the backend.
    pub fn send_command(&self, command: &Command, source: Option<&ActorName>) {
        self.svc.send_command(command, source);
    }

    /// Sends a system message to all actors via the backend.
    pub fn send_system(&self, msg: SystemMessage) {
        self.svc.send_system(msg);
    }

    /// Initiates coordinated shutdown tracking via the backend.
    pub fn begin_shutdown(&self, completion_tx: tokio::sync::oneshot::Sender<()>) {
        self.svc.begin_shutdown(completion_tx);
    }

    /// Shuts down all actors via the backend.
    ///
    /// # Errors
    ///
    /// Returns an error if any actors fail to shut down.
    pub fn shutdown(&self) -> Result<(), Report<ActorHostError>> {
        self.svc.shutdown()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn service_returns_backend_name() {
        // Given a FakeActorHost wrapped in a service.
        let host = Arc::new(crate::common::actor_host::fake::FakeActorHost::new());
        let service = ActorHostService::new(host);

        // When querying the backend name.
        // Then backend returns the host name.
        assert_eq!(service.backend().name(), "FakeActorHost");
    }

    #[rstest::rstest]
    fn service_send_does_not_panic() {
        // Given a FakeActorHost wrapped in a service.
        let host = Arc::new(crate::common::actor_host::fake::FakeActorHost::new());
        let service = ActorHostService::new(host);

        // When sending events, commands, and system messages.
        // Then none of them panic.
        service.send_event(
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
        service.send_command(&Command::RefreshModels, None);
        service.send_system(crate::common::actor::SystemMessage::ApplicationShuttingDown);
    }

    #[rstest::rstest]
    fn service_shutdown_ok() {
        // Given a FakeActorHost wrapped in a service.
        let host = Arc::new(crate::common::actor_host::fake::FakeActorHost::new());
        let service = ActorHostService::new(host);

        // When shutting down.
        // Then shutdown returns Ok.
        service.shutdown().expect("shutdown should succeed");
    }
}
