//! Session crossing contracts.
//!
//! The EXPORT surface of the session family: the discriminant and
//! lifecycle events that cross the core bridge (trouper
//! topic). Kernel publishers (session actors) and slice consumers
//! (e.g. the discord bridge) both depend on this crate — the types
//! have exactly one home.
//!
//! The crate publishes shared session vocabulary: phase values and the
//! validated phase machine, plus lifecycle events consumed across slices.
//! Persistence commands remain owned by their implementation boundary.

use jinn_core_types::SessionId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod phase_machine;
pub mod session_origin;
mod session_seed;

pub use session_origin::SessionOrigin;
pub use session_seed::SessionSeed;

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
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "A session's phase transitioned.")]
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
/// Emitted by the session-lifecycle actor after running a lifecycle
/// setup command. On success, `cwd` is the directory reported by the
/// command. On failure, `cwd` is the default CWD and `error` contains
/// the failure details.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "A session's setup command completed (success or failure).")]
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
/// Emitted by the session-lifecycle actor after running a lifecycle
/// teardown command. On success, the session has already been removed
/// from the sessions map. On failure, the session is still open and
/// `error` describes the problem.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "A session's teardown command finished (success or failure).")]
pub struct SessionTeardownFinished {
    /// The session that was being torn down.
    pub session_id: SessionId,
    /// Error message if teardown failed.
    pub error: Option<String>,
}

/// Session archived in persistent storage.
///
/// Emitted by the session-store actor after marking a session as archived in
/// SQLite. Emitted before the session-closed event so consumers can distinguish
/// archived closes from empty-session closes.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "A session was archived in persistent storage.")]
pub struct SessionArchived {
    /// The session that was archived.
    pub session_id: SessionId,
}

/// Mark a session as having been interacted with by the user.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Mark a session as interacted by the user.")]
pub struct MarkSessionInteracted {
    /// The session the user interacted with.
    pub session_id: SessionId,
}

/// Emitted after a session records its first user interaction.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "A session recorded its first user interaction.")]
pub struct UserInteracted {
    /// The session that was interacted with.
    pub session_id: SessionId,
}

/// Re-dispatch a turn whose in-flight provider stream stalled.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Command)]
#[schema(description = "Re-dispatch a turn whose stream stalled.")]
pub struct RetryStalledSession {
    /// The session whose turn has stalled.
    pub session_id: SessionId,
    /// The one-based restart attempt within the current stall lineage.
    pub attempt: u32,
    /// The restart budget enforced by the watchdog.
    pub max_restarts: u32,
}

/// Emitted after a session's close workflow has completed.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "A session close workflow completed.")]
pub struct SessionClosed {
    /// The session that was closed.
    pub session_id: SessionId,
}

/// Emitted immediately after a session is removed from the live session map.
#[derive(Debug, Clone, Serialize, Deserialize, trouper::schema::Event)]
#[schema(description = "A session was removed from the live session map.")]
pub struct SessionRemoved {
    /// The session removed from the live map.
    pub session_id: SessionId,
    /// The removed session's persisted direct parent, captured before deletion.
    pub removed_parent: Option<SessionId>,
}

// ── wire contracts ──────────────────────────────────────────────────

impl jinn_slices::BusMessage for PhaseKind {}
impl jinn_slices::BusMessage for MarkSessionInteracted {}
impl jinn_slices::BusMessage for RetryStalledSession {}
impl jinn_slices::BusMessage for SessionClosed {}
impl jinn_slices::BusMessage for SessionRemoved {}
impl jinn_slices::BusMessage for SessionPhaseChanged {}
impl jinn_slices::BusMessage for SessionSetupCompleted {}
impl jinn_slices::BusMessage for SessionTeardownFinished {}
impl jinn_slices::BusMessage for SessionArchived {}
impl jinn_slices::BusMessage for UserInteracted {}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]

    use super::MarkSessionInteracted;
    use super::PhaseKind;
    use super::RetryStalledSession;
    use super::SessionArchived;
    use super::SessionClosed;
    use super::SessionPhaseChanged;
    use super::SessionRemoved;
    use super::SessionSetupCompleted;
    use super::SessionTeardownFinished;
    use super::UserInteracted;
    use jinn_core_types::SessionId;
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
        let round: (
            SessionPhaseChanged,
            SessionSetupCompleted,
            SessionTeardownFinished,
            SessionArchived,
        ) = serde_json::from_str(&json).unwrap();

        // Then every event survives with its fields intact.
        assert_eq!(round.0.new_phase, PhaseKind::Idle);
        assert_eq!(round.1.cwd, PathBuf::from("/repo"));
        assert_eq!(round.1.error.as_deref(), Some("boom"));
        assert_eq!(round.2.error, None);
        assert_eq!(round.3.session_id, round.0.session_id);
    }

    #[rstest::rstest]
    #[test]
    fn promoted_session_contracts_roundtrip_through_json() {
        // Given one of each promoted session command and event.
        let id = jinn_core_types::SessionId::new();
        let removed_parent = SessionId::new();
        let contracts = (
            MarkSessionInteracted {
                session_id: id.clone(),
            },
            UserInteracted {
                session_id: id.clone(),
            },
            RetryStalledSession {
                session_id: id.clone(),
                attempt: 2,
                max_restarts: 5,
            },
            SessionClosed {
                session_id: id.clone(),
            },
            SessionRemoved {
                session_id: id.clone(),
                removed_parent: Some(removed_parent.clone()),
            },
        );

        // When serializing and deserializing the wire tuple.
        let json = serde_json::to_string(&contracts).unwrap();
        let restored = serde_json::from_str::<(
            MarkSessionInteracted,
            UserInteracted,
            RetryStalledSession,
            SessionClosed,
            SessionRemoved,
        )>(&json)
        .unwrap();

        // Then every moved payload survives unchanged.
        assert_eq!(restored.0.session_id, id);
        assert_eq!(restored.1.session_id, id);
        assert_eq!(restored.2.session_id, id);
        assert_eq!(restored.2.attempt, 2);
        assert_eq!(restored.2.max_restarts, 5);
        assert_eq!(restored.3.session_id, id);
        assert_eq!(restored.4.session_id, id);
        assert_eq!(restored.4.removed_parent, Some(removed_parent));
    }
}
