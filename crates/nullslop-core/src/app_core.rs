//! Application core: state, message channel, and processing.
//!
//! [`AppCore`] owns the processing pipeline — shared state, an internal
//! message channel for [`AppMsg`], and an optional actor host.
//!
//! Phase 7: The bus has been deleted. An async forwarding task continuously
//! drains the `AppMsg` channel and forwards directly to the actor host.
//! The main loop is input + rendering only — no tick call.

use std::time::{Duration, Instant};

use kanal::{Receiver, Sender};
use nullslop_actor::SystemMessage;
use nullslop_actor_host::ActorHostService;
use nullslop_component::State;
use nullslop_services::CoreNotification;

use crate::AppMsg;

/// How long to wait between ticks during coordinated shutdown.
const SHUTDOWN_TICK_INTERVAL: Duration = Duration::from_millis(50);

/// Default timeout for coordinated shutdown.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Result of a [`AppCore::tick`] call.
///
/// Kept for backward compatibility with callers that check `should_quit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickResult {
    /// The application has requested to quit.
    pub should_quit: bool,
    /// At least one message was processed or the bus had pending work.
    pub did_work: bool,
}

/// Application core: state, message channel, and processing.
///
/// Owns the processing pipeline. The caller feeds [`AppMsg`] values
/// via [`Self::sender`] and the async forwarding task handles routing
/// to the actor host.
pub struct AppCore {
    /// Shared application state.
    pub state: State,
    /// Sender half of the internal message channel.
    pub sender: Sender<AppMsg>,
    /// Receiver half of the internal message channel.
    ///
    /// Usually consumed by [`spawn_forwarding_task`]. Kept here so
    /// callers that don't use the async task can drain manually.
    pub receiver: Receiver<AppMsg>,
    /// Optional actor host for forwarding processed messages.
    pub actor_host: Option<ActorHostService>,
    /// Receiver for core lifecycle notifications (e.g. shutdown complete).
    ///
    /// Set during startup wiring. Used by [`coordinated_shutdown`](Self::coordinated_shutdown)
    /// to block until the actor system signals completion, replacing sleep-polling.
    pub core_receiver: Option<Receiver<CoreNotification>>,
}

impl std::fmt::Debug for AppCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppCore")
            .field("state", &self.state)
            .field("actor_host", &self.actor_host)
            .finish_non_exhaustive()
    }
}

impl AppCore {
    /// Creates a new `AppCore` with default state and empty channel.
    #[must_use]
    pub fn new() -> Self {
        let (sender, receiver) = kanal::unbounded();
        Self {
            state: State::new(nullslop_component::AppState::default()),
            sender,
            receiver,
            actor_host: None,
            core_receiver: None,
        }
    }

    /// Returns a sender for submitting messages to the core.
    #[must_use]
    pub fn sender(&self) -> Sender<AppMsg> {
        self.sender.clone()
    }

    /// Returns a reference to the actor host, if set.
    #[must_use]
    pub fn actor_host(&self) -> Option<&ActorHostService> {
        self.actor_host.as_ref()
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

    /// Processes one batch of pending messages.
    ///
    /// Drains all available [`AppMsg`] values from the internal channel
    /// and forwards them directly to the actor host.
    ///
    /// Returns a [`TickResult`] indicating whether quit was requested and
    /// whether any work was performed.
    ///
    /// **Note:** This is kept for backward compatibility with the headless
    /// runner and coordinated shutdown. The main TUI loop no longer calls
    /// this — the async forwarding task handles message routing instead.
    pub fn tick(&mut self) -> TickResult {
        let mut received_messages = false;

        while let Ok(Some(msg)) = self.receiver.try_recv() {
            received_messages = true;
            match msg {
                AppMsg::Command { command, source } => {
                    if let Some(host) = &self.actor_host {
                        host.send_command(&command, source.as_ref());
                    }
                }
                AppMsg::Event { event, source } => {
                    if let Some(host) = &self.actor_host {
                        host.send_event(&event, source.as_ref());
                    }
                }
            }
        }

        TickResult {
            should_quit: self.state.read().should_quit,
            did_work: received_messages,
        }
    }

    /// Runs coordinated shutdown of the actor system.
    ///
    /// 1. Marks shutdown active on the tracker.
    /// 2. Sends `SystemMessage::ApplicationShuttingDown` to all actors.
    /// 3. If a [`core_receiver`](Self::core_receiver) is available, blocks until
    ///    `CoreNotification::ShutdownComplete` is received or the timeout expires.
    ///    Otherwise, falls back to tick-loop polling.
    /// 4. Joins actor tasks via the host.
    ///
    /// Pass the default timeout with [`SHUTDOWN_TIMEOUT`] or a custom duration.
    pub fn coordinated_shutdown(
        &mut self,
        actor_host: &dyn nullslop_actor_host::ActorHost,
        timeout: Duration,
    ) {
        // 1. Mark shutdown active.
        self.state.write().shutdown_tracker.begin_shutdown();

        // 2. Send ApplicationShuttingDown to all actors.
        actor_host.send_system(SystemMessage::ApplicationShuttingDown);

        // 3. Wait for shutdown completion via notification channel.
        if let Some(ref core_rx) = self.core_receiver {
            let cloned = core_rx.clone();
            let async_rx = cloned.as_async();
            let _ = async_rx.recv(); // Block until ShutdownComplete or channel closes
        } else {
            // Fallback: tick loop polling (backward compat for tests without core_receiver).
            let start = Instant::now();
            loop {
                self.tick();
                if self.state.read().shutdown_tracker.is_complete() {
                    break;
                }
                if start.elapsed() > timeout {
                    break;
                }
                std::thread::sleep(SHUTDOWN_TICK_INTERVAL);
            }
        }

        // 4. Join actor tasks.
        if let Err(e) = actor_host.shutdown() {
            tracing::error!(err = ?e, "actor host shutdown error");
        }
    }
}

impl Default for AppCore {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawns a background task that continuously drains the `AppMsg` channel
/// and forwards to the actor host.
///
/// No tick dependency — messages are forwarded immediately as they arrive.
/// The task ends when the `receiver` channel is dropped (happens when
/// `AppCore` is dropped).
pub fn spawn_forwarding_task(
    receiver: Receiver<AppMsg>,
    actor_host: ActorHostService,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn tick_returns_no_work_when_idle() {
        // Given an AppCore with no messages.
        let mut core = AppCore::new();

        // When ticking with no messages.
        let result = core.tick();

        // Then returns false for did_work.
        assert!(!result.should_quit);
        assert!(!result.did_work);
    }

    #[rstest::rstest]
    fn submit_command_records_through_channel() {
        // Given an AppCore.
        let mut core = AppCore::new();

        // When submitting a command and ticking.
        core.submit_command(nullslop_protocol::Command::RefreshModels);
        let result = core.tick();

        // Then work was done.
        assert!(result.did_work);
    }

    #[rstest::rstest]
    fn processed_command_forwarded_to_actor_host() {
        // Given an AppCore with a fake actor host.
        use nullslop_actor_host::FakeActorHost;
        let mut core = AppCore::new();
        let host = std::sync::Arc::new(FakeActorHost::new());
        core.actor_host = Some(nullslop_actor_host::ActorHostService::new(host.clone()));

        // When submitting a command and ticking.
        core.submit_command(nullslop_protocol::Command::RefreshModels);
        core.tick();

        // Then the command was forwarded to the actor host.
        let sent = host.commands_sent();
        assert_eq!(sent.len(), 1);
        assert!(matches!(
            sent[0],
            nullslop_protocol::Command::RefreshModels
        ));
    }
}
