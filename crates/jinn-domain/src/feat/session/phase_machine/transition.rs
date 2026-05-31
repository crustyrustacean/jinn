use super::machine::{SessionPhaseMachine, TransitionError, TransitionOutcome};
use super::phase::{IdlePhase, Phase, PhaseKind, SendingPhase, StreamingPhase, WorkingPhase};

/// Transition methods for [`SessionPhaseMachine`].
///
/// Each method is named after the event that triggers the transition.
/// The machine validates the current phase and returns [`TransitionError`]
/// if the transition is not valid from the current state.
///
/// `cancel()` and `soft_cancel()` are on the machine itself because they
/// need direct access to private phase data.
pub trait PhaseTransitions {
    /// `Idle → Sending` - a message has been dispatched to the LLM.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `Idle`.
    fn on_dispatch_message(&mut self) -> Result<TransitionOutcome, TransitionError>;

    /// `Sending → Streaming` - the first token has arrived from the LLM.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `Sending`.
    fn on_first_token(&mut self) -> Result<TransitionOutcome, TransitionError>;

    /// `Streaming → Sending` - stream ended with tool use (continue tool loop).
    ///
    /// If `soft_cancel_requested` was set on the `StreamingPhase`, transitions
    /// to `Idle` instead of continuing the tool loop.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `Streaming`.
    fn on_stream_completed_tool_use(&mut self) -> Result<TransitionOutcome, TransitionError>;

    /// `Streaming → Idle` - stream ended normally (no tool use).
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `Streaming`.
    fn on_stream_completed_finished(&mut self) -> Result<TransitionOutcome, TransitionError>;

    /// `Streaming → Idle` - stream ended with an error.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `Streaming`.
    fn on_stream_completed_error(&mut self) -> Result<TransitionOutcome, TransitionError>;

    /// `Streaming → Idle` - stream was canceled by the user.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `Streaming`.
    fn on_stream_completed_canceled(&mut self) -> Result<TransitionOutcome, TransitionError>;

    /// `Sending → Streaming` or `Sending → Idle`.
    ///
    /// If `tool_loop_disabled` is set on the `SendingPhase`, transitions to
    /// `Idle` (tool loop stops). Otherwise transitions to `Streaming`
    /// (tool loop continues with the next LLM request).
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `Sending`.
    fn on_tool_batch_completed(&mut self) -> Result<TransitionOutcome, TransitionError>;

    /// `Idle → Working` or `Working → Working` (increment) - a background operation started.
    ///
    /// If currently `Idle`, transitions to `Working` with count = 1.
    /// If already `Working`, increments the count (stays in `Working`).
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `Idle` or `Working`.
    fn on_start_working(&mut self) -> Result<TransitionOutcome, TransitionError>;

    /// `Working → Working` (decrement) or `Working → Idle` (count hits zero) - one operation completed.
    ///
    /// Returns `None` if not currently `Working`.
    /// Returns `Some(TransitionOutcome)` with `new_phase == Idle` when the count reaches zero.
    /// Returns `Some(TransitionOutcome)` with `new_phase == Working` when count is still > 0.
    fn on_working_complete(&mut self) -> Option<TransitionOutcome>;

    /// `Working → Idle` - hard cancel, resets count to zero.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] if not in `Working`.
    fn cancel_working(&mut self) -> Result<TransitionOutcome, TransitionError>;
}

impl PhaseTransitions for SessionPhaseMachine {
    fn on_dispatch_message(&mut self) -> Result<TransitionOutcome, TransitionError> {
        self.transition(PhaseKind::Idle, Phase::Sending(SendingPhase))
    }

    fn on_first_token(&mut self) -> Result<TransitionOutcome, TransitionError> {
        self.transition(
            PhaseKind::Sending,
            Phase::Streaming(StreamingPhase::default()),
        )
    }

    fn on_stream_completed_tool_use(&mut self) -> Result<TransitionOutcome, TransitionError> {
        let soft_cancel = self
            .streaming_phase()
            .is_some_and(|sp| sp.soft_cancel_requested);

        let next = if soft_cancel {
            Phase::Idle(IdlePhase)
        } else {
            Phase::Sending(SendingPhase)
        };
        self.transition(PhaseKind::Streaming, next)
    }

    fn on_stream_completed_finished(&mut self) -> Result<TransitionOutcome, TransitionError> {
        self.transition(PhaseKind::Streaming, Phase::Idle(IdlePhase))
    }

    fn on_stream_completed_error(&mut self) -> Result<TransitionOutcome, TransitionError> {
        self.transition(PhaseKind::Streaming, Phase::Idle(IdlePhase))
    }

    fn on_stream_completed_canceled(&mut self) -> Result<TransitionOutcome, TransitionError> {
        self.transition(PhaseKind::Streaming, Phase::Idle(IdlePhase))
    }

    fn on_tool_batch_completed(&mut self) -> Result<TransitionOutcome, TransitionError> {
        let disabled = self.take_tool_loop_disabled();

        let next = if disabled {
            Phase::Idle(IdlePhase)
        } else {
            Phase::Streaming(StreamingPhase::default())
        };
        self.transition(PhaseKind::Sending, next)
    }

    fn on_start_working(&mut self) -> Result<TransitionOutcome, TransitionError> {
        match self.kind() {
            PhaseKind::Idle => self.transition(
                PhaseKind::Idle,
                Phase::Working(WorkingPhase { count: 1 }),
            ),
            PhaseKind::Working => {
                if let Phase::Working(ref mut wp) = self.phase {
                    wp.count += 1;
                }
                Ok(TransitionOutcome {
                    old_phase: PhaseKind::Working,
                    new_phase: PhaseKind::Working,
                })
            }
            other => Err(TransitionError { from: other }),
        }
    }

    fn on_working_complete(&mut self) -> Option<TransitionOutcome> {
        if self.kind() != PhaseKind::Working {
            return None;
        }
        if let Phase::Working(ref mut wp) = self.phase {
            if wp.count == 0 {
                tracing::warn!("on_working_complete called with count already zero");
            } else {
                wp.count -= 1;
            }
            if wp.count == 0 {
                let old_phase = PhaseKind::Working;
                self.phase = Phase::Idle(IdlePhase);
                return Some(TransitionOutcome {
                    old_phase,
                    new_phase: PhaseKind::Idle,
                });
            }
        }
        Some(TransitionOutcome {
            old_phase: PhaseKind::Working,
            new_phase: PhaseKind::Working,
        })
    }

    fn cancel_working(&mut self) -> Result<TransitionOutcome, TransitionError> {
        let old = self.validate(PhaseKind::Working)?;
        self.phase = Phase::Idle(IdlePhase);
        Ok(TransitionOutcome {
            old_phase: old,
            new_phase: PhaseKind::Idle,
        })
    }
}
