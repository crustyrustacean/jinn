#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

use std::time::Duration;

use super::*;
use crate::common::app_state::AppState;
use crate::common::bus::test_harness::{TestHarness, await_recorded};
use crate::feat::provider::protocol::command::CancelStream;
use crate::protocol::{ChatEntry, SessionId};

/// Test harness: spawns the watchdog with a `scan_once` handle, recording bus,
/// and preferences configured for a tiny stall window so scans trip quickly.
struct WatchdogHarness {
    watchdog: StallWatchdogActor,
    state: State,
    session_id: SessionId,
    deps_storage: crate::common::actor_deps::ActorDeps,
    retry_recorder:
        kameo::actor::ActorRef<crate::common::bus::test_harness::Recorder<RetryStalledSession>>,
    cancel_recorder:
        kameo::actor::ActorRef<crate::common::bus::test_harness::Recorder<CancelStream>>,
}

impl WatchdogHarness {
    async fn new() -> Self {
        let harness = TestHarness::new().await;
        let deps = harness.actor_deps().await;

        // Tight stall window + small budget so scans trigger in milliseconds.
        {
            let mut prefs = deps.services.user_preferences_storage.read();
            prefs.history_stall_timeout_secs = 1;
            prefs.stall_retry_max_retries = 1;
            deps.services
                .user_preferences_storage
                .save(&prefs)
                .expect("save prefs");
        }

        let state = State::new(AppState::default());
        let session_id = {
            let s = state.read();
            s.session.active_session_id().clone()
        };

        let watchdog = StallWatchdogActor::for_test(deps.clone(), state.clone());

        let retry_recorder = harness.spawn_recorder::<RetryStalledSession>().await;
        let cancel_recorder = harness.spawn_recorder::<CancelStream>().await;

        Self {
            watchdog,
            state,
            session_id,
            deps_storage: deps,
            retry_recorder,
            cancel_recorder,
        }
    }

    /// Override the stall-retry backoff prefs (and raise the budget so a
    /// second retry is permitted within the same turn).
    fn set_backoff(&self, base_secs: u64, max_secs: u64) {
        let mut prefs = self.deps_storage.services.user_preferences_storage.read();
        prefs.stall_retry_base_delay_secs = base_secs;
        prefs.stall_retry_max_delay_secs = max_secs;
        prefs.stall_retry_max_retries = 2;
        self.deps_storage
            .services
            .user_preferences_storage
            .save(&prefs)
            .expect("save prefs");
    }
}

#[tokio::test]
async fn watchdog_detects_session_stuck_in_sending() {
    // Given a session in Sending whose history was last mutated long ago.
    let mut wh = WatchdogHarness::new().await;
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.begin_sending();
        // Force the activity timestamp into the distant past so the 1s window
        // is exceeded immediately.
        session.core.last_history_activity_at = Timestamp::now()
            .checked_sub(Duration::from_mins(1))
            .unwrap();
    }

    // When the watchdog scans.
    wh.watchdog.scan_once().await;
    // Allow the async publish to land.
    let retries = await_recorded(&wh.retry_recorder, 1, Duration::from_millis(500)).await;

    // Then a RetryStalledSession command was published.
    assert!(
        retries.iter().any(|r| r.session_id == wh.session_id),
        "expected RetryStalledSession for a Sending session with stale activity"
    );
}

#[tokio::test]
async fn watchdog_detects_streaming_session_with_no_history_change() {
    // Given a session in Streaming whose history is stale (keepalive-only feed).
    let mut wh = WatchdogHarness::new().await;
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.push_entry(ChatEntry::user("go"));
        session.begin_streaming();
        session.core.last_history_activity_at = Timestamp::now()
            .checked_sub(Duration::from_mins(1))
            .unwrap();
    }

    // When the watchdog scans.
    wh.watchdog.scan_once().await;
    let retries = await_recorded(&wh.retry_recorder, 1, Duration::from_millis(500)).await;

    // Then a retry was requested.
    assert!(
        retries.iter().any(|r| r.session_id == wh.session_id),
        "expected retry for Streaming session with stale activity"
    );
}

#[tokio::test]
async fn watchdog_publishes_cancel_after_budget_exhausted() {
    // Given a stalled session and a budget of one retry.
    let mut wh = WatchdogHarness::new().await;
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.begin_sending();
        session.core.last_history_activity_at = Timestamp::now()
            .checked_sub(Duration::from_mins(1))
            .unwrap();
    }

    // First scan: within budget → retry.
    wh.watchdog.scan_once().await;
    let _ = await_recorded(&wh.retry_recorder, 1, Duration::from_millis(500)).await;

    // Force staleness again so the retry's own re-seeding does not satisfy the
    // next tick (the floor mechanism records the post-retry timestamp).
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.core.last_history_activity_at = Timestamp::now()
            .checked_sub(Duration::from_mins(1))
            .unwrap();
    }

    // Second scan: budget exhausted → cancel.
    wh.watchdog.scan_once().await;
    let cancels = await_recorded(&wh.cancel_recorder, 1, Duration::from_millis(500)).await;

    assert!(
        cancels.iter().any(|c| c.session_id == wh.session_id),
        "expected CancelStream once stall-retry budget is exhausted"
    );
}

#[tokio::test]
async fn counter_resets_at_turn_boundary_not_on_activity_jitter() {
    // Given a stalled session that already consumed one retry (budget = 1).
    let mut wh = WatchdogHarness::new().await;
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.begin_sending();
        session.core.last_history_activity_at = Timestamp::now()
            .checked_sub(Duration::from_mins(1))
            .unwrap();
    }
    wh.watchdog.scan_once().await;
    let first = await_recorded(&wh.retry_recorder, 1, Duration::from_millis(500)).await;
    assert_eq!(first.len(), 1, "first stall should retry within budget");

    // When the retry re-seeds the timestamp (the handler always does) but the
    // session STAYS stalled (perpetually hung provider) — the re-seed must NOT
    // reset the budget. Force staleness again.
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.core.last_history_activity_at = Timestamp::now()
            .checked_sub(Duration::from_mins(1))
            .unwrap();
    }
    wh.watchdog.scan_once().await;
    let cancels = await_recorded(&wh.cancel_recorder, 1, Duration::from_millis(500)).await;

    // Then the budget is exhausted → CancelStream (not another retry).
    assert!(
        cancels.iter().any(|c| c.session_id == wh.session_id),
        "a perpetually hung session must be canceled — re-seeding must not grant infinite retries"
    );
    let extra_retries = await_recorded(&wh.retry_recorder, 2, Duration::from_millis(100)).await;
    assert!(
        extra_retries.len() <= 1,
        "must not retry again once the budget is exhausted"
    );
}

#[tokio::test]
async fn counter_resets_when_session_completes_a_turn() {
    // Given a session that stalled, was retried, then completed the turn (Idle).
    let mut wh = WatchdogHarness::new().await;
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.begin_sending();
        session.core.last_history_activity_at = Timestamp::now()
            .checked_sub(Duration::from_mins(1))
            .unwrap();
    }
    wh.watchdog.scan_once().await; // retry 1 (message left on the recorder)

    // The retried turn actually completes → session returns to Idle.
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        let _ = session.core.ephemeral.machine.cancel();
    }
    wh.watchdog.scan_once().await; // observes the turn boundary, clears budget

    // When a NEW turn starts and stalls.
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.begin_sending();
        session.core.last_history_activity_at = Timestamp::now()
            .checked_sub(Duration::from_mins(1))
            .unwrap();
    }
    wh.watchdog.scan_once().await;
    let retries = await_recorded(&wh.retry_recorder, 2, Duration::from_millis(500)).await;
    let retries_for_session: Vec<_> = retries
        .into_iter()
        .filter(|r| r.session_id == wh.session_id)
        .collect();

    // Then the new stall is treated as a fresh attempt: exactly 2 total retries
    // (one per turn), not a cancel — because the budget reset at the turn boundary.
    assert_eq!(
        retries_for_session.len(),
        2,
        "budget must reset at a turn boundary so a fresh stall in a new turn gets a full budget"
    );
    let cancels = await_recorded(&wh.cancel_recorder, 1, Duration::from_millis(100)).await;
    assert!(
        cancels.iter().all(|c| c.session_id != wh.session_id),
        "must not cancel when each turn stays within its own budget"
    );
}

#[tokio::test]
async fn active_streaming_session_is_never_flagged() {
    // Given a Streaming session whose history was just mutated.
    let mut wh = WatchdogHarness::new().await;
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.push_entry(ChatEntry::user("go"));
        session.begin_streaming();
        session.core.last_history_activity_at = Timestamp::now();
    }

    // When the watchdog scans.
    wh.watchdog.scan_once().await;
    let retries = await_recorded(&wh.retry_recorder, 0, Duration::from_millis(100)).await;

    // Then no retry is published.
    assert!(
        retries.iter().all(|r| r.session_id != wh.session_id),
        "active streaming session must not be flagged"
    );
}

#[tokio::test]
async fn idle_session_is_never_scanned() {
    // Given an idle session with a very stale timestamp.
    let mut wh = WatchdogHarness::new().await;
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.core.last_history_activity_at = Timestamp::now()
            .checked_sub(Duration::from_mins(1))
            .unwrap();
    }

    // When the watchdog scans.
    wh.watchdog.scan_once().await;
    let retries = await_recorded(&wh.retry_recorder, 0, Duration::from_millis(100)).await;

    // Then no retry is published for the idle session.
    assert!(
        retries.iter().all(|r| r.session_id != wh.session_id),
        "idle sessions must never be flagged"
    );
}

/// A session stalled mid tool batch (no tool results landing in history) is
/// indistinguishable from a keepalive-only stall at the history-signal level:
/// the phase is `Streaming` and `last_history_activity_at` is stale. This test
/// anchors that the tool-batch gap is covered by the same single signal.
#[tokio::test]
async fn watchdog_detects_mid_tool_batch_stall() {
    // Given a session mid tool batch: it entered Streaming, a tool-call entry
    // was pushed, but no tool result ever lands and the timestamp has aged.
    let mut wh = WatchdogHarness::new().await;
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.begin_streaming();
        session.push_entry(ChatEntry::system("tool call pending"));
        // Simulate the tool batch stalling: no further history activity.
        session.core.last_history_activity_at = Timestamp::now()
            .checked_sub(Duration::from_mins(1))
            .unwrap();
    }

    // When the watchdog scans.
    wh.watchdog.scan_once().await;
    let retries = await_recorded(&wh.retry_recorder, 1, Duration::from_millis(500)).await;

    // Then the mid-tool-batch stall is detected.
    assert!(
        retries.iter().any(|r| r.session_id == wh.session_id),
        "a session stalled mid tool batch must be detected"
    );
}

#[tokio::test]
async fn watchdog_suppresses_second_retry_within_backoff_window() {
    // Given a stalled session and a large backoff window (base=max=60s, so the
    // delay is exactly 60s).
    let mut wh = WatchdogHarness::new().await;
    wh.set_backoff(60, 60);
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.begin_sending();
        session.core.last_history_activity_at = Timestamp::now()
            .checked_sub(Duration::from_mins(1))
            .unwrap();
    }

    // First scan: no prior retry → publish RetryStalledSession immediately.
    wh.watchdog.scan_once().await;
    let first = await_recorded(&wh.retry_recorder, 1, Duration::from_millis(500)).await;
    assert_eq!(first.len(), 1, "first stall should retry");

    // Force staleness again (the retry re-seeds the timestamp).
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.core.last_history_activity_at = Timestamp::now()
            .checked_sub(Duration::from_mins(1))
            .unwrap();
    }

    // Second scan immediately after: within backoff window → suppressed.
    // The first `await_recorded` drained the recorder, so any new publish
    // would appear as a non-empty Vec here.
    wh.watchdog.scan_once().await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let retries_after = await_recorded(&wh.retry_recorder, 1, Duration::from_millis(50)).await;
    assert!(
        retries_after.is_empty(),
        "a second retry must be suppressed within the backoff window"
    );
}

#[tokio::test]
async fn watchdog_allows_retry_after_backoff_window_elapses() {
    // Given a stalled session that already consumed one retry, with a backoff
    // window whose last_retry_at has been backdated past it.
    let mut wh = WatchdogHarness::new().await;
    wh.set_backoff(60, 60);
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.begin_sending();
        session.core.last_history_activity_at = Timestamp::now()
            .checked_sub(Duration::from_mins(1))
            .unwrap();
    }

    // First scan: retry published.
    wh.watchdog.scan_once().await;
    let first = await_recorded(&wh.retry_recorder, 1, Duration::from_millis(500)).await;
    assert_eq!(first.len(), 1, "first stall should retry");

    // Force staleness again.
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.core.last_history_activity_at = Timestamp::now()
            .checked_sub(Duration::from_mins(1))
            .unwrap();
    }

    // Backdate the recorded retry time past the 60s backoff window so the
    // gate observes elapsed > 60s.
    wh.watchdog
        .backdate_last_retry(&wh.session_id, Duration::from_secs(70));

    // Second scan: window elapsed → second retry allowed.
    // The first `await_recorded` drained the recorder, so the second scan
    // should produce exactly one new publish.
    wh.watchdog.scan_once().await;
    let second = await_recorded(&wh.retry_recorder, 1, Duration::from_millis(500)).await;
    assert_eq!(
        second.len(),
        1,
        "a second retry must be allowed once the backoff window has elapsed"
    );
}

/// A session that stalls, produces genuine provider output (the provider
/// `last_provider_activity_at` advances between ticks), then stalls again must
/// get a fresh retry budget — the prior attempt must not count against it.
/// This is the responsive-provider recovery case: intermittent stalls
/// caused by contention should not accumulate toward a hard cancel.
#[tokio::test]
async fn stall_budget_resets_when_provider_activity_resumes() {
    // Given a stalled session that already consumed one retry (budget = 1).
    let mut wh = WatchdogHarness::new().await;
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.begin_sending();
        session.core.last_history_activity_at = Timestamp::now()
            .checked_sub(Duration::from_mins(1))
            .unwrap();
        // Initialize the provider-activity baseline so the first scan records it.
        session.core.last_provider_activity_at = Timestamp::now()
            .checked_sub(Duration::from_mins(1))
            .unwrap();
    }
    wh.watchdog.scan_once().await;
    let first = await_recorded(&wh.retry_recorder, 1, Duration::from_millis(500)).await;
    assert_eq!(first.len(), 1, "first stall should retry within budget");

    // When the provider produced genuine output between scans — advance
    // `last_provider_activity_at` to a more recent value than the baseline.
    // This scan detects recovery and clears the budget (no action taken).
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.core.last_provider_activity_at = Timestamp::now();
        session.core.last_history_activity_at = Timestamp::now();
    }
    wh.watchdog.scan_once().await;

    // Then the session stalls again (history ages out). Because the budget was
    // reset on recovery, this stall must retry — not cancel from the exhausted budget.
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.core.last_history_activity_at = Timestamp::now()
            .checked_sub(Duration::from_mins(1))
            .unwrap();
    }
    wh.watchdog.scan_once().await;
    let second = await_recorded(&wh.retry_recorder, 1, Duration::from_millis(500)).await;
    assert!(
        second.iter().any(|r| r.session_id == wh.session_id),
        "budget must reset when provider activity resumes — second stall should retry, not cancel"
    );
    let cancels = await_recorded(&wh.cancel_recorder, 1, Duration::from_millis(100)).await;
    assert!(
        cancels.iter().all(|c| c.session_id != wh.session_id),
        "must not cancel a responsive session that recovered then re-stalled"
    );
}

#[tokio::test]
async fn watchdog_detects_stall_when_provider_activity_stale() {
    // Given a session that produced provider output on an earlier tick (so the
    // watchdog has recorded a `last_provider_activity` baseline) but is now
    // hard-stuck — no further provider output and stale history activity.
    let mut wh = WatchdogHarness::new().await;
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.begin_streaming();
        // Simulate a prior token arriving (advances provider activity).
        session.core.last_provider_activity_at = Timestamp::now()
            .checked_sub(Duration::from_secs(30))
            .unwrap();
    }

    // First scan: the watchdog records the provider-activity baseline.
    // Force history staleness so the stall window is exceeded.
    wh.watchdog.scan_once().await;
    {
        let mut s = wh.state.write();
        let session = s.session_mut(&wh.session_id);
        session.core.last_history_activity_at = Timestamp::now()
            .checked_sub(Duration::from_mins(1))
            .unwrap();
        // Provider activity unchanged from the baseline above — NOT recovered.
    }

    // When the watchdog scans again with no provider-activity advance.
    wh.watchdog.scan_once().await;
    let retries = await_recorded(&wh.retry_recorder, 1, Duration::from_millis(500)).await;

    // Then the session is detected as stalled despite having been active earlier.
    assert!(
        retries.iter().any(|r| r.session_id == wh.session_id),
        "a session with stale provider activity must reach the stall check, not be skipped by recovered→continue"
    );
}
