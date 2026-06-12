//! Session phase changed event.
//!
//! Emitted by the session actor whenever the session phase transitions
//! (e.g., Idle → Sending, Sending → Streaming, Streaming → Idle).
//!
//! The QueueActor subscribes to this event to react to `Idle` transitions
//! and pop the turn dispatch queue.

use serde::{Deserialize, Serialize};

use crate::feat::session::phase_machine::PhaseKind;
use crate::protocol::SessionId;

/// Session phase transitioned to a new state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionPhaseChanged {
    /// The session whose phase changed.
    pub session_id: SessionId,
    /// The phase before the transition.
    pub old_phase: PhaseKind,
    /// The new phase after the transition.
    pub new_phase: PhaseKind,
}

impl crate::common::bus::BusMessage for SessionPhaseChanged {}
