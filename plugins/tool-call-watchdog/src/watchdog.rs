//! Watchdog accumulator — decides when a session's tool failures have
//! become a spiral.
//!
//! One saturating counter per session: a failed tool result increments it,
//! a successful one debits it by one (floor at zero), and reaching the
//! configured maximum trips the watchdog — the guest pushes an
//! [`InsertSystemEntry`](jinn_plugin_api::InsertSystemEntry) explaining the
//! kill followed by a [`CancelStream`](jinn_plugin_api::CancelStream),
//! then resets the counter so the same session is not re-killed
//! immediately. A turn that ends in a genuine final answer resets the
//! counter (recovery latch); a turn ended by error/cancel retains it.

use std::collections::HashMap;

use jinn_plugin_api::{CancelStream, InsertSystemEntry, PluginToHost, Welcome};

/// Default maximum tolerated failures before the watchdog trips.
pub const DEFAULT_MAX_FAILURES: u8 = 4;

/// Config key read from `Welcome.config` for the maximum.
pub const MAX_FAILURES_KEY: &str = "max_failures";

/// Per-session failure accumulator.
pub struct Watchdog {
    /// Failure counters keyed by opaque session id string.
    accumulators: HashMap<String, u32>,
    /// Maximum tolerated failures before tripping.
    max_failures: u8,
}

impl Watchdog {
    /// Builds a watchdog configured from the host's `Welcome.config`.
    ///
    /// Reads `max_failures` (integer, clamped to `1..=255`); an absent,
    /// non-numeric, or zero value falls back to
    /// [`DEFAULT_MAX_FAILURES`] — a zero maximum is nonsensical (it would
    /// trip on the first failure regardless of configuration intent).
    #[must_use]
    pub fn from_welcome(welcome: &Welcome) -> Self {
        Self::with_max_failures(Self::parse_max_failures(&welcome.config))
    }

    /// Builds a watchdog with an explicit maximum.
    #[must_use]
    pub fn with_max_failures(max_failures: u8) -> Self {
        Self {
            accumulators: HashMap::new(),
            max_failures,
        }
    }

    /// Records one tool result and returns the messages to push, in order.
    ///
    /// Success debits the session's counter (never below zero); failure
    /// increments it, and hitting the maximum produces the watchdog pair —
    /// the system entry first, then the cancel — and zeroes the counter.
    #[must_use]
    pub fn on_tool_result(&mut self, event: &jinn_plugin_api::ToolResultEvent) -> Vec<PluginToHost> {
        let session = event.session_id.clone();
        if event.success {
            if let Some(count) = self.accumulators.get_mut(&session) {
                *count = count.saturating_sub(1);
            }
            return Vec::new();
        }
        let count = self.accumulators.get(&session).copied().unwrap_or(0) + 1;
        if count < u32::from(self.max_failures) {
            self.accumulators.insert(session, count);
            return Vec::new();
        }
        // Latch: zero the counter so the same session is not re-killed
        // immediately.
        self.accumulators.insert(session.clone(), 0);
        vec![
            PluginToHost::InsertSystemEntry(InsertSystemEntry {
                session_id: session.clone(),
                text: Self::trip_text(self.max_failures, count),
            }),
            PluginToHost::CancelStream(CancelStream { session_id: session }),
        ]
    }

    /// Applies the turn-end policy: a genuine final answer resets the
    /// session's counter; an aborted turn (error/cancel) retains it so the
    /// spiral context survives into the retry.
    pub fn on_turn_end(&mut self, event: &jinn_plugin_api::TurnEndEvent) {
        if event.final_answer {
            self.accumulators.remove(&event.session_id);
        }
    }

    /// The watchdog system-entry text for a trip after `failures`
    /// consecutive failures with a `max` limit.
    #[must_use]
    fn trip_text(max: u8, failures: u32) -> String {
        format!(
            "\u{1f6d1} tool-call-watchdog: {failures} consecutive tool failures reached the allowed maximum ({max}); cancelling the stream."
        )
    }

    /// Reads `max_failures` out of the free-form config value.
    fn parse_max_failures(config: &serde_json::Value) -> u8 {
        match config.get(MAX_FAILURES_KEY).and_then(serde_json::Value::as_u64) {
            Some(raw) if (1..=u64::from(u8::MAX)).contains(&raw) => raw as u8,
            _ => DEFAULT_MAX_FAILURES,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]
    use super::*;
    use jinn_plugin_api::{ToolResultEvent, TurnEndEvent};

    /// A failing tool-result event factory.
    fn failure(session: &str, call: &str) -> ToolResultEvent {
        ToolResultEvent {
            session_id: session.to_owned(),
            tool_call_id: call.to_owned(),
            name: "web-fetch".to_owned(),
            content: "boom".to_owned(),
            success: false,
        }
    }

    /// A successful tool-result event factory.
    fn success(session: &str, call: &str) -> ToolResultEvent {
        ToolResultEvent {
            session_id: session.to_owned(),
            tool_call_id: call.to_owned(),
            name: "web-fetch".to_owned(),
            content: "ok".to_owned(),
            success: true,
        }
    }

    /// A turn-end event factory.
    fn turn_end(session: &str, final_answer: bool) -> TurnEndEvent {
        TurnEndEvent {
            session_id: session.to_owned(),
            final_answer,
        }
    }

    #[rstest::rstest]
    #[test]
    fn four_consecutive_failures_send_entry_then_cancel() {
        // Given a watchdog with the default maximum of 4.
        let mut watchdog = Watchdog::with_max_failures(4);

        // When four tool results fail in a row.
        let mut pushes = Vec::new();
        for call in ["c1", "c2", "c3", "c4"] {
            pushes.extend(watchdog.on_tool_result(&failure("s-1", call)));
        }

        // Then exactly two messages were pushed — the system entry first,
        // then the cancel — both for the failing session.
        assert_eq!(pushes.len(), 2);
        let PluginToHost::InsertSystemEntry(entry) = &pushes[0] else {
            panic!("first push must be the system entry");
        };
        assert_eq!(entry.session_id, "s-1");
        assert!(entry.text.contains("4"));
        let PluginToHost::CancelStream(cancel) = &pushes[1] else {
            panic!("second push must be the cancel");
        };
        assert_eq!(cancel.session_id, "s-1");

        // And the accumulator was zeroed: the next lone failure pushes
        // nothing (the latch prevents an immediate re-kill).
        assert!(watchdog.on_tool_result(&failure("s-1", "c5")).is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn fewer_than_max_failures_push_nothing() {
        // Given a watchdog with the default maximum of 4.
        let mut watchdog = Watchdog::with_max_failures(4);

        // When three failures arrive.
        let pushes: Vec<_> = ["c1", "c2", "c3"]
            .iter()
            .flat_map(|call| watchdog.on_tool_result(&failure("s-1", call)))
            .collect();

        // Then nothing was pushed.
        assert!(pushes.is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn successes_debit_and_floor_at_zero() {
        // Given a watchdog that has seen two failures.
        let mut watchdog = Watchdog::with_max_failures(4);
        let _ = watchdog.on_tool_result(&failure("s-1", "c1"));
        let _ = watchdog.on_tool_result(&failure("s-1", "c2"));

        // When three successes follow.
        for call in ["c3", "c4", "c5"] {
            let _ = watchdog.on_tool_result(&success("s-1", call));
        }

        // Then the counter debited to zero and floored there: the next two
        // failures alone cannot trip it.
        let pushes: Vec<_> = ["c6", "c7"]
            .iter()
            .flat_map(|call| watchdog.on_tool_result(&failure("s-1", call)))
            .collect();
        assert!(pushes.is_empty(), "successes must have debited to 0");
    }

    #[rstest::rstest]
    #[test]
    fn successes_before_reaching_max_prevent_the_trip() {
        // Given a watchdog that saw three failures then one success.
        let mut watchdog = Watchdog::with_max_failures(4);
        for call in ["c1", "c2", "c3"] {
            let _ = watchdog.on_tool_result(&failure("s-1", call));
        }
        let _ = watchdog.on_tool_result(&success("s-1", "c4"));

        // When a fourth failure arrives.
        let pushes = watchdog.on_tool_result(&failure("s-1", "c5"));

        // Then the watchdog did not trip — only three failures are counted.
        assert!(pushes.is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn accumulator_is_per_session() {
        // Given a watchdog that tripped for session A (counter reset to 0)
        // and session B untouched.
        let mut watchdog = Watchdog::with_max_failures(2);
        let _ = watchdog.on_tool_result(&failure("s-a", "c1"));
        let _ = watchdog.on_tool_result(&failure("s-a", "c2"));

        // When session B fails once.
        let pushes = watchdog.on_tool_result(&failure("s-b", "c1"));

        // Then B's trip did not fire off A's history: each session counts
        // alone (B needs its own second failure).
        assert!(pushes.is_empty());
        let pushes = watchdog.on_tool_result(&failure("s-b", "c2"));
        assert_eq!(pushes.len(), 2, "B tripped on its own count");
    }

    #[rstest::rstest]
    #[test]
    fn max_failures_from_config_default_4() {
        // Given a Welcome whose config carries max_failures = 2.
        let welcome = Welcome {
            protocol_version: 1,
            plugin_id: "tool-call-watchdog".to_owned(),
            read_dirs: vec![],
            write_dirs: vec![],
            http_allowed: false,
            config: serde_json::json!({ "max_failures": 2 }),
        };

        // When building the watchdog and feeding two failures.
        let mut watchdog = Watchdog::from_welcome(&welcome);
        let _ = watchdog.on_tool_result(&failure("s-1", "c1"));
        let pushes = watchdog.on_tool_result(&failure("s-1", "c2"));

        // Then it tripped at 2, per the config — where the default of 4
        // would still be one failure short.
        assert_eq!(pushes.len(), 2);
    }

    #[rstest::rstest]
    #[case::absent(serde_json::Value::Null)]
    #[case::zero(serde_json::json!({ "max_failures": 0 }))]
    #[case::not_a_number(serde_json::json!({ "max_failures": "lots" }))]
    #[case::above_byte(serde_json::json!({ "max_failures": 300 }))]
    fn nonsense_config_falls_back_to_default_4(#[case] config: serde_json::Value) {
        // Given a Welcome carrying a nonsensical config value.
        let welcome = Welcome {
            protocol_version: 1,
            plugin_id: "tool-call-watchdog".to_owned(),
            read_dirs: vec![],
            write_dirs: vec![],
            http_allowed: false,
            config,
        };

        // When building the watchdog and feeding three failures.
        let mut watchdog = Watchdog::from_welcome(&welcome);
        for call in ["c1", "c2", "c3"] {
            let _ = watchdog.on_tool_result(&failure("s-1", call));
        }
        let pushes = watchdog.on_tool_result(&failure("s-1", "c4"));

        // Then it tripped on the fourth failure — the default of 4, not the
        // (0/absent/garbage) config value.
        assert_eq!(pushes.len(), 2);
    }

    #[rstest::rstest]
    #[test]
    fn final_answer_turn_end_resets_accumulator() {
        // Given a watchdog holding three failures for a session.
        let mut watchdog = Watchdog::with_max_failures(4);
        for call in ["c1", "c2", "c3"] {
            let _ = watchdog.on_tool_result(&failure("s-1", call));
        }

        // When the turn ends with a genuine final answer and a new turn
        // logs one more failure.
        watchdog.on_turn_end(&turn_end("s-1", true));
        let pushes = watchdog.on_tool_result(&failure("s-1", "c4"));

        // Then the watchdog did not trip: the clean turn reset the count.
        assert!(pushes.is_empty());
    }

    #[rstest::rstest]
    #[test]
    fn non_final_turn_end_retains_accumulator() {
        // Given a watchdog holding three failures for a session.
        let mut watchdog = Watchdog::with_max_failures(4);
        for call in ["c1", "c2", "c3"] {
            let _ = watchdog.on_tool_result(&failure("s-1", call));
        }

        // When the turn is aborted (no final answer) and the retry turn
        // logs one more failure.
        watchdog.on_turn_end(&turn_end("s-1", false));
        let pushes = watchdog.on_tool_result(&failure("s-1", "c4"));

        // Then the watchdog trips: the retained count carried over.
        assert_eq!(pushes.len(), 2);
    }

    #[rstest::rstest]
    #[test]
    fn unknown_session_turn_end_is_harmless() {
        // Given a watchdog with no state.
        let mut watchdog = Watchdog::with_max_failures(4);

        // When a turn ends for a session it never saw.
        watchdog.on_turn_end(&turn_end("s-ghost", true));

        // Then nothing happens and the watchdog still trips normally later.
        for call in ["c1", "c2", "c3", "c4"] {
            let pushes = watchdog.on_tool_result(&failure("s-1", call));
            if call == "c4" {
                assert_eq!(pushes.len(), 2);
            } else {
                assert!(pushes.is_empty());
            }
        }
    }

    #[rstest::rstest]
    #[test]
    fn unknown_session_success_is_harmless() {
        // Given a watchdog with no state.
        let mut watchdog = Watchdog::with_max_failures(4);

        // When a success arrives for a session it never saw.
        let pushes = watchdog.on_tool_result(&success("s-1", "c0"));

        // Then nothing was pushed and nothing tripped afterwards.
        assert!(pushes.is_empty());
        for call in ["c1", "c2", "c3", "c4"] {
            let pushes = watchdog.on_tool_result(&failure("s-1", call));
            if call == "c4" {
                assert_eq!(pushes.len(), 2);
            } else {
                assert!(pushes.is_empty());
            }
        }
    }
}
