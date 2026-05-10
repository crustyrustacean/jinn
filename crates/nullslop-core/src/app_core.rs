//! Application core: state, message channel, and processing.
//!
//! [`AppCore`] owns the shared application state and a sender for the
//! internal message channel. The async forwarding task (started by
//! [`spawn_forwarding_task`]) drains the channel and forwards to the
//! actor host.

use std::time::Duration;

use kanal::Sender;
use nullslop_actor::SystemMessage;
use nullslop_component::State;

use crate::AppMsg;

/// Default timeout for coordinated shutdown.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Application core: state and message channel.
///
/// Owns the shared state and a sender for the internal [`AppMsg`] channel.
/// The async forwarding task holds the receiver and routes messages to the
/// actor host.
pub struct AppCore {
    /// Shared application state.
    pub state: State,
    /// Sender half of the internal message channel.
    /// The async forwarding task holds the receiver and routes to the actor host.
    pub sender: Sender<AppMsg>,
}

impl std::fmt::Debug for AppCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppCore")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl AppCore {
    /// Returns a sender for submitting messages to the core.
    #[must_use]
    pub fn sender(&self) -> Sender<AppMsg> {
        self.sender.clone()
    }

    /// Submits a command to the core's message channel.
    ///
    /// Convenience method equivalent to
    /// `self.sender().send(AppMsg::Command { command: cmd, source: None })`.
    pub fn submit_command(&self, cmd: nullslop_protocol::Command) {
        let _ = self.sender.send(AppMsg::Command {
            command: cmd,
            source: None,
        });
    }
}

/// Spawns a background task that continuously drains the `AppMsg` channel
/// and forwards to the actor host.
///
/// No tick dependency — messages are forwarded immediately as they arrive.
/// The task ends when the `receiver` channel is dropped (happens when
/// `AppCore` is dropped).
pub fn spawn_forwarding_task(
    receiver: kanal::Receiver<AppMsg>,
    actor_host: nullslop_actor_host::ActorHostService,
    handle: &tokio::runtime::Handle,
) -> tokio::task::JoinHandle<()> {
    handle.spawn(async move {
        let async_rx = receiver.as_async();
        loop {
            match async_rx.recv().await {
                Ok(msg) => match msg {
                    AppMsg::Command { command, source } => {
                        actor_host.send_command(&command, source.as_ref());
                    }
                    AppMsg::Event { event, source } => {
                        actor_host.send_event(&event, source.as_ref());
                    }
                },
                Err(_) => break, // Channel closed
            }
        }
    })
}

/// Runs coordinated shutdown of the actor system.
///
/// 1. Marks shutdown active on the tracker in `state`.
/// 2. Sends `SystemMessage::ApplicationShuttingDown` to all actors.
/// 3. Blocks until `CoreNotification::ShutdownComplete` is received on
///    `core_receiver` (or the channel closes), with the given `timeout`.
/// 4. Joins actor tasks via `actor_host.shutdown()`.
///
/// # FIXME: Race condition with async forwarding task
///
/// The async forwarding task may not have drained all pending `AppMsg` values
/// from the sender channel before `send_system(ApplicationShuttingDown)` reaches
/// actor mailboxes. Commands still in the `AppMsg` channel could arrive at actors
/// *after* `ApplicationShuttingDown`, violating ordering.
///
/// Fix: close the sender (`sender.close()`), then loop `receiver.try_recv()` until
/// empty before sending `ApplicationShuttingDown`. Low priority — the race window
/// is tiny and the current code has the same race (masked by `tick()` being dead).
pub fn coordinated_shutdown(
    actor_host: &dyn nullslop_actor_host::ActorHost,
    state: &State,
    core_receiver: kanal::Receiver<nullslop_protocol::CoreNotification>,
    handle: tokio::runtime::Handle,
    timeout: Duration,
) {
    // 1. Mark shutdown active.
    state.write().shutdown_tracker.begin_shutdown();

    // 2. Send ApplicationShuttingDown to all actors.
    actor_host.send_system(SystemMessage::ApplicationShuttingDown);

    // 3. Block until shutdown complete notification (with timeout).
    //    Spawns an async task that does proper async recv + timeout,
    //    communicates the result via a oneshot channel, and blocks
    //    the calling thread on blocking_recv(). Must be called from
    //    outside the tokio runtime (e.g., main thread or spawn_blocking).
    let (tx, rx) = tokio::sync::oneshot::channel();
    let cloned = core_receiver.clone();
    handle.spawn(async move {
        let async_rx = cloned.as_async();
        let result = tokio::time::timeout(timeout, async_rx.recv()).await;
        let _ = tx.send(result);
    });
    match rx.blocking_recv() {
        Ok(Ok(Ok(_))) => {}
        _ => tracing::warn!(?timeout, "coordinated shutdown timed out"),
    }

    // 4. Join actor tasks.
    if let Err(e) = actor_host.shutdown() {
        tracing::error!(err = ?e, "actor host shutdown error");
    }
}
