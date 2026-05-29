//! Validated phase transition machine for a chat session.
//!
//! [`SessionPhaseMachine`] owns the current [`Phase`] and enforces that all
//! transitions are valid. Each transition method is named after the triggering
//! event and returns [`Result<TransitionOutcome, TransitionError>`].
//!
//! # Transition graph
//!
//! ```text
//! Idle ──on_dispatch_message()──► Sending ──on_first_token()──► Streaming
//!   ▲                               │                              │
//!   │                               │     on_stream_completed_*()  │
//!   │                               │◄─────────────────────────────┘
//!   │                               │
//!   │         on_tool_batch_completed()
//!   │                               │
//!   └───────────────────────────────┘ (if tool_loop_disabled)
//!
//! Idle ──on_request_teardown()──► TearingDown ──on_teardown_complete()──► Idle
//! ```

use serde::{Deserialize, Serialize};

use super::phase::{IdlePhase, Phase, PhaseKind, SendingPhase, StreamingPhase};

/// Result of a successful phase transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionOutcome {
    /// The phase before the transition.
    pub old_phase: PhaseKind,
    /// The phase after the transition.
    pub new_phase: PhaseKind,
}

/// Result of a successful `cancel()` — includes the old streaming phase data
/// so the caller can force-exclude dangling tool calls and drain the queue.
#[derive(Debug)]
pub struct CancelOutcome {
    /// The transition outcome.
    pub outcome: TransitionOutcome,
    /// The old `StreamingPhase` data, consumed by the caller for cleanup.
    pub old_streaming: StreamingPhase,
}

/// Error returned when a transition is not valid from the current phase.
///
/// Callers should attach contextual information via `.attach()` to explain
/// why the transition was attempted.
#[derive(Debug, wherror::Error)]
#[error("invalid transition from {from:?}")]
pub struct TransitionError {
    /// The phase the machine was in when the invalid transition was attempted.
    pub from: PhaseKind,
}

/// Validated phase transition machine for a chat session.
///
/// Owns the current [`Phase`] and enforces that all transitions are valid.
/// Invalid transitions return [`TransitionError`] rather than panicking or
/// silently no-oping.
///
/// Construct with [`SessionPhaseMachine::new`] (starts in `Idle`).
/// Call transition methods to advance the phase. Use accessor methods
/// (`streaming_phase`, `sending_phase_mut`, etc.) to read or modify
/// per-phase data without transitioning.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionPhaseMachine {
    phase: Phase,
    /// When `true`, `on_tool_batch_completed` transitions to `Idle`
    /// instead of continuing the tool loop. Set by judge verdict tools
    /// (`task_complete`, `task_incomplete`) during `Streaming` or `Sending`.
    /// Machine-level flag that survives phase transitions.
    /// Self-clearing on read.
    tool_loop_disabled: bool,
}

impl SessionPhaseMachine {
    /// Create a new machine in the `Idle` phase.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Read-only access to the current phase.
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// The discriminant of the current phase.
    pub fn kind(&self) -> PhaseKind {
        self.phase.kind()
    }

    // ── Escape hatches (Phase 1 bridge) ──────────────────────────────

    /// Force the machine to Idle regardless of current state.
    ///
    /// Used during Phase 1 migration to support legacy methods that bypass
    /// the normal transition graph (e.g., `finish_sending` which does
    /// `Sending → Idle` — not a valid machine transition).
    ///
    /// Will be removed once all callers go through proper transitions.
    pub fn force_idle(&mut self) {
        self.phase = Phase::Idle(IdlePhase);
    }

    // ── Transitions ─────────────────────────────────────────────────────

    /// Disable the tool loop for this session's current turn.
    ///
    /// After the current tool batch completes, `on_tool_batch_completed`
    /// will transition to `Idle` instead of continuing the tool loop.
    /// Machine-level flag that survives phase transitions.
    /// The flag is consumed (cleared) by [`take_tool_loop_disabled`](Self::take_tool_loop_disabled).
    pub fn set_tool_loop_disabled(&mut self) {
        self.tool_loop_disabled = true;
    }

    /// Take the tool-loop-disabled flag, clearing it.
    ///
    /// Returns `true` if the tool loop was disabled, and clears the flag.
    pub fn take_tool_loop_disabled(&mut self) -> bool {
        std::mem::take(&mut self.tool_loop_disabled)
    }

    /// `Streaming → Idle` — hard cancel, returns old streaming data.
    ///
    /// Returns [`CancelOutcome`] which includes the old [`StreamingPhase`]
    /// data so the caller can force-exclude dangling tool calls and drain
    /// the queue to the input buffer.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `Streaming`.
    pub fn cancel(&mut self) -> Result<CancelOutcome, TransitionError> {
        let old = self.validate(PhaseKind::Streaming)?;
        let old_phase = std::mem::replace(&mut self.phase, Phase::Idle(IdlePhase));
        let Phase::Streaming(old_streaming) = old_phase else {
            return Ok(CancelOutcome {
                outcome: TransitionOutcome {
                    old_phase: old,
                    new_phase: PhaseKind::Idle,
                },
                old_streaming: StreamingPhase::default(),
            });
        };
        Ok(CancelOutcome {
            outcome: TransitionOutcome {
                old_phase: old,
                new_phase: PhaseKind::Idle,
            },
            old_streaming,
        })
    }

    /// Set soft cancel flag on the current `Streaming` phase.
    ///
    /// Does NOT transition — the flag is checked at the next stream-completion
    /// boundary (`on_stream_completed_tool_use` or `on_stream_completed_finished`).
    /// At that point, the transition goes to `Idle` instead of `Sending`.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `Streaming`.
    pub fn soft_cancel(&mut self) -> Result<(), TransitionError> {
        self.validate(PhaseKind::Streaming)?;
        if let Phase::Streaming(ref mut streaming) = self.phase {
            streaming.soft_cancel_requested = true;
        }
        Ok(())
    }

    // ── Phase data accessors ────────────────────────────────────────────

    /// Read-only access to `StreamingPhase` data, if currently streaming.
    pub fn streaming_phase(&self) -> Option<&StreamingPhase> {
        match &self.phase {
            Phase::Streaming(s) => Some(s),
            _ => None,
        }
    }

    /// Mutable access to `StreamingPhase` data, if currently streaming.
    pub fn streaming_phase_mut(&mut self) -> Option<&mut StreamingPhase> {
        match &mut self.phase {
            Phase::Streaming(s) => Some(s),
            _ => None,
        }
    }

    // ── Streaming state accessors ─────────────────────────────────────────

    /// The streaming assistant entry index, if streaming.
    pub fn streaming_entry_index(&self) -> Option<usize> {
        self.streaming_phase().and_then(|sp| sp.streaming_entry_index)
    }

    /// Set the streaming assistant entry index. No-op if not streaming.
    pub fn set_streaming_entry_index(&mut self, index: usize) {
        if let Some(sp) = self.streaming_phase_mut() {
            sp.streaming_entry_index = Some(index);
        }
    }

    /// The streaming thinking entry index, if streaming.
    pub fn streaming_thinking_entry_index(&self) -> Option<usize> {
        self.streaming_phase().and_then(|sp| sp.streaming_thinking_entry_index)
    }

    /// Set the streaming thinking entry index. No-op if not streaming.
    pub fn set_streaming_thinking_entry_index(&mut self, index: usize) {
        if let Some(sp) = self.streaming_phase_mut() {
            sp.streaming_thinking_entry_index = Some(index);
        }
    }

    /// Read-only access to tool-call tracking map. Returns empty if not streaming.
    pub fn streaming_tool_call_indices(&self) -> &std::collections::HashMap<usize, usize> {
        use std::sync::OnceLock;
        static EMPTY: OnceLock<std::collections::HashMap<usize, usize>> = OnceLock::new();
        self.streaming_phase()
            .map(|sp| &sp.streaming_tool_call_indices)
            .unwrap_or_else(|| EMPTY.get_or_init(std::collections::HashMap::new))
    }

    /// Mutable access to tool-call tracking map. Returns `None` if not streaming.
    pub fn streaming_tool_call_indices_mut(&mut self) -> Option<&mut std::collections::HashMap<usize, usize>> {
        self.streaming_phase_mut().map(|sp| &mut sp.streaming_tool_call_indices)
    }

    /// Read-only access to tool-result tracking map. Returns empty if not streaming.
    pub fn streaming_tool_result_indices(&self) -> &std::collections::HashMap<String, usize> {
        use std::sync::OnceLock;
        static EMPTY: OnceLock<std::collections::HashMap<String, usize>> = OnceLock::new();
        self.streaming_phase()
            .map(|sp| &sp.streaming_tool_result_indices)
            .unwrap_or_else(|| EMPTY.get_or_init(std::collections::HashMap::new))
    }

    /// Mutable access to tool-result tracking map. Returns `None` if not streaming.
    pub fn streaming_tool_result_indices_mut(&mut self) -> Option<&mut std::collections::HashMap<String, usize>> {
        self.streaming_phase_mut().map(|sp| &mut sp.streaming_tool_result_indices)
    }

    /// Shift all streaming indices >= `inserted_at` by +1.
    ///
    /// Called after `insert_entry_at` to keep indices valid.
    /// No-op if not streaming.
    pub fn shift_streaming_indices_for_insert_at(&mut self, inserted_at: usize) {
        let Some(sp) = self.streaming_phase_mut() else { return };
        if let Some(ref mut i) = sp.streaming_entry_index && *i >= inserted_at {
            *i += 1;
        }
        if let Some(ref mut i) = sp.streaming_thinking_entry_index && *i >= inserted_at {
            *i += 1;
        }
        for v in sp.streaming_tool_result_indices.values_mut() {
            if *v >= inserted_at {
                *v += 1;
            }
        }
        for key in sp.streaming_tool_call_indices.keys().copied().collect::<Vec<_>>() {
            if let Some(v) = sp.streaming_tool_call_indices.get_mut(&key) && *v >= inserted_at {
                *v += 1;
            }
        }
    }

    /// Whether the machine is tracking a tool call at the given history index.
    pub fn is_tool_call_at_history_index(&self, history_index: usize) -> bool {
        self.streaming_tool_call_indices().values().any(|&v| v == history_index)
    }

    /// Read-only access to `SendingPhase` data, if currently sending.
    #[expect(dead_code, reason = "will be used when SendingPhase carries state")]
    pub fn sending_phase(&self) -> Option<&SendingPhase> {
        match &self.phase {
            Phase::Sending(s) => Some(s),
            _ => None,
        }
    }

    /// Mutable access to `SendingPhase` data, if currently sending.
    #[expect(dead_code, reason = "will be used when SendingPhase carries state")]
    pub fn sending_phase_mut(&mut self) -> Option<&mut SendingPhase> {
        match &mut self.phase {
            Phase::Sending(s) => Some(s),
            _ => None,
        }
    }

    // ── Internal helpers ────────────────────────────────────────────────

    /// Validate the current phase, then swap to `next`.
    ///
    /// Returns [`TransitionOutcome`] recording the before/after phases.
    pub(in crate::feat::session::phase_machine) fn transition(
        &mut self,
        expected: PhaseKind,
        next: Phase,
    ) -> Result<TransitionOutcome, TransitionError> {
        let old = self.validate(expected)?;
        let new_phase = next.kind();
        self.phase = next;
        Ok(TransitionOutcome {
            old_phase: old,
            new_phase,
        })
    }

    /// Validate that the current phase matches the expected kind.
    pub(in crate::feat::session::phase_machine) fn validate(
        &self,
        expected: PhaseKind,
    ) -> Result<PhaseKind, TransitionError> {
        let actual = self.phase.kind();
        if actual == expected {
            Ok(actual)
        } else {
            Err(TransitionError { from: actual })
        }
    }
}
