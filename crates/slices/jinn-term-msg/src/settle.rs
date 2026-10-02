//! Output settle detection: when a terminal's output has stopped changing
//! long enough that the agent can act on it.

use std::time::Duration;

/// Default quiet window: no output for this long ⇒ treat as settled.
pub const DEFAULT_QUIET_MS: u64 = 400;

/// Default hard cap on the settle wait, even with continuous output.
pub const DEFAULT_MAX_WAIT_MS: u64 = 3000;

/// A `Duration` for the default quiet window.
#[must_use]
pub fn default_quiet() -> Duration {
    Duration::from_millis(DEFAULT_QUIET_MS)
}

/// A `Duration` for the default settle cap.
#[must_use]
pub fn default_max_wait() -> Duration {
    Duration::from_millis(DEFAULT_MAX_WAIT_MS)
}

/// Whether the settle condition is met.
///
/// `quiet_for` is how long no output arrived; `waited` is the total time in
/// this settle wait. Settled when either bound is reached.
#[must_use]
pub fn should_settle(
    quiet_for: Duration,
    waited: Duration,
    quiet: Duration,
    cap: Duration,
) -> bool {
    quiet_for >= quiet || waited >= cap
}

/// The deadline for the quiet window, given the last output instant.
///
/// Encapsulated so the actor's select loop and tests share one definition of
/// "quiet deadline" instead of each recomputing it.
#[must_use]
pub fn quiet_deadline(last_output_at: std::time::Instant, quiet: Duration) -> std::time::Instant {
    last_output_at + quiet
}
