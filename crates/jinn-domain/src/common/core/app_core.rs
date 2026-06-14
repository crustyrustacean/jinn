//! Application core: shared state and the kanal bridge.
//!
//! [`AppCore`] owns the shared application state and the [`Bridge`] whose
//! drain task publishes closures to the kameo bus.

use std::time::Duration;

use crate::State;
use crate::common::bridge::Bridge;

/// Default timeout for coordinated shutdown.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Default timeout for actor system startup.
pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(3);

/// Application core: shared state and the bridge to the actor system.
///
/// Owns the shared state and the [`Bridge`] whose drain task publishes
/// closures to the kameo bus.
pub struct AppCore {
    /// Shared application state.
    pub state: State,
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
