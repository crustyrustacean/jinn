//! Per-session state machine for the LLM actor.
//!
//! Each active LLM conversation is tracked by a [`SessionData`] instance
//! that records the current state and stream data.

/// Per-session state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionState {
    /// No active streaming.
    Idle,
    /// Streaming tokens from the LLM.
    Streaming,
}

/// Per-session data tracked by the actor.
pub(crate) struct SessionData {
    /// Current state in the streaming lifecycle.
    pub(crate) state: SessionState,
}

impl SessionData {
    /// Creates a new [`SessionData`] in Idle state.
    pub(crate) fn new() -> Self {
        Self {
            state: SessionState::Idle,
        }
    }
}
