//! Validated session phase transitions and per-phase runtime state.

mod machine;
mod phase;
mod transition;

pub use machine::{CancelOutcome, SessionPhaseMachine, TransitionError, TransitionOutcome};
pub use phase::{IdlePhase, Phase, SendingPhase, StreamingPhase};
pub use transition::PhaseTransitions;

#[cfg(test)]
mod tests;
