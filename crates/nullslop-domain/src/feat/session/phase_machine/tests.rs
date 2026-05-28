//! Transition test matrix for [`SessionPhaseMachine`].
//!
//! Tests are organized into sections:
//! 1. Valid transitions — each from/to/side-effects verified
//! 2. Invalid transitions — each returns `TransitionError`
//! 3. Tool loop cycles — multi-step sequences
//! 4. Side effects — data tracking within phases

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

use super::machine::{SessionPhaseMachine, TransitionError};
use super::phase::{Phase, PhaseKind};

// ── Helpers ─────────────────────────────────────────────────────────────

/// Create a machine in `Idle` phase.
fn idle_machine() -> SessionPhaseMachine {
    SessionPhaseMachine::new()
}

/// Create a machine in `Sending` phase.
fn sending_machine() -> SessionPhaseMachine {
    let mut m = SessionPhaseMachine::new();
    m.on_dispatch_message().expect("dispatch should succeed");
    m
}

/// Create a machine in `Streaming` phase.
fn streaming_machine() -> SessionPhaseMachine {
    let mut m = SessionPhaseMachine::new();
    m.on_dispatch_message().expect("dispatch should succeed");
    m.on_first_token().expect("first token should succeed");
    m
}

/// Create a machine in `TearingDown` phase.
fn tearing_down_machine() -> SessionPhaseMachine {
    let mut m = SessionPhaseMachine::new();
    m.on_request_teardown().expect("teardown should succeed");
    m
}

fn assert_err(err: &TransitionError, expected_from: PhaseKind, expected_trigger: &str) {
    assert_eq!(err.from, expected_from, "error from phase mismatch");
    assert_eq!(err.trigger, expected_trigger, "error trigger mismatch");
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 1: Valid transitions
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn idle_to_sending_on_dispatch() {
    // Given a machine in Idle.
    let mut m = idle_machine();

    // When dispatching a message.
    let outcome = m.on_dispatch_message().expect("should succeed");

    // Then the outcome records the transition.
    assert_eq!(outcome.old_phase, PhaseKind::Idle);
    assert_eq!(outcome.new_phase, PhaseKind::Sending);
    // And the machine is in Sending.
    assert_eq!(m.kind(), PhaseKind::Sending);
}

#[test]
fn sending_to_streaming_on_first_token() {
    // Given a machine in Sending.
    let mut m = sending_machine();

    // When receiving the first token.
    let outcome = m.on_first_token().expect("should succeed");

    // Then the transition is Sending → Streaming.
    assert_eq!(outcome.old_phase, PhaseKind::Sending);
    assert_eq!(outcome.new_phase, PhaseKind::Streaming);
    assert_eq!(m.kind(), PhaseKind::Streaming);
}

#[test]
fn streaming_to_sending_on_tool_use() {
    // Given a machine in Streaming.
    let mut m = streaming_machine();

    // When stream completes with tool use.
    let outcome = m.on_stream_completed_tool_use().expect("should succeed");

    // Then the transition is Streaming → Sending.
    assert_eq!(outcome.old_phase, PhaseKind::Streaming);
    assert_eq!(outcome.new_phase, PhaseKind::Sending);
    assert!(m.streaming_phase().is_none(), "streaming data should be gone");
}

#[test]
fn streaming_to_idle_on_finished() {
    // Given a machine in Streaming.
    let mut m = streaming_machine();

    // When stream completes normally.
    let outcome = m.on_stream_completed_finished().expect("should succeed");

    // Then the transition is Streaming → Idle.
    assert_eq!(outcome.old_phase, PhaseKind::Streaming);
    assert_eq!(outcome.new_phase, PhaseKind::Idle);
    assert!(m.streaming_phase().is_none(), "streaming data should be gone");
}

#[test]
fn streaming_to_idle_on_error() {
    // Given a machine in Streaming.
    let mut m = streaming_machine();

    // When stream completes with error.
    let outcome = m.on_stream_completed_error().expect("should succeed");

    // Then the transition is Streaming → Idle.
    assert_eq!(outcome.old_phase, PhaseKind::Streaming);
    assert_eq!(outcome.new_phase, PhaseKind::Idle);
}

#[test]
fn streaming_to_idle_on_canceled() {
    // Given a machine in Streaming.
    let mut m = streaming_machine();

    // When stream is canceled.
    let outcome = m.on_stream_completed_canceled().expect("should succeed");

    // Then the transition is Streaming → Idle.
    assert_eq!(outcome.old_phase, PhaseKind::Streaming);
    assert_eq!(outcome.new_phase, PhaseKind::Idle);
}

#[test]
fn sending_to_streaming_on_tool_batch() {
    // Given a machine in Sending (no flags set).
    let mut m = sending_machine();

    // When tool batch completes.
    let outcome = m.on_tool_batch_completed().expect("should succeed");

    // Then the transition is Sending → Streaming.
    assert_eq!(outcome.old_phase, PhaseKind::Sending);
    assert_eq!(outcome.new_phase, PhaseKind::Streaming);
}

#[test]
fn sending_to_idle_on_tool_loop_disabled() {
    // Given a machine in Sending with tool_loop_disabled set.
    let mut m = sending_machine();
    m.sending_phase_mut()
        .expect("should be in Sending")
        .tool_loop_disabled = true;

    // When tool batch completes.
    let outcome = m.on_tool_batch_completed().expect("should succeed");

    // Then the transition is Sending → Idle.
    assert_eq!(outcome.old_phase, PhaseKind::Sending);
    assert_eq!(outcome.new_phase, PhaseKind::Idle);
}

#[test]
fn idle_to_tearing_down() {
    // Given a machine in Idle.
    let mut m = idle_machine();

    // When requesting teardown.
    let outcome = m.on_request_teardown().expect("should succeed");

    // Then the transition is Idle → TearingDown.
    assert_eq!(outcome.old_phase, PhaseKind::Idle);
    assert_eq!(outcome.new_phase, PhaseKind::TearingDown);
}

#[test]
fn tearing_down_to_idle() {
    // Given a machine in TearingDown.
    let mut m = tearing_down_machine();

    // When teardown completes.
    let outcome = m.on_teardown_complete().expect("should succeed");

    // Then the transition is TearingDown → Idle.
    assert_eq!(outcome.old_phase, PhaseKind::TearingDown);
    assert_eq!(outcome.new_phase, PhaseKind::Idle);
}

#[test]
fn cancel_during_streaming() {
    // Given a machine in Streaming.
    let mut m = streaming_machine();

    // When cancel is called.
    let result = m.cancel().expect("should succeed");

    // Then the outcome is Streaming → Idle.
    assert_eq!(result.outcome.old_phase, PhaseKind::Streaming);
    assert_eq!(result.outcome.new_phase, PhaseKind::Idle);
    // And the machine is in Idle.
    assert_eq!(m.kind(), PhaseKind::Idle);
}

#[test]
fn soft_cancel_sets_flag() {
    // Given a machine in Streaming.
    let mut m = streaming_machine();

    // When soft cancel is called.
    m.soft_cancel().expect("should succeed");

    // Then the flag is set on the streaming phase.
    assert!(m.streaming_phase().expect("should be streaming").soft_cancel_requested);
    // And the phase is still Streaming (no transition).
    assert_eq!(m.kind(), PhaseKind::Streaming);
}

#[test]
fn soft_cancel_deferred_to_finished() {
    // Given a machine in Streaming with soft cancel requested.
    let mut m = streaming_machine();
    m.soft_cancel().expect("soft cancel should succeed");

    // When stream completes normally.
    let outcome = m.on_stream_completed_finished().expect("should succeed");

    // Then the transition is Streaming → Idle.
    assert_eq!(outcome.old_phase, PhaseKind::Streaming);
    assert_eq!(outcome.new_phase, PhaseKind::Idle);
}

#[test]
fn soft_cancel_deferred_to_tool_use() {
    // Given a machine in Streaming with soft cancel requested.
    let mut m = streaming_machine();
    m.soft_cancel().expect("soft cancel should succeed");

    // When stream completes with tool use.
    let outcome = m.on_stream_completed_tool_use().expect("should succeed");

    // Then the transition goes to Idle (not Sending) because soft cancel was set.
    assert_eq!(outcome.old_phase, PhaseKind::Streaming);
    assert_eq!(outcome.new_phase, PhaseKind::Idle);
}

#[test]
fn first_token_creates_default_streaming_state() {
    // Given a machine in Sending.
    let mut m = sending_machine();

    // When receiving first token.
    m.on_first_token().expect("should succeed");

    // Then the streaming phase has default (empty) values.
    let sp = m.streaming_phase().expect("should be streaming");
    assert!(sp.streaming_entry_index.is_none());
    assert!(sp.streaming_thinking_entry_index.is_none());
    assert!(sp.streaming_tool_call_indices.is_empty());
    assert!(sp.streaming_tool_result_indices.is_empty());
    assert!(!sp.soft_cancel_requested);
}

#[test]
fn streaming_state_cleared_on_finish() {
    // Given a machine in Streaming with populated state.
    let mut m = streaming_machine();
    {
        let sp = m.streaming_phase_mut().expect("should be streaming");
        sp.streaming_entry_index = Some(5);
        sp.streaming_thinking_entry_index = Some(3);
        sp.streaming_tool_call_indices.insert(0, 10);
        sp.streaming_tool_result_indices.insert("tc_1".to_owned(), 12);
    }

    // When stream finishes.
    m.on_stream_completed_finished().expect("should succeed");

    // Then all streaming state is gone (dropped with the variant).
    assert!(m.streaming_phase().is_none());
    assert_eq!(m.kind(), PhaseKind::Idle);
}

#[test]
fn tool_loop_disabled_cleared() {
    // Given a machine in Sending with tool_loop_disabled set.
    let mut m = sending_machine();
    m.sending_phase_mut()
        .expect("should be sending")
        .tool_loop_disabled = true;

    // When tool batch completes.
    let outcome = m.on_tool_batch_completed().expect("should succeed");

    // Then we go to Idle (not Streaming) and the flag is gone.
    assert_eq!(outcome.new_phase, PhaseKind::Idle);
    assert!(m.sending_phase().is_none(), "sending phase should be gone");
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 2: Invalid transitions
// ═══════════════════════════════════════════════════════════════════════════

// --- on_dispatch_message ---

#[test]
fn reject_dispatch_while_streaming() {
    let mut m = streaming_machine();
    let err = m.on_dispatch_message().unwrap_err();
    assert_err(&err, PhaseKind::Streaming, "on_dispatch_message");
}

#[test]
fn reject_dispatch_while_sending() {
    let mut m = sending_machine();
    let err = m.on_dispatch_message().unwrap_err();
    assert_err(&err, PhaseKind::Sending, "on_dispatch_message");
}

#[test]
fn reject_dispatch_while_tearing_down() {
    let mut m = tearing_down_machine();
    let err = m.on_dispatch_message().unwrap_err();
    assert_err(&err, PhaseKind::TearingDown, "on_dispatch_message");
}

// --- on_first_token ---

#[test]
fn reject_first_token_while_idle() {
    let mut m = idle_machine();
    let err = m.on_first_token().unwrap_err();
    assert_err(&err, PhaseKind::Idle, "on_first_token");
}

#[test]
fn reject_first_token_while_streaming() {
    let mut m = streaming_machine();
    let err = m.on_first_token().unwrap_err();
    assert_err(&err, PhaseKind::Streaming, "on_first_token");
}

#[test]
fn reject_first_token_while_tearing_down() {
    let mut m = tearing_down_machine();
    let err = m.on_first_token().unwrap_err();
    assert_err(&err, PhaseKind::TearingDown, "on_first_token");
}

// --- on_stream_completed_finished ---

#[test]
fn reject_stream_completed_while_idle() {
    let mut m = idle_machine();
    let err = m.on_stream_completed_finished().unwrap_err();
    assert_err(&err, PhaseKind::Idle, "on_stream_completed_finished");
}

#[test]
fn reject_stream_completed_while_sending() {
    let mut m = sending_machine();
    let err = m.on_stream_completed_finished().unwrap_err();
    assert_err(&err, PhaseKind::Sending, "on_stream_completed_finished");
}

#[test]
fn reject_stream_completed_while_tearing_down() {
    let mut m = tearing_down_machine();
    let err = m.on_stream_completed_finished().unwrap_err();
    assert_err(&err, PhaseKind::TearingDown, "on_stream_completed_finished");
}

// --- on_tool_batch_completed ---

#[test]
fn reject_tool_batch_while_idle() {
    let mut m = idle_machine();
    let err = m.on_tool_batch_completed().unwrap_err();
    assert_err(&err, PhaseKind::Idle, "on_tool_batch_completed");
}

#[test]
fn reject_tool_batch_while_streaming() {
    let mut m = streaming_machine();
    let err = m.on_tool_batch_completed().unwrap_err();
    assert_err(&err, PhaseKind::Streaming, "on_tool_batch_completed");
}

#[test]
fn reject_tool_batch_while_tearing_down() {
    let mut m = tearing_down_machine();
    let err = m.on_tool_batch_completed().unwrap_err();
    assert_err(&err, PhaseKind::TearingDown, "on_tool_batch_completed");
}

// --- on_request_teardown ---

#[test]
fn reject_teardown_while_streaming() {
    let mut m = streaming_machine();
    let err = m.on_request_teardown().unwrap_err();
    assert_err(&err, PhaseKind::Streaming, "on_request_teardown");
}

#[test]
fn reject_teardown_while_sending() {
    let mut m = sending_machine();
    let err = m.on_request_teardown().unwrap_err();
    assert_err(&err, PhaseKind::Sending, "on_request_teardown");
}

#[test]
fn reject_teardown_while_tearing_down() {
    let mut m = tearing_down_machine();
    let err = m.on_request_teardown().unwrap_err();
    assert_err(&err, PhaseKind::TearingDown, "on_request_teardown");
}

// --- on_teardown_complete ---

#[test]
fn reject_teardown_complete_while_idle() {
    let mut m = idle_machine();
    let err = m.on_teardown_complete().unwrap_err();
    assert_err(&err, PhaseKind::Idle, "on_teardown_complete");
}

#[test]
fn reject_teardown_complete_while_streaming() {
    let mut m = streaming_machine();
    let err = m.on_teardown_complete().unwrap_err();
    assert_err(&err, PhaseKind::Streaming, "on_teardown_complete");
}

// --- cancel ---

#[test]
fn reject_cancel_while_idle() {
    let mut m = idle_machine();
    let err = m.cancel().unwrap_err();
    assert_err(&err, PhaseKind::Idle, "cancel");
}

#[test]
fn reject_cancel_while_sending() {
    let mut m = sending_machine();
    let err = m.cancel().unwrap_err();
    assert_err(&err, PhaseKind::Sending, "cancel");
}

#[test]
fn reject_cancel_while_tearing_down() {
    let mut m = tearing_down_machine();
    let err = m.cancel().unwrap_err();
    assert_err(&err, PhaseKind::TearingDown, "cancel");
}

// --- soft_cancel ---

#[test]
fn reject_soft_cancel_while_idle() {
    let mut m = idle_machine();
    let err = m.soft_cancel().unwrap_err();
    assert_err(&err, PhaseKind::Idle, "soft_cancel");
}

#[test]
fn reject_soft_cancel_while_sending() {
    let mut m = sending_machine();
    let err = m.soft_cancel().unwrap_err();
    assert_err(&err, PhaseKind::Sending, "soft_cancel");
}

#[test]
fn reject_soft_cancel_while_tearing_down() {
    let mut m = tearing_down_machine();
    let err = m.soft_cancel().unwrap_err();
    assert_err(&err, PhaseKind::TearingDown, "soft_cancel");
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 3: Tool loop cycles
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn full_tool_loop_cycle() {
    // Idle → Sending → Streaming → Sending → Streaming → Idle
    let mut m = SessionPhaseMachine::new();

    // First turn: dispatch and stream with tool use.
    m.on_dispatch_message().expect("dispatch 1");
    assert_eq!(m.kind(), PhaseKind::Sending);
    m.on_first_token().expect("first token 1");
    assert_eq!(m.kind(), PhaseKind::Streaming);
    m.on_stream_completed_tool_use().expect("tool use 1");
    assert_eq!(m.kind(), PhaseKind::Sending);

    // Tool batch completes — continue to second stream.
    m.on_tool_batch_completed().expect("tool batch");
    assert_eq!(m.kind(), PhaseKind::Streaming);

    // Second stream finishes normally.
    m.on_stream_completed_finished().expect("finished");
    assert_eq!(m.kind(), PhaseKind::Idle);
}

#[test]
fn tool_loop_with_cancel_mid_stream() {
    // Idle → Sending → Streaming → cancel() → Idle
    let mut m = SessionPhaseMachine::new();

    m.on_dispatch_message().expect("dispatch");
    m.on_first_token().expect("first token");
    let result = m.cancel().expect("cancel");

    assert_eq!(result.outcome.new_phase, PhaseKind::Idle);
    assert_eq!(m.kind(), PhaseKind::Idle);
}

#[test]
fn tool_loop_with_soft_cancel_at_boundary() {
    // Idle → Sending → Streaming → soft_cancel() → on_stream_completed_tool_use() → Idle
    let mut m = SessionPhaseMachine::new();

    m.on_dispatch_message().expect("dispatch");
    m.on_first_token().expect("first token");
    m.soft_cancel().expect("soft cancel");

    // Soft cancel should cause tool-use completion to go to Idle, not Sending.
    let outcome = m.on_stream_completed_tool_use().expect("tool use");
    assert_eq!(outcome.new_phase, PhaseKind::Idle);
    assert_eq!(m.kind(), PhaseKind::Idle);
}

#[test]
fn tool_loop_disabled_mid_cycle() {
    // Idle → Sending → Streaming → Sending(tool_loop_disabled=true) → Idle
    let mut m = SessionPhaseMachine::new();

    m.on_dispatch_message().expect("dispatch");
    m.on_first_token().expect("first token");
    m.on_stream_completed_tool_use().expect("tool use");

    // Set tool_loop_disabled while in Sending.
    m.sending_phase_mut()
        .expect("should be sending")
        .tool_loop_disabled = true;

    let outcome = m.on_tool_batch_completed().expect("tool batch");
    assert_eq!(outcome.new_phase, PhaseKind::Idle);
    assert_eq!(m.kind(), PhaseKind::Idle);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 4: Side effects
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn cancel_returns_streaming_data() {
    // Given a machine in Streaming with populated state.
    let mut m = streaming_machine();
    {
        let sp = m.streaming_phase_mut().expect("streaming");
        sp.streaming_entry_index = Some(42);
        sp.streaming_tool_call_indices.insert(0, 10);
        sp.streaming_tool_call_indices.insert(1, 11);
        sp.streaming_tool_result_indices.insert("tc_a".to_owned(), 20);
    }

    // When cancel is called.
    let result = m.cancel().expect("cancel should succeed");

    // Then the old streaming data is preserved in the result.
    assert_eq!(result.old_streaming.streaming_entry_index, Some(42));
    assert_eq!(result.old_streaming.streaming_tool_call_indices.len(), 2);
    assert_eq!(result.old_streaming.streaming_tool_result_indices.len(), 1);
    assert_eq!(
        result.old_streaming.streaming_tool_call_indices.get(&0),
        Some(&10)
    );
    assert_eq!(
        result.old_streaming.streaming_tool_result_indices.get("tc_a"),
        Some(&20)
    );
}

#[test]
fn streaming_phase_tracks_tool_calls() {
    // Given a machine in Streaming.
    let mut m = streaming_machine();

    // When tool call indices are set.
    m.streaming_phase_mut()
        .expect("streaming")
        .streaming_tool_call_indices
        .insert(3, 15);

    // Then they persist until the phase changes.
    assert_eq!(
        m.streaming_phase()
            .expect("streaming")
            .streaming_tool_call_indices
            .get(&3),
        Some(&15)
    );
}

#[test]
fn streaming_phase_tracks_thinking() {
    // Given a machine in Streaming.
    let mut m = streaming_machine();

    // When thinking index is set.
    m.streaming_phase_mut().expect("streaming").streaming_thinking_entry_index = Some(7);

    // Then it persists.
    assert_eq!(
        m.streaming_phase()
            .expect("streaming")
            .streaming_thinking_entry_index,
        Some(7)
    );
}

#[test]
fn sending_phase_tracks_tool_loop_disabled() {
    // Given a machine in Sending.
    let mut m = sending_machine();

    // When the flag is set.
    m.sending_phase_mut().expect("sending").tool_loop_disabled = true;

    // Then on_tool_batch_completed reads it and goes to Idle.
    let outcome = m.on_tool_batch_completed().expect("tool batch");
    assert_eq!(outcome.new_phase, PhaseKind::Idle);
}

#[test]
fn soft_cancel_flag_consumed_on_transition() {
    // Given a machine in Streaming with soft cancel set.
    let mut m = streaming_machine();
    m.soft_cancel().expect("soft cancel");
    assert!(m.streaming_phase().expect("streaming").soft_cancel_requested);

    // When transition happens.
    m.on_stream_completed_finished().expect("finished");

    // Then the flag is gone (the StreamingPhase was dropped).
    assert!(m.streaming_phase().is_none());
    assert_eq!(m.kind(), PhaseKind::Idle);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 5: Accessor edge cases
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn streaming_accessor_returns_none_when_not_streaming() {
    let m = SessionPhaseMachine::new();
    assert!(m.streaming_phase().is_none());
    let mut m = m;
    assert!(m.streaming_phase_mut().is_none());
}

#[test]
fn sending_accessor_returns_none_when_not_sending() {
    let m = SessionPhaseMachine::new();
    assert!(m.sending_phase().is_none());
    let mut m = m;
    assert!(m.sending_phase_mut().is_none());
}

#[test]
fn phase_starts_as_idle() {
    let m = SessionPhaseMachine::new();
    assert_eq!(m.kind(), PhaseKind::Idle);
    assert!(matches!(m.phase(), Phase::Idle(_)));
}
