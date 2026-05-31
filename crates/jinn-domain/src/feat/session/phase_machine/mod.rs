//! Session phase machine - validated phase transitions with per-phase state.
//!
//! The [`SessionPhaseMachine`] owns the current [`Phase`] and enforces
//! that all transitions are valid. Each transition method is named after
//! the triggering event and returns [`Result<TransitionOutcome, TransitionError>`].
//!
//! See [`machine`](super::machine) module for the transition graph and
//! detailed documentation.

mod machine;
mod phase;
mod transition;

pub use machine::{CancelOutcome, SessionPhaseMachine, TransitionError, TransitionOutcome};
pub use phase::{IdlePhase, Phase, PhaseKind, PhaseKindParseError, SendingPhase, StreamingPhase, WorkingPhase};
pub use transition::PhaseTransitions;

#[cfg(test)]
mod tests;
