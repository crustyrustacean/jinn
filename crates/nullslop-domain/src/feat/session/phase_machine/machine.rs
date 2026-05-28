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

use std::fmt;

use super::phase::{IdlePhase, Phase, PhaseKind, SendingPhase, StreamingPhase, TearingDownPhase};

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
#[derive(Debug)]
pub struct TransitionError {
    /// The phase the machine was in when the invalid transition was attempted.
    pub from: PhaseKind,
    /// The name of the transition method that was called.
    pub trigger: &'static str,
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid transition from {:?} via {}",
            self.from, self.trigger
        )
    }
}

impl std::error::Error for TransitionError {}

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
#[derive(Debug, Clone, Default)]
pub struct SessionPhaseMachine {
    phase: Phase,
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

    // ── Transitions ─────────────────────────────────────────────────────

    /// `Idle → Sending` — a message has been dispatched to the LLM.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `Idle`.
    pub fn on_dispatch_message(&mut self) -> Result<TransitionOutcome, TransitionError> {
        let old = self.require_kind(PhaseKind::Idle, "on_dispatch_message")?;
        self.phase = Phase::Sending(SendingPhase::default());
        Ok(TransitionOutcome {
            old_phase: old,
            new_phase: PhaseKind::Sending,
        })
    }

    /// `Sending → Streaming` — the first token has arrived from the LLM.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `Sending`.
    pub fn on_first_token(&mut self) -> Result<TransitionOutcome, TransitionError> {
        let old = self.require_kind(PhaseKind::Sending, "on_first_token")?;
        self.phase = Phase::Streaming(StreamingPhase::default());
        Ok(TransitionOutcome {
            old_phase: old,
            new_phase: PhaseKind::Streaming,
        })
    }

    /// `Streaming → Sending` — stream ended with tool use (continue tool loop).
    ///
    /// If `soft_cancel_requested` was set on the `StreamingPhase`, transitions
    /// to `Idle` instead of continuing the tool loop.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `Streaming`.
    pub fn on_stream_completed_tool_use(&mut self) -> Result<TransitionOutcome, TransitionError> {
        let old = self.require_kind(PhaseKind::Streaming, "on_stream_completed_tool_use")?;
        let Phase::Streaming(ref streaming) = self.phase else {
            unreachable!()
        };

        if streaming.soft_cancel_requested {
            self.phase = Phase::Idle(IdlePhase);
            Ok(TransitionOutcome {
                old_phase: old,
                new_phase: PhaseKind::Idle,
            })
        } else {
            self.phase = Phase::Sending(SendingPhase::default());
            Ok(TransitionOutcome {
                old_phase: old,
                new_phase: PhaseKind::Sending,
            })
        }
    }

    /// `Streaming → Idle` — stream ended normally (no tool use).
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `Streaming`.
    pub fn on_stream_completed_finished(&mut self) -> Result<TransitionOutcome, TransitionError> {
        let old = self.require_kind(PhaseKind::Streaming, "on_stream_completed_finished")?;
        self.phase = Phase::Idle(IdlePhase);
        Ok(TransitionOutcome {
            old_phase: old,
            new_phase: PhaseKind::Idle,
        })
    }

    /// `Streaming → Idle` — stream ended with an error.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `Streaming`.
    pub fn on_stream_completed_error(&mut self) -> Result<TransitionOutcome, TransitionError> {
        let old = self.require_kind(PhaseKind::Streaming, "on_stream_completed_error")?;
        self.phase = Phase::Idle(IdlePhase);
        Ok(TransitionOutcome {
            old_phase: old,
            new_phase: PhaseKind::Idle,
        })
    }

    /// `Streaming → Idle` — stream was canceled by the user.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `Streaming`.
    pub fn on_stream_completed_canceled(&mut self) -> Result<TransitionOutcome, TransitionError> {
        let old = self.require_kind(PhaseKind::Streaming, "on_stream_completed_canceled")?;
        self.phase = Phase::Idle(IdlePhase);
        Ok(TransitionOutcome {
            old_phase: old,
            new_phase: PhaseKind::Idle,
        })
    }

    /// `Sending → Streaming` or `Sending → Idle`.
    ///
    /// If `tool_loop_disabled` is set on the `SendingPhase`, transitions to
    /// `Idle` (tool loop stops). Otherwise transitions to `Streaming`
    /// (tool loop continues with the next LLM request).
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `Sending`.
    pub fn on_tool_batch_completed(&mut self) -> Result<TransitionOutcome, TransitionError> {
        let old = self.require_kind(PhaseKind::Sending, "on_tool_batch_completed")?;
        let Phase::Sending(ref sending) = self.phase else {
            unreachable!()
        };

        if sending.tool_loop_disabled {
            self.phase = Phase::Idle(IdlePhase);
            Ok(TransitionOutcome {
                old_phase: old,
                new_phase: PhaseKind::Idle,
            })
        } else {
            self.phase = Phase::Streaming(StreamingPhase::default());
            Ok(TransitionOutcome {
                old_phase: old,
                new_phase: PhaseKind::Streaming,
            })
        }
    }

    /// `Idle → TearingDown` — a lifecycle teardown script has started.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `Idle`.
    pub fn on_request_teardown(&mut self) -> Result<TransitionOutcome, TransitionError> {
        let old = self.require_kind(PhaseKind::Idle, "on_request_teardown")?;
        self.phase = Phase::TearingDown(TearingDownPhase);
        Ok(TransitionOutcome {
            old_phase: old,
            new_phase: PhaseKind::TearingDown,
        })
    }

    /// `TearingDown → Idle` — teardown script completed.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `TearingDown`.
    pub fn on_teardown_complete(&mut self) -> Result<TransitionOutcome, TransitionError> {
        let old = self.require_kind(PhaseKind::TearingDown, "on_teardown_complete")?;
        self.phase = Phase::Idle(IdlePhase);
        Ok(TransitionOutcome {
            old_phase: old,
            new_phase: PhaseKind::Idle,
        })
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
        let old = self.require_kind(PhaseKind::Streaming, "cancel")?;
        let old_phase = std::mem::replace(&mut self.phase, Phase::Idle(IdlePhase));
        let Phase::Streaming(old_streaming) = old_phase else {
            unreachable!()
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
        self.require_kind(PhaseKind::Streaming, "soft_cancel")?;
        let Phase::Streaming(ref mut streaming) = self.phase else {
            unreachable!()
        };
        streaming.soft_cancel_requested = true;
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

    /// Read-only access to `SendingPhase` data, if currently sending.
    pub fn sending_phase(&self) -> Option<&SendingPhase> {
        match &self.phase {
            Phase::Sending(s) => Some(s),
            _ => None,
        }
    }

    /// Mutable access to `SendingPhase` data, if currently sending.
    pub fn sending_phase_mut(&mut self) -> Option<&mut SendingPhase> {
        match &mut self.phase {
            Phase::Sending(s) => Some(s),
            _ => None,
        }
    }

    // ── Internal helpers ────────────────────────────────────────────────

    /// Validate that the current phase matches the expected kind.
    fn require_kind(
        &self,
        expected: PhaseKind,
        trigger: &'static str,
    ) -> Result<PhaseKind, TransitionError> {
        let actual = self.phase.kind();
        if actual == expected {
            Ok(actual)
        } else {
            Err(TransitionError {
                from: actual,
                trigger,
            })
        }
    }
}
