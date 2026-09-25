//! Compatibility exports for the session phase machine.
//!
//! The implementation is owned by `jinn-session-msg`; this path preserves
//! existing kernel and downstream imports during the state-layer migration.

pub use jinn_session_msg::phase_machine::{
    CancelOutcome, IdlePhase, Phase, PhaseTransitions, SendingPhase, SessionPhaseMachine,
    StreamingPhase, TransitionError, TransitionOutcome,
};
pub use jinn_session_msg::{PhaseKind, PhaseKindParseError};
