//! Actor counter — tracks the total number of actors spawned.
//!
//! Used by the system-ready actor to know how many `ActorStarted`
//! events to expect without hard-coding the count.

use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn new_counter_starts_at_zero() {
        // Given a new counter.
        let counter = ActorCounter::new();

        // Then it reads zero.
        assert_eq!(counter.load(), 0);
    }

    #[rstest::rstest]
    fn default_counter_starts_at_zero() {
        // Given a default counter.
        let counter = ActorCounter::default();

        // Then it reads zero.
        assert_eq!(counter.load(), 0);
    }

    #[rstest::rstest]
    fn increment_adds_one() {
        // Given a counter.
        let counter = ActorCounter::new();

        // When incrementing.
        counter.increment();

        // Then it reads one.
        assert_eq!(counter.load(), 1);
    }

    #[rstest::rstest]
    fn multiple_increments_accumulate() {
        // Given a counter.
        let counter = ActorCounter::new();

        // When incrementing three times.
        counter.increment();
        counter.increment();
        counter.increment();

        // Then it reads three.
        assert_eq!(counter.load(), 3);
    }

    #[rstest::rstest]
    fn clone_shares_state() {
        // Given a counter.
        let counter = ActorCounter::new();

        // When cloning and incrementing the clone.
        let clone = counter.clone();
        clone.increment();

        // Then the original also reads one.
        assert_eq!(counter.load(), 1);
    }
}
