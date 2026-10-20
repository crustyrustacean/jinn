//! Deterministic waiting for tests.
//!
//! A test that sleeps for a fixed interval and then asserts is making a bet
//! on how fast the machine is. The bet is wrong in two directions:
//!
//! - On a **slow** host the sleep expires before the work completes, and the
//!   assertion fails for a reason that has nothing to do with the code.
//! - On a **fast** host the sleep outlives the work, and a test that asserts
//!   *nothing happened* passes **vacuously** — it would have passed even if
//!   the code were broken, because the thing it is waiting for had not
//!   arrived yet.
//!
//! The second failure is the dangerous one: it is invisible. A green suite
//! proves nothing about the assertion it never reached.
//!
//! This module provides the two shapes that fix it:
//!
//! - [`await_recorded`] — wait for a *positive* signal (a recorder reaching
//!   a count) rather than for a duration.
//! - [`await_completion_then_assert_absent`] — for assertions about something
//!   that must **not** happen. The completion signal is observed first, so
//!   the absence assertion is only reached once the work is known to be
//!   done, and a missing completion signal fails the test instead of
//!   silently permitting the negative assertion.
#![allow(
    clippy::panic,
    clippy::expect_used,
    reason = "test harness: a failed assertion aborts the test loudly, which is the desired mode"
)]

use std::future::Future;
use std::time::Duration;

use crate::bus_harness::{Recorder, await_recorded};

pub use crate::bus_harness::await_recorded as wait_for_messages;

/// The outcome of a two-phase wait: the completion signal arrived, and the
/// negative assertion may now be evaluated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionSignal {
    /// How many completion messages were observed.
    pub count: usize,
}

/// Waits for a positive completion signal, then evaluates a negative check.
///
/// The `completion` future resolves once the work under test is known to be
/// done. Only then is `assert_absent` called, and its result is returned.
///
/// If the completion signal never arrives within `timeout`, this **fails**:
/// an absence assertion made before the work completed proves nothing, so
/// the test must not report success. The panic message says exactly that.
///
/// # Panics
///
/// Panics when the completion signal does not arrive within `timeout`, or
/// when `assert_absent` returns `false`.
///
/// # Example
///
/// ```
/// # use std::time::Duration;
/// # use jinn_testutil::wait::await_completion_then_assert_absent;
/// # async fn demo() {
/// let ok = await_completion_then_assert_absent(
///     async { 42 },
///     Duration::from_secs(1),
///     || false,
/// )
/// .await;
/// assert!(ok);
/// # }
/// ```
pub async fn await_completion_then_assert_absent<F, Fut>(
    completion: Fut,
    timeout: Duration,
    assert_absent: F,
) -> bool
where
    F: FnOnce() -> bool,
    Fut: Future<Output = usize>,
{
    let Ok(count) = tokio::time::timeout(timeout, completion).await else {
        panic!(
            "the completion signal never arrived within {timeout:?}; \
             an absence assertion made before the work completed proves nothing, \
             so this test fails rather than passing vacuously"
        );
    };
    let _ = CompletionSignal { count };
    assert_absent()
}

/// Reads a text file, normalizing CRLF line endings to LF.
///
/// A Windows checkout with `core.autocrlf=true` delivers CRLF line endings,
/// and `include_str!` embeds the *checked-out* bytes — so a fixture written
/// with `\n` arrives with `\r\n`, and an assertion about a specific line
/// fails. This reads the file and collapses the carriage returns, so the
/// assertion sees the same bytes on every platform.
///
/// # Errors
///
/// Returns the underlying `std::fs::read_to_string` error.
pub fn read_normalized(path: &std::path::Path) -> std::io::Result<String> {
    let content = std::fs::read_to_string(path)?;
    Ok(content.replace("\r\n", "\n"))
}

/// Polls `predicate` until it holds, or `timeout` expires.
///
/// The polling counterpart to [`await_recorded`] for state that lives in a
/// cell rather than on the bus. Returns whether the predicate held.
///
/// # Example
///
/// ```
/// # use std::time::Duration;
/// let done = jinn_testutil::wait::poll_until(Duration::from_secs(1), || true);
/// assert!(done);
/// ```
#[must_use]
pub fn poll_until<F>(timeout: Duration, mut predicate: F) -> bool
where
    F: FnMut() -> bool,
{
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if predicate() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The async counterpart of [`poll_until`].
///
/// # Example
///
/// ```
/// # use std::time::Duration;
/// # async fn demo() {
/// let done = jinn_testutil::wait::poll_until_async(Duration::from_secs(1), || true).await;
/// assert!(done);
/// # }
/// ```
pub async fn poll_until_async<F>(timeout: Duration, mut predicate: F) -> bool
where
    F: FnMut() -> bool,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if predicate() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Waits until a synchronous check holds, or the timeout expires.
///
/// The panicking counterpart of [`poll_until_async`]: a test that waits
/// for a condition and then asserts on it wants the wait to *fail* the
/// test when the condition never holds, not to return a flag the test
/// might forget to check.
///
/// # Panics
///
/// Panics when the check never holds within `timeout`.
///
/// # Example
///
/// ```
/// # use std::time::Duration;
/// # async fn demo() {
/// jinn_testutil::wait::wait_for(Duration::from_secs(2), || true).await;
/// # }
/// ```
pub async fn wait_for<F>(timeout: Duration, mut check: F)
where
    F: FnMut() -> bool,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if check() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the condition never held within {timeout:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Polls an async check until it holds, or the timeout expires.
///
/// The async-predicate counterpart of [`poll_until_async`], for a
/// condition that must await something to evaluate.
///
/// # Example
///
/// ```
/// # use std::time::Duration;
/// # async fn demo() {
/// let done = jinn_testutil::wait::poll_until_async_fn(Duration::from_secs(1), || async { true })
///     .await;
/// assert!(done);
/// # }
/// ```
pub async fn poll_until_async_fn<F, Fut>(timeout: Duration, mut condition: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if condition().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Waits until an actor's inbox is empty.
///
/// An actor's inbox is empty only when everything published to it has been
/// claimed and dispatched. So once the inbox drains, the work published
/// before the wait has been *processed* — which is what makes a
/// subsequent "nothing else happened" assertion meaningful rather than
/// vacuous.
///
/// This is the barrier for actors that do not handle
/// [`Barrier`](crate::barrier::Barrier) and cannot be changed to: it
/// observes the actor's own queue rather than adding a message it would
/// have to answer.
///
/// The wait has a floor of one poll interval, so a fast actor cannot
/// drain between the publish and the first check and make the wait
/// vacuous again.
///
/// # Panics
///
/// Panics when the inbox has not drained within `timeout`.
///
/// # Example
///
/// ```
/// # use std::time::Duration;
/// # use trouper::actor::ActorPath;
/// # use jinn_testutil::wait::await_inbox_drained;
/// # async fn demo(system: &trouper::system::ActorSystem, path: &ActorPath) {
/// // ... publish the work under test ...
/// await_inbox_drained(system, path, Duration::from_secs(5)).await;
/// // ... now "nothing else happened" is a real observation ...
/// # }
/// ```
pub async fn await_inbox_drained(
    system: &trouper::system::ActorSystem,
    path: &trouper::actor::ActorPath,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    // The floor guarantees at least one full poll interval passes, so a
    // message that is published but not yet delivered is still counted.
    let floor = tokio::time::Instant::now() + Duration::from_millis(20);
    loop {
        let queued = system.inbox_debug_len(path).await;
        let now = tokio::time::Instant::now();
        if queued == 0 && now >= floor {
            return;
        }
        assert!(
            now < deadline,
            "the actor at {path} still has {queued} message(s) queued after \
             {timeout:?}; the work under test was never processed, so any \
             assertion made now would prove nothing"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Waits until a polled value stops changing.
///
/// The state-quiescence counterpart of [`await_settled`], for the case
/// where the observable is application state rather than a bus message.
/// A test that reads a state field after a fixed sleep can read it before
/// the actor wrote it — and an *absence* read that way proves nothing,
/// because "the state has not changed yet" is indistinguishable from "the
/// state will not change".
///
/// The wait returns when the polled value has been identical for `quiet`,
/// so a burst of writes that is still arriving does not look settled.
///
/// # Panics
///
/// Panics when the value never settles within `timeout`.
///
/// # Example
///
/// ```
/// # use std::time::Duration;
/// # use jinn_testutil::wait::await_value_settled;
/// # async fn demo(phase: impl FnMut() -> String) {
/// let phase = await_value_settled(
///     Duration::from_millis(100),
///     Duration::from_secs(5),
///     phase,
/// )
/// .await;
/// // ... the phase is now stable ...
/// assert!(!phase.is_empty());
/// # }
/// ```
pub async fn await_value_settled<T, F>(quiet: Duration, timeout: Duration, mut poll: F) -> T
where
    T: PartialEq + Clone,
    F: FnMut() -> T,
{
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last = poll();
    let mut last_change = tokio::time::Instant::now();
    loop {
        tokio::time::sleep(Duration::from_millis(10)).await;
        let current = poll();
        let now = tokio::time::Instant::now();
        if current != last {
            last = current;
            last_change = now;
        } else if now - last_change >= quiet {
            return current;
        } else {
            // Still changing, or not yet quiet. Keep polling.
        }
        assert!(
            now < deadline,
            "the polled value never settled within {timeout:?}"
        );
    }
}

/// Waits until a recorder's message count stops changing, then drains it.
///
/// The message-returning counterpart of [`await_settled`]. The drain
/// happens *after* the count has settled, so the returned messages are
/// every message that arrived during the settling window — which is what
/// an "absence" assertion needs: the recorder must have stopped
/// changing before "nothing arrived" is an observation rather than a race
/// with a dispatch still in flight.
///
/// The drain empties the recorder, so a caller that wants the earlier
/// messages too must collect them before calling this.
///
/// # Panics
///
/// Panics when the recorder has not settled within `timeout`.
///
/// # Example
///
/// ```
/// # use std::time::Duration;
/// # use jinn_testutil::bus_harness::Recorder;
/// # use jinn_testutil::wait::await_settled_messages;
/// # async fn demo(recorder: &Recorder<String>) {
/// let arrived = await_settled_messages(
///     recorder,
///     Duration::from_millis(100),
///     Duration::from_secs(5),
/// )
/// .await;
/// assert!(arrived.is_empty());
/// # }
/// ```
pub async fn await_settled_messages<M: Clone + Send + 'static>(
    recorder: &Recorder<M>,
    quiet: Duration,
    timeout: Duration,
) -> Vec<M> {
    await_settled(recorder, quiet, timeout).await;
    recorder.drain()
}

/// Polls a cell until a predicate over its value holds.
///
/// The cell-poll counterpart of [`poll_until_async`], for the case where
/// the observable is state a test reads back rather than a message on the
/// bus. A test that reads a cell after a fixed sleep has the same defect
/// as one that reads a bus recorder after a fixed sleep: the read may
/// happen before the write, and the assertion then proves nothing.
///
/// # Panics
///
/// Panics when the predicate has not held within `timeout`.
///
/// # Example
///
/// ```
/// # use std::time::Duration;
/// # use jinn_slices::cell::TypedCell;
/// # use jinn_testutil::wait::await_cell;
/// # async fn demo(cell: &TypedCell<u32>) {
/// await_cell(cell, Duration::from_secs(5), |value| *value > 0).await;
/// // ... the cell now holds a value the predicate accepted ...
/// # }
/// ```
pub async fn await_cell<T, F>(
    cell: &jinn_slices::cell::TypedCell<T>,
    timeout: Duration,
    predicate: F,
) where
    T: Send + Sync + 'static,
    F: Fn(&T) -> bool,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if predicate(&cell.read()) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the cell never satisfied its predicate within {timeout:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Waits until a recorder's message count stops changing.
///
/// A quiescence wait, for the case where the *number* of messages is the
/// assertion but the count is not known in advance — a spawn counter, a
/// set of lifecycle events. The wait returns when the recorder has held the
/// same count for `quiet` (so a burst that is still arriving does not look
/// settled), or when `timeout` expires.
///
/// A quiescence wait cannot prove an *absence* on its own: "no message
/// arrived" is indistinguishable from "the message has not arrived yet".
/// Use it when the assertion is about a count that should have reached a
/// known value, and pair it with a positive wait when the assertion is
/// about a message that must not arrive.
///
/// # Panics
///
/// Panics when the recorder has not settled within `timeout`.
///
/// # Example
///
/// ```
/// # use std::time::Duration;
/// # use jinn_testutil::bus_harness::Recorder;
/// # use jinn_testutil::wait::await_settled;
/// # async fn demo(recorder: &Recorder<String>) {
/// await_settled(recorder, Duration::from_millis(100), Duration::from_secs(5)).await;
/// // ... the count is now stable ...
/// # }
/// ```
pub async fn await_settled<M: Clone + Send + 'static>(
    recorder: &Recorder<M>,
    quiet: Duration,
    timeout: Duration,
) -> usize {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last_count = recorder.len();
    let mut last_change = tokio::time::Instant::now();
    loop {
        tokio::time::sleep(Duration::from_millis(10)).await;
        let count = recorder.len();
        let now = tokio::time::Instant::now();
        if count != last_count {
            last_count = count;
            last_change = now;
        } else if now - last_change >= quiet {
            return count;
        } else {
            // Still changing, or not yet quiet. Keep polling.
        }
        assert!(
            now < deadline,
            "the recorder never settled within {timeout:?} (last count {last_count}, \
             still changing every {quiet:?}); the work under test is not finishing"
        );
    }
}

/// Awaits `min_count` messages on `recorder`, or the timeout expires.
///
/// A convenience re-export of [`crate::bus_harness::await_recorded`] so
/// waiting helpers are discoverable from one module.
pub async fn await_messages<M: Clone + Send + 'static>(
    recorder: &Recorder<M>,
    min_count: usize,
    timeout: Duration,
) -> Vec<M> {
    await_recorded(recorder, min_count, timeout).await
}

/// Publishes a [`Barrier`](crate::barrier::Barrier) and waits for its
/// acknowledgement.
///
/// The barrier is published *after* the work under test. Because an actor
/// dispatches its inbox in FIFO order, the acknowledgement proves the work
/// under test has already been processed — which is what makes a
/// subsequent "nothing else happened" assertion meaningful rather than
/// vacuous.
///
/// # Panics
///
/// Panics when the acknowledgement does not arrive within `timeout`. A
/// barrier that never comes back means the actor under test never drained
/// its inbox, so the test must fail rather than proceed to an assertion
/// that would prove nothing.
///
/// # Example
///
/// ```
/// # use std::time::Duration;
/// # use jinn_testutil::barrier::BarrierAck;
/// # use jinn_testutil::bus_harness::{Recorder, TestHarness};
/// # use jinn_testutil::wait::await_barrier;
/// # async fn demo(harness: &TestHarness, acks: &Recorder<BarrierAck>) {
/// // ... publish the work under test ...
/// await_barrier(harness, acks, "after-work", Duration::from_secs(5)).await;
/// // ... now "nothing else happened" is a real observation ...
/// # }
/// ```
pub async fn await_barrier(
    harness: &crate::bus_harness::TestHarness,
    acks: &Recorder<crate::barrier::BarrierAck>,
    label: &str,
    timeout: Duration,
) {
    harness.publish(crate::barrier::Barrier::new(label)).await;
    let received = await_recorded(acks, 1, timeout).await;
    assert!(
        received.iter().any(|ack| ack.label == label),
        "the barrier {label:?} was never acknowledged within {timeout:?}; \
         the actor under test never drained its inbox, so any assertion made \
         now would prove nothing"
    );
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        await_completion_then_assert_absent, poll_until, poll_until_async, read_normalized,
    };

    #[rstest::rstest]
    #[tokio::test]
    async fn completion_then_absent_reports_the_absence() {
        // Given a completion signal that arrives, and nothing that should follow.
        // When waiting for completion and then checking for absence.
        let absent =
            await_completion_then_assert_absent(async { 1 }, Duration::from_secs(1), || true).await;

        // Then the absence is reported.
        assert!(absent);
    }

    #[rstest::rstest]
    #[tokio::test]
    #[should_panic(expected = "the completion signal never arrived")]
    async fn completion_then_absent_fails_when_the_completion_never_arrives() {
        // Given a completion signal that never arrives.
        // When waiting for completion and then checking for absence.
        let _ = await_completion_then_assert_absent(
            std::future::pending::<usize>(),
            Duration::from_millis(50),
            || true,
        )
        .await;

        // Then the wait fails, rather than permitting the negative assertion.
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn completion_then_absent_evaluates_the_check_after_completion() {
        // Given a completion signal and a check that records when it ran.
        let ran_after_completion = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&ran_after_completion);

        // When waiting for completion and then checking for absence.
        await_completion_then_assert_absent(async { 7 }, Duration::from_secs(1), move || {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            true
        })
        .await;

        // Then the check ran.
        assert!(ran_after_completion.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[rstest::rstest]
    #[test]
    fn read_normalized_collapses_crlf_to_lf() {
        // Given a file written with CRLF line endings.
        let dir = crate::temp::tempdir().expect("temp dir");
        let path = dir.path().join("crlf.txt");
        std::fs::write(&path, b"one\r\ntwo\r\n").expect("write file");

        // When reading it through the normalizer.
        let content = read_normalized(&path).expect("read file");

        // Then no carriage return survives.
        assert!(!content.contains('\r'), "got {content:?}");
        assert_eq!(content, "one\ntwo\n");
    }

    #[rstest::rstest]
    #[test]
    fn read_normalized_leaves_lf_content_unchanged() {
        // Given a file written with LF line endings.
        let dir = crate::temp::tempdir().expect("temp dir");
        let path = dir.path().join("lf.txt");
        std::fs::write(&path, b"one\ntwo\n").expect("write file");

        // When reading it through the normalizer.
        let content = read_normalized(&path).expect("read file");

        // Then the content is unchanged.
        assert_eq!(content, "one\ntwo\n");
    }

    #[rstest::rstest]
    #[test]
    fn poll_until_reports_a_predicate_that_holds() {
        // Given a predicate that holds immediately.
        // When polling for it.
        let held = poll_until(Duration::from_secs(1), || true);

        // Then the poll succeeds.
        assert!(held);
    }

    #[rstest::rstest]
    #[test]
    fn poll_until_reports_a_predicate_that_never_holds() {
        // Given a predicate that never holds.
        // When polling for it.
        let held = poll_until(Duration::from_millis(50), || false);

        // Then the poll reports failure.
        assert!(!held);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn poll_until_async_reports_a_predicate_that_holds() {
        // Given a predicate that holds immediately.
        // When polling for it.
        let held = poll_until_async(Duration::from_secs(1), || true).await;

        // Then the poll succeeds.
        assert!(held);
    }
}
