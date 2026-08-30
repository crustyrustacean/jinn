//! Stall watchdog state machine — decides when a silent in-flight LLM
//! stream should be restarted.
//!
//! One timer per session, armed by [`StreamStartEvent`](jinn_plugin_api::StreamStartEvent)
//! and reset by every [`StreamEventPing`](jinn_plugin_api::StreamEventPing).
//! When a [`TickEvent`](jinn_plugin_api::TickEvent) reveals a session has
//! been silent past the configured timeout, the guest pushes a mirrored
//! [`RestartStalledStream`](jinn_plugin_api::RestartStalledStream) — up to
//! `max_restarts` consecutive times. Beyond the budget it gives up instead:
//! an [`InsertSystemEntry`](jinn_plugin_api::InsertSystemEntry) explaining
//! the surrender followed by a mirrored
//! [`CancelStream`](jinn_plugin_api::CancelStream).
//!
//! Budget semantics: a stream ending in `Finished` clears the session
//! entirely (genuine completion — fresh budget next turn); `ToolUse`,
//! `Canceled`, and `Error` merely disarm the timer while retaining the count
//! (the same turn or its retry continues). Any stream event after a restart
//! proves the retry connected and resets the budget to zero — the budget
//! counts *consecutive* silent stalls.

use std::collections::HashMap;

use jinn_plugin_api::{
    CancelStream, InsertSystemEntry, PluginToHost, RestartStalledStream, StreamEndEvent,
    StreamEndReason, StreamEventPing, StreamStartEvent, Welcome,
};

/// Default silence window before a restart, in seconds.
pub const DEFAULT_TIMEOUT_SECS: u64 = 60;

/// Config key read from `Welcome.config` for the silence window.
pub const TIMEOUT_SECS_KEY: &str = "timeout_secs";

/// Default consecutive restarts before giving up.
pub const DEFAULT_MAX_RESTARTS: u32 = 3;

/// Config key read from `Welcome.config` for the restart budget.
pub const MAX_RESTARTS_KEY: &str = "max_restarts";

/// Per-session stall timer and restart budget.
#[derive(Default)]
struct SessionStall {
    /// Whether an LLM stream is believed to be in flight.
    armed: bool,
    /// Guest epoch-ms timestamp of the last stream activity (or arm time).
    last_event_ms: u64,
    /// Consecutive stall restarts since the last observed stream output.
    restarts: u32,
}

/// Event-driven stall watchdog: one timer per session, host-tick driven.
pub struct StallWatchdog {
    /// Timers keyed by opaque session id string.
    sessions: HashMap<String, SessionStall>,
    /// Silence window before a restart is considered, in milliseconds.
    timeout_ms: u64,
    /// Consecutive restarts allowed before giving up on the session's turn.
    max_restarts: u32,
}

impl StallWatchdog {
    /// Builds a watchdog configured from the host's `Welcome.config`.
    ///
    /// Reads `timeout_secs` (integer ≥ 1) and `max_restarts` (integer ≥ 1);
    /// absent, non-numeric, or zero values fall back to
    /// [`DEFAULT_TIMEOUT_SECS`] / [`DEFAULT_MAX_RESTARTS`] — a zero window
    /// or zero budget is nonsensical (it would fire on every tick or never
    /// restart at all, regardless of configuration intent).
    #[must_use]
    pub fn from_welcome(welcome: &Welcome) -> Self {
        Self::with_limits(
            parse_timeout_secs(&welcome.config),
            parse_max_restarts(&welcome.config),
        )
    }

    /// Builds a watchdog with an explicit silence window and budget.
    #[must_use]
    pub fn with_limits(timeout_secs: u64, max_restarts: u32) -> Self {
        Self {
            sessions: HashMap::new(),
            timeout_ms: timeout_secs.saturating_mul(1_000),
            max_restarts,
        }
    }

    /// Arms (or re-arms) the session's timer at dispatch time.
    ///
    /// Arming at `stream_start` — not first token — covers the silent
    /// HTTP-handshake gap. A tool-loop turn produces one event per
    /// generation, so each re-dispatch re-arms naturally. The restart budget
    /// survives the re-arm: consecutive stalls within one turn accumulate.
    pub fn on_stream_start(&mut self, event: &StreamStartEvent, now_ms: u64) {
        let stall = self.sessions.entry(event.session_id.clone()).or_default();
        stall.armed = true;
        stall.last_event_ms = now_ms;
    }

    /// Records stream output — the timer resets, and a recovered stall
    /// clears the restart budget.
    ///
    /// Pings for sessions with no armed timer are harmless (the session may
    /// have been disarmed between host forwarding and this event arriving).
    pub fn on_stream_event(&mut self, event: &StreamEventPing, now_ms: u64) {
        if let Some(stall) = self.sessions.get_mut(&event.session_id) {
            stall.last_event_ms = now_ms;
            stall.restarts = 0;
        }
    }

    /// Applies the stream-end policy per terminal reason.
    ///
    /// `Finished` removes the session entirely (budget reset — the turn
    /// completed genuinely). Every other reason disarms the timer while
    /// retaining the budget: `ToolUse` because the same turn continues after
    /// the tool batch, `Canceled`/`Error` because the retry inherits the
    /// turn's stall history.
    pub fn on_stream_end(&mut self, event: &StreamEndEvent) {
        match event.reason {
            StreamEndReason::Finished => {
                self.sessions.remove(&event.session_id);
            }
            StreamEndReason::Canceled | StreamEndReason::ToolUse | StreamEndReason::Error => {
                if let Some(stall) = self.sessions.get_mut(&event.session_id) {
                    stall.armed = false;
                }
            }
        }
    }

    /// Advances time to the host's tick and returns the messages to push,
    /// in order.
    ///
    /// Every armed session silent past the timeout trips exactly once per
    /// window: within budget it yields one restart (and the window restarts
    /// from the tick); past budget it yields the give-up pair — system entry
    /// first, then cancel — and disarms so it cannot fire again until the
    /// next `stream_start`.
    #[must_use]
    pub fn on_tick(&mut self, now_ms: u64) -> Vec<PluginToHost> {
        let timeout_ms = self.timeout_ms;
        let max_restarts = self.max_restarts;
        self.sessions
            .iter_mut()
            .filter(|(_, stall)| {
                stall.armed && now_ms.saturating_sub(stall.last_event_ms) >= timeout_ms
            })
            .flat_map(|(session, stall)| trip(session.clone(), stall, max_restarts, now_ms))
            .collect()
    }
}

/// Trips one stalled session: a restart within budget, otherwise the
/// give-up pair. Mutates the stall so the next tick cannot re-fire early —
/// a restart re-windows from the tick, the give-up disarms entirely.
fn trip(
    session: String,
    stall: &mut SessionStall,
    max_restarts: u32,
    now_ms: u64,
) -> Vec<PluginToHost> {
    if stall.restarts < max_restarts {
        stall.restarts += 1;
        stall.last_event_ms = now_ms;
        return vec![PluginToHost::RestartStalledStream(RestartStalledStream {
            session_id: session,
            attempt: stall.restarts,
            max_restarts,
        })];
    }
    stall.armed = false;
    vec![
        PluginToHost::InsertSystemEntry(InsertSystemEntry {
            session_id: session.clone(),
            text: give_up_text(max_restarts),
        }),
        PluginToHost::CancelStream(CancelStream {
            session_id: session,
        }),
    ]
}

/// The surrender system-entry text after exhausting `max` restarts.
fn give_up_text(max: u32) -> String {
    format!(
        "\u{23f9} stall-watchdog: the LLM stream stalled {max} times without recovery; giving up — cancelling the turn."
    )
}

/// Reads `timeout_secs` out of the free-form config value (seconds ≥ 1).
fn parse_timeout_secs(config: &serde_json::Value) -> u64 {
    match config
        .get(TIMEOUT_SECS_KEY)
        .and_then(serde_json::Value::as_u64)
    {
        Some(secs) if secs >= 1 => secs,
        _ => DEFAULT_TIMEOUT_SECS,
    }
}

/// Reads `max_restarts` out of the free-form config value (count ≥ 1).
fn parse_max_restarts(config: &serde_json::Value) -> u32 {
    match config
        .get(MAX_RESTARTS_KEY)
        .and_then(serde_json::Value::as_u64)
    {
        Some(raw) if (1..=u64::from(u32::MAX)).contains(&raw) => raw as u32,
        _ => DEFAULT_MAX_RESTARTS,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]
    use super::*;
    use serde_json::json;

    /// `StreamStartEvent` factory.
    fn start(session: &str) -> StreamStartEvent {
        StreamStartEvent {
            session_id: session.to_owned(),
        }
    }

    /// `StreamEventPing` factory.
    fn ping(session: &str) -> StreamEventPing {
        StreamEventPing {
            session_id: session.to_owned(),
        }
    }

    /// `StreamEndEvent` factory.
    fn end(session: &str, reason: StreamEndReason) -> StreamEndEvent {
        StreamEndEvent {
            session_id: session.to_owned(),
            reason,
        }
    }

    /// Asserts the pushes are exactly one restart for `session`.
    fn assert_restart(pushes: &[PluginToHost], session: &str, attempt: u32) {
        assert_eq!(pushes.len(), 1, "expected one restart, got: {pushes:?}");
        let PluginToHost::RestartStalledStream(restart) = &pushes[0] else {
            panic!("expected a RestartStalledStream, got: {pushes:?}");
        };
        assert_eq!(restart.session_id, session);
        // And the reported attempt matches the restart ordinal within the
        // stall lineage.
        assert_eq!(restart.attempt, attempt);
    }

    /// Asserts the pushes are exactly the give-up pair (entry then cancel).
    fn assert_give_up(pushes: &[PluginToHost], session: &str) {
        assert_eq!(
            pushes.len(),
            2,
            "expected the give-up pair, got: {pushes:?}"
        );
        let PluginToHost::InsertSystemEntry(entry) = &pushes[0] else {
            panic!("first push must be the system entry, got: {pushes:?}");
        };
        assert_eq!(entry.session_id, session);
        let PluginToHost::CancelStream(cancel) = &pushes[1] else {
            panic!("second push must be the cancel, got: {pushes:?}");
        };
        assert_eq!(cancel.session_id, session);
    }

    #[rstest::rstest]
    #[test]
    fn tick_inside_the_window_pushes_nothing() {
        // Given a watchdog armed for a session at t=0 with a 60s window.
        let mut watchdog = StallWatchdog::with_limits(60, 3);
        watchdog.on_stream_start(&start("s-1"), 1_000);

        // When a tick arrives 59.9 seconds later.
        let pushes = watchdog.on_tick(60_999);

        // Then nothing was pushed — the stream is not yet silent long enough.
        assert!(pushes.is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn dispatch_to_first_token_gap_counts_as_stall_time() {
        // Given a watchdog armed by stream_start alone — no token ever arrived.
        let mut watchdog = StallWatchdog::with_limits(60, 3);
        watchdog.on_stream_start(&start("s-1"), 1_000);

        // When a tick arrives past the window.
        let pushes = watchdog.on_tick(61_000);

        // Then the session restarts — the silent handshake gap is covered.
        assert_restart(&pushes, "s-1", 1);
    }

    #[rstest::rstest]
    #[test]
    fn stream_event_resets_the_window() {
        // Given a stream that produced output 10 seconds into its window.
        let mut watchdog = StallWatchdog::with_limits(60, 3);
        watchdog.on_stream_start(&start("s-1"), 1_000);
        watchdog.on_stream_event(&ping("s-1"), 51_000);

        // When a tick arrives 60 seconds after the original dispatch.
        let pushes = watchdog.on_tick(61_000);

        // Then nothing was pushed — the window runs from the last event.
        assert!(pushes.is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn restart_rewindows_so_one_stall_trips_once() {
        // Given a stalled stream that tripped once at t=61s.
        let mut watchdog = StallWatchdog::with_limits(60, 3);
        watchdog.on_stream_start(&start("s-1"), 1_000);
        assert_restart(&watchdog.on_tick(61_000), "s-1", 1);

        // When the next host tick arrives 4 seconds later (5s cadence).
        let pushes = watchdog.on_tick(65_000);

        // Then the same stall does not re-fire — the window restarted.
        assert!(pushes.is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn consecutive_stall_windows_accumulate_budget() {
        // Given a stream that stalled and was restarted once.
        let mut watchdog = StallWatchdog::with_limits(60, 3);
        watchdog.on_stream_start(&start("s-1"), 1_000);
        assert_restart(&watchdog.on_tick(61_000), "s-1", 1);

        // When the retried stream also goes silent past the window.
        let pushes = watchdog.on_tick(121_000);

        // Then a second restart fires — consecutive stalls count against
        // the budget.
        assert_restart(&pushes, "s-1", 2);
    }

    #[rstest::rstest]
    #[test]
    fn budget_exhaustion_gives_up_with_entry_then_cancel() {
        // Given a watchdog at its budget of 3 that already restarted 3 times.
        let mut watchdog = StallWatchdog::with_limits(60, 3);
        watchdog.on_stream_start(&start("s-1"), 0);
        for window in 1..=3 {
            assert_restart(&watchdog.on_tick(window * 60_000), "s-1", window as u32);
        }

        // When the fourth window also goes silent.
        let pushes = watchdog.on_tick(240_000);

        // Then the watchdog surrenders: the system entry first, then the
        // cancel.
        assert_give_up(&pushes, "s-1");
    }

    #[rstest::rstest]
    #[test]
    fn give_up_fires_only_once() {
        // Given a watchdog that already surrendered for a session.
        let mut watchdog = StallWatchdog::with_limits(60, 3);
        watchdog.on_stream_start(&start("s-1"), 0);
        for window in 1..=3 {
            let _ = watchdog.on_tick(window * 60_000);
        }
        let _ = watchdog.on_tick(240_000);

        // When more ticks arrive.
        let pushes = watchdog.on_tick(300_000);

        // Then nothing is pushed again — the session is disarmed.
        assert!(pushes.is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn tool_use_boundary_retains_the_budget() {
        // Given a watchdog at a budget of 2 that restarted once, then the
        // stream paused for a tool batch and re-dispatched.
        let mut watchdog = StallWatchdog::with_limits(60, 2);
        watchdog.on_stream_start(&start("s-1"), 0);
        assert_restart(&watchdog.on_tick(60_000), "s-1", 1);
        watchdog.on_stream_end(&end("s-1", StreamEndReason::ToolUse));
        watchdog.on_stream_start(&start("s-1"), 61_000);
        assert_restart(&watchdog.on_tick(121_000), "s-1", 2);
        watchdog.on_stream_end(&end("s-1", StreamEndReason::ToolUse));
        watchdog.on_stream_start(&start("s-1"), 122_000);

        // When the third generation also goes silent past the window.
        let pushes = watchdog.on_tick(182_000);

        // Then it gives up — the tool-loop boundaries did not reset the
        // consecutive-stall budget.
        assert_give_up(&pushes, "s-1");
    }

    #[rstest::rstest]
    #[test]
    fn finished_end_resets_the_budget() {
        // Given a watchdog at a budget of 2 that restarted once, then the
        // turn genuinely completed.
        let mut watchdog = StallWatchdog::with_limits(60, 2);
        watchdog.on_stream_start(&start("s-1"), 0);
        assert_restart(&watchdog.on_tick(60_000), "s-1", 1);
        watchdog.on_stream_end(&end("s-1", StreamEndReason::Finished));

        // When a fresh turn stalls past the window.
        watchdog.on_stream_start(&start("s-1"), 120_000);
        let pushes = watchdog.on_tick(180_000);

        // Then it restarts again — the completion restored the full budget
        // (a give-up pair here would mean the budget carried over).
        assert_restart(&pushes, "s-1", 1);
    }

    #[rstest::rstest]
    #[test]
    fn finished_end_removes_the_timer_entirely() {
        // Given a stream that ended in a genuine completion.
        let mut watchdog = StallWatchdog::with_limits(60, 3);
        watchdog.on_stream_start(&start("s-1"), 0);
        watchdog.on_stream_end(&end("s-1", StreamEndReason::Finished));

        // When ticks arrive far into the future.
        let pushes = watchdog.on_tick(600_000);

        // Then nothing is pushed — a finished stream has no timer to trip.
        assert!(pushes.is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn canceled_end_disarms_the_timer() {
        // Given a stream that was canceled mid-flight.
        let mut watchdog = StallWatchdog::with_limits(60, 3);
        watchdog.on_stream_start(&start("s-1"), 0);
        watchdog.on_stream_end(&end("s-1", StreamEndReason::Canceled));

        // When ticks arrive past the window.
        let pushes = watchdog.on_tick(120_000);

        // Then nothing is pushed — a canceled turn must not be restarted.
        assert!(pushes.is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn recovered_stream_clears_the_budget() {
        // Given a watchdog at a budget of 1 that restarted once and then saw
        // the retry produce output (the retry connected).
        let mut watchdog = StallWatchdog::with_limits(60, 1);
        watchdog.on_stream_start(&start("s-1"), 0);
        assert_restart(&watchdog.on_tick(60_000), "s-1", 1);
        watchdog.on_stream_start(&start("s-1"), 61_000);
        watchdog.on_stream_event(&ping("s-1"), 62_000);

        // When the stream later stalls again past the window.
        let pushes = watchdog.on_tick(122_000);

        // Then it restarts rather than giving up — the observed recovery
        // reset the consecutive-stall budget.
        assert_restart(&pushes, "s-1", 1);
    }

    #[rstest::rstest]
    #[test]
    fn sessions_are_tracked_independently() {
        // Given two sessions: one stalled, one actively streaming.
        let mut watchdog = StallWatchdog::with_limits(60, 3);
        watchdog.on_stream_start(&start("s-a"), 0);
        watchdog.on_stream_start(&start("s-b"), 0);
        watchdog.on_stream_event(&ping("s-b"), 55_000);

        // When a tick arrives past the window.
        let pushes = watchdog.on_tick(61_000);

        // Then only the silent session tripped — the pinged one stays quiet.
        assert_restart(&pushes, "s-a", 1);
    }

    #[rstest::rstest]
    #[test]
    fn unknown_session_events_are_harmless() {
        // Given a watchdog with no state for a session.
        let mut watchdog = StallWatchdog::with_limits(60, 3);

        // When pings and stream ends arrive for that unknown session.
        watchdog.on_stream_event(&ping("s-ghost"), 0);
        watchdog.on_stream_end(&end("s-ghost", StreamEndReason::Error));

        // Then nothing panics and an unrelated session still trips normally.
        watchdog.on_stream_start(&start("s-1"), 0);
        let pushes = watchdog.on_tick(60_000);
        assert_restart(&pushes, "s-1", 1);
    }

    #[rstest::rstest]
    #[test]
    fn timeout_from_config_overrides_the_default() {
        // Given a Welcome configured with a 5-second window.
        let welcome = Welcome {
            protocol_version: 1,
            plugin_id: "stall-watchdog".to_owned(),
            read_dirs: vec![],
            write_dirs: vec![],
            http_allowed: false,
            config: json!({ "timeout_secs": 5, "max_restarts": 3 }),
        };
        let mut watchdog = StallWatchdog::from_welcome(&welcome);
        watchdog.on_stream_start(&start("s-1"), 0);

        // When ticks arrive at 4.9s and then 5s.
        let early = watchdog.on_tick(4_999);
        let on_time = watchdog.on_tick(5_000);

        // Then the trip lands exactly at the configured window, where the
        // 60s default would still be silent.
        assert!(early.is_empty());
        assert_restart(&on_time, "s-1", 1);
    }

    #[rstest::rstest]
    #[test]
    fn max_restarts_from_config_overrides_the_default() {
        // Given a Welcome configured with a budget of 1.
        let welcome = Welcome {
            protocol_version: 1,
            plugin_id: "stall-watchdog".to_owned(),
            read_dirs: vec![],
            write_dirs: vec![],
            http_allowed: false,
            config: json!({ "timeout_secs": 1, "max_restarts": 1 }),
        };
        let mut watchdog = StallWatchdog::from_welcome(&welcome);
        watchdog.on_stream_start(&start("s-1"), 0);

        // When two stall windows pass.
        let first = watchdog.on_tick(1_000);
        let second = watchdog.on_tick(2_000);

        // Then the first window restarts and the second gives up — the
        // default budget of 3 would still have restarts left.
        assert_restart(&first, "s-1", 1);
        assert_give_up(&second, "s-1");
    }

    #[rstest::rstest]
    #[case::absent(serde_json::Value::Null)]
    #[case::zero(json!({ "timeout_secs": 0, "max_restarts": 3 }))]
    #[case::not_a_number(json!({ "timeout_secs": "forever", "max_restarts": 3 }))]
    fn nonsense_timeout_falls_back_to_sixty_seconds(#[case] config: serde_json::Value) {
        // Given a Welcome carrying a nonsensical timeout (and a valid budget).
        let welcome = Welcome {
            protocol_version: 1,
            plugin_id: "stall-watchdog".to_owned(),
            read_dirs: vec![],
            write_dirs: vec![],
            http_allowed: false,
            config,
        };
        let mut watchdog = StallWatchdog::from_welcome(&welcome);
        watchdog.on_stream_start(&start("s-1"), 0);

        // When ticks arrive just inside and exactly at 60 seconds.
        let early = watchdog.on_tick(59_999);
        let on_time = watchdog.on_tick(60_000);

        // Then the default 60-second window is in force.
        assert!(early.is_empty());
        assert_restart(&on_time, "s-1", 1);
    }

    #[rstest::rstest]
    #[case::absent(json!({ "timeout_secs": 1 }))]
    #[case::zero(json!({ "timeout_secs": 1, "max_restarts": 0 }))]
    #[case::not_a_number(json!({ "timeout_secs": 1, "max_restarts": "many" }))]
    fn nonsense_max_restarts_falls_back_to_three(#[case] config: serde_json::Value) {
        // Given a Welcome carrying a nonsensical budget (and a fast window).
        let welcome = Welcome {
            protocol_version: 1,
            plugin_id: "stall-watchdog".to_owned(),
            read_dirs: vec![],
            write_dirs: vec![],
            http_allowed: false,
            config,
        };
        let mut watchdog = StallWatchdog::from_welcome(&welcome);
        watchdog.on_stream_start(&start("s-1"), 0);

        // When four stall windows pass.
        let w1 = watchdog.on_tick(1_000);
        let w2 = watchdog.on_tick(2_000);
        let w3 = watchdog.on_tick(3_000);
        let w4 = watchdog.on_tick(4_000);

        // Then the first three windows restart (default budget of 3) and the
        // fourth gives up.
        assert_restart(&w1, "s-1", 1);
        assert_restart(&w2, "s-1", 2);
        assert_restart(&w3, "s-1", 3);
        assert_give_up(&w4, "s-1");
    }
}
