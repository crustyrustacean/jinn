//! Application core: state, message channel, and processing.
//!
//! [`AppCore`] owns the shared application state and a sender for the
//! internal message channel. The async forwarding task (started by
//! [`spawn_forwarding_task`]) drains the channel and forwards to the
//! actor host.

use std::time::Duration;

use crate::common::bridge::Bridge;
use crate::State;
use crate::SystemMessage;
use kanal::Sender;

use crate::AppMsg;

/// Default timeout for coordinated shutdown.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Default timeout for actor system startup.
pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(3);

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
    /// Bridge for sending typed message closures to the kameo bus.
    pub bridge: Bridge,
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
    pub fn submit_command(&self, cmd: crate::Command) {
        let _ = self.sender.send(AppMsg::Command {
            command: cmd,
            source: None,
        });
    }
}

/// Spawns a background task that continuously drains the `AppMsg` channel
/// and forwards to the actor host.
///
/// No tick dependency - messages are forwarded immediately as they arrive.
/// The task ends when the `receiver` channel is dropped (happens when
/// `AppCore` is dropped).
pub fn spawn_forwarding_task(
    receiver: kanal::AsyncReceiver<AppMsg>,
    actor_host: crate::ActorHostService,
    handle: &tokio::runtime::Handle,
) -> tokio::task::JoinHandle<()> {
    handle.spawn(async move {
        let async_rx = receiver;
        loop {
            let Ok(msg) = async_rx.recv().await else {
                break;
            };
            match msg {
                AppMsg::Command { command, source } => {
                    actor_host.send_command(&command, source.as_ref());
                }
                AppMsg::Event { event, source } => {
                    actor_host.send_event(&event, source.as_ref());
                }
            }
        }
    })
}

/// Runs coordinated shutdown of the actor system.
///
/// 1. Initiates shutdown tracking on the host.
/// 2. Sends `SystemMessage::ApplicationShuttingDown` to all actors.
/// 3. Blocks until the host signals all actors have completed (or timeout).
/// 4. Sets `should_quit` on state.
/// 5. Joins actor tasks via `actor_host.shutdown()`.
///
/// The main thread owns the timeout - if the actor system doesn't signal
/// back within `timeout`, we force quit.
pub fn coordinated_shutdown(
    actor_host: &dyn crate::ActorHost,
    state: &State,
    handle: &tokio::runtime::Handle,
    timeout: Duration,
) {
    // 1. Initiate shutdown tracking.
    let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();
    actor_host.begin_shutdown(completion_tx);

    // 2. Send ApplicationShuttingDown to all actors.
    //    Run loops intercept this, call on_shutdown(), auto-announce,
    //    and signal the host's tracker.
    actor_host.send_system(SystemMessage::ApplicationShuttingDown);

    // 3. Block until all actors complete (with timeout).
    //    Spawns an async task that does proper async recv + timeout,
    //    communicates the result via a oneshot channel, and blocks
    //    the calling thread on blocking_recv().
    let (tx, rx) = tokio::sync::oneshot::channel();
    handle.spawn(async move {
        let result = tokio::time::timeout(timeout, completion_rx).await;
        let _ = tx.send(result);
    });
    match rx.blocking_recv() {
        Ok(Ok(Ok(()))) => {
            tracing::info!("all actors shut down gracefully");
        }
        _ => {
            tracing::warn!(?timeout, "coordinated shutdown timed out, forcing quit");
        }
    }

    // 4. Set should_quit.
    state.write().frontend.should_quit = true;

    // 5. Join actor tasks (close channels + join).
    if let Err(e) = actor_host.shutdown() {
        tracing::error!(err = ?e, "actor host shutdown error");
    }
}

/// Blocks the calling thread until the actor system signals readiness.
///
/// Takes a `tokio::sync::oneshot::Receiver` that the system-ready actor
/// signals when all actors have started. Waits with a [`STARTUP_TIMEOUT`]
/// timeout. Must be called from outside the tokio runtime (e.g., main thread).
///
/// # Panics
///
/// Panics if called from within the tokio runtime context.
///
pub fn wait_for_system_ready(
    ready_rx: tokio::sync::oneshot::Receiver<()>,
    handle: &tokio::runtime::Handle,
) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    handle.spawn(async move {
        let result = tokio::time::timeout(STARTUP_TIMEOUT, ready_rx).await;
        let _ = tx.send(result);
    });
    match rx.blocking_recv() {
        Ok(Ok(Ok(()))) => {
            tracing::info!("actor system ready");
        }
        _ => {
            tracing::error!(
                timeout = ?STARTUP_TIMEOUT,
                "actor system failed to start within timeout"
            );
        }
    }
}
