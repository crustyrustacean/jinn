//! Session crossing contracts.
//!
//! The EXPORT surface of the session family: the discriminant and
//! lifecycle events that cross the core bridge (kameo bus → trouper
//! topic). Kernel publishers (session actors) and slice consumers
//! (e.g. the discord bridge) both depend on this crate — the types
//! have exactly one home.
//!
//! The crate stays narrow: only types that at least one slice consumes
//! move here. Kernel-only session machinery (the [`Phase`] state
//! machine, persistence commands) remains in `jinn-domain`.
//!
//! [`Phase`]: jinn_domain::feat::session::phase_machine::Phase

use jinn_core_types::SessionId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── phase discriminant ──────────────────────────────────────────────

/// The discriminant of a session's phase — used for event emission and
/// logging where the per-phase data is not needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhaseKind {
    /// Session is idle — no LLM request in flight.
    Idle,
    /// A message has been dispatched to the LLM but no tokens have
    /// arrived yet.
    Sending,
    /// LLM tokens are actively streaming into the session.
    Streaming,
}

impl std::str::FromStr for PhaseKind {
    type Err = PhaseKindParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "idle" => Ok(Self::Idle),
            "sending" => Ok(Self::Sending),
            "streaming" => Ok(Self::Streaming),
            _ => Err(PhaseKindParseError(s.to_owned())),
        }
    }
}

/// Error returned when a string does not match any [`PhaseKind`] variant.
#[derive(Debug, wherror::Error)]
#[error("unknown phase kind: {0}")]
pub struct PhaseKindParseError(String);

// ── session events (jinn bus → crossing topics) ─────────────────────

/// Session phase transitioned to a new state.
///
/// Emitted by the session actor whenever the session phase transitions
/// (e.g., Idle → Sending, Sending → Streaming, Streaming → Idle).
///
/// The QueueActor subscribes to this event to react to `Idle`
/// transitions and pop the turn dispatch queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionPhaseChanged {
    /// The session whose phase changed.
    pub session_id: SessionId,
    /// The phase before the transition.
    pub old_phase: PhaseKind,
    /// The new phase after the transition.
    pub new_phase: PhaseKind,
}

/// Setup command completed (success or failure).
///
/// Emitted by the session-persistence actor after running a lifecycle
/// setup command. On success, `cwd` is the directory reported by the
/// command. On failure, `cwd` is the default CWD and `error` contains
/// the failure details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSetupCompleted {
    /// The session that was being set up.
    pub session_id: SessionId,
    /// The resulting CWD on success, or default CWD on failure.
    pub cwd: PathBuf,
    /// Error message if setup failed.
    pub error: Option<String>,
}

/// Teardown command finished (success or failure).
///
/// Emitted by the session-persistence actor after running a lifecycle
/// teardown command. On success, the session has already been removed
/// from the sessions map. On failure, the session is still open and
/// `error` describes the problem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTeardownFinished {
    /// The session that was being torn down.
    pub session_id: SessionId,
    /// Error message if teardown failed.
    pub error: Option<String>,
}

/// Session archived in persistent storage.
///
/// Emitted by the session-persistence actor after marking a session as
/// archived in SQLite. Emitted before the session-closed event so
/// consumers can distinguish archived closes from empty-session closes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionArchived {
    /// The session that was archived.
    pub session_id: SessionId,
}

// ── wire contracts ──────────────────────────────────────────────────

/// The trouper topic the session family's events cross on.
#[must_use]
pub fn session_topic() -> trouper::types::Topic {
    trouper::types::Topic::new("jinn.session")
}

impl jinn_slices::BusMessage for PhaseKind {}
impl jinn_slices::BusMessage for SessionPhaseChanged {}
impl jinn_slices::BusMessage for SessionSetupCompleted {}
impl jinn_slices::BusMessage for SessionTeardownFinished {}
impl jinn_slices::BusMessage for SessionArchived {}

jinn_slices::crossing_schema!(SessionPhaseChanged, "SessionPhaseChanged",
    trouper::schema::SchemaKind::Event,
    description: "A session's phase transitioned.",
    fields: [
        "session_id" => trouper::schema::FieldTy::Uuid,
        "old_phase" => trouper::schema::FieldTy::Str,
        "new_phase" => trouper::schema::FieldTy::Str,
    ]);

jinn_slices::crossing_schema!(SessionSetupCompleted, "SessionSetupCompleted",
    trouper::schema::SchemaKind::Event,
    description: "A session's setup command completed (success or failure).",
    fields: [
        "session_id" => trouper::schema::FieldTy::Uuid,
        "cwd" => trouper::schema::FieldTy::Str,
        "error" => trouper::schema::FieldTy::Str,
    ]);

jinn_slices::crossing_schema!(SessionTeardownFinished, "SessionTeardownFinished",
    trouper::schema::SchemaKind::Event,
    description: "A session's teardown command finished (success or failure).",
    fields: [
        "session_id" => trouper::schema::FieldTy::Uuid,
        "error" => trouper::schema::FieldTy::Str,
    ]);

jinn_slices::crossing_schema!(SessionArchived, "SessionArchived",
    trouper::schema::SchemaKind::Event,
    description: "A session was archived in persistent storage.",
    fields: ["session_id" => trouper::schema::FieldTy::Uuid]);

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test code")]

    use super::PhaseKind;
    use super::SessionArchived;
    use super::SessionPhaseChanged;
    use super::SessionSetupCompleted;
    use super::SessionTeardownFinished;
    use std::path::PathBuf;
    use std::str::FromStr;

    #[rstest::rstest]
    #[case("idle", PhaseKind::Idle)]
    #[case("SENDING", PhaseKind::Sending)]
    #[case("streaming", PhaseKind::Streaming)]
    fn phase_kind_parses_case_insensitively(#[case] raw: &str, #[case] expected: PhaseKind) {
        // Given a phase-kind string in any case.
        // When parsing.
        let parsed = PhaseKind::from_str(raw);
        // Then the variant matches.
        assert_eq!(parsed.expect("parses"), expected);
    }

    #[rstest::rstest]
    #[test]
    fn phase_kind_rejects_unknown_strings() {
        // Given a string that is no phase.
        // When parsing.
        let parsed = PhaseKind::from_str("paused");
        // Then parsing fails.
        assert!(parsed.is_err());
    }

    #[rstest::rstest]
    #[test]
    fn session_events_roundtrip_through_json() {
        // Given one of each moved event.
        let id = jinn_core_types::SessionId::new();
        let events = (
            SessionPhaseChanged {
                session_id: id.clone(),
                old_phase: PhaseKind::Streaming,
                new_phase: PhaseKind::Idle,
            },
            SessionSetupCompleted {
                session_id: id.clone(),
                cwd: PathBuf::from("/repo"),
                error: Some("boom".to_owned()),
            },
            SessionTeardownFinished {
                session_id: id.clone(),
                error: None,
            },
            SessionArchived { session_id: id },
        );

        // When serializing and deserializing the tuple.
        let json = serde_json::to_string(&events).unwrap();
        let round: (SessionPhaseChanged, SessionSetupCompleted, SessionTeardownFinished, SessionArchived) =
            serde_json::from_str(&json).unwrap();

        // Then every event survives with its fields intact.
        assert_eq!(round.0.new_phase, PhaseKind::Idle);
        assert_eq!(round.1.cwd, PathBuf::from("/repo"));
        assert_eq!(round.1.error.as_deref(), Some("boom"));
        assert_eq!(round.2.error, None);
        assert_eq!(round.3.session_id, round.0.session_id);
    }
}
