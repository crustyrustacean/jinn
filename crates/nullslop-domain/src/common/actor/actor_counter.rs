//! Actor counter — tracks the total number of actors spawned.
//!
//! Used by the system-ready actor to know how many `ActorStarted`
//! events to expect without hard-coding the count.

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

/// Tracks the total number of actors spawned in the system.
///
/// Incremented atomically by `spawn` and `system_spawn`. The system-ready
/// actor reads the final count to determine when all actors have started.
#[derive(Debug, Clone)]
pub struct ActorCounter {
    inner: Arc<AtomicU16>,
}

impl ActorCounter {
    /// Creates a new counter starting at zero.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AtomicU16::new(0)),
        }
    }

    /// Increments the counter by one.
    pub fn increment(&self) {
        self.inner.fetch_add(1, Ordering::Release);
    }

    /// Returns the current count.
    #[must_use]
    pub fn load(&self) -> u16 {
        self.inner.load(Ordering::Acquire)
    }
}

impl Default for ActorCounter {
    fn default() -> Self {
        Self::new()
    }
}
