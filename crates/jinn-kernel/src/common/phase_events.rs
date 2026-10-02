//! Publishing a session phase change and the working state it implies.
//!
//! Two independent parts of the codebase mutate a session's phase: the
//! session actor (streaming, resume, enqueue) and the turn-dispatch queue
//! actor (drain, dispatch). Both must announce the transition, and both must
//! announce it the same way — a subscriber that sees one and not the other
//! records a session that went busy with no event, and one that sees them
//! disagree records two boundaries for one turn.
//!
//! So the pairing lives here rather than in either actor: one timestamp,
//! read once, carried by both events.

use jiff::Timestamp;
use jinn_core_types::SessionId;
use jinn_session_msg::PhaseKind;
use jinn_session_msg::SessionPhaseChanged;
use jinn_session_msg::WorkStateChanged;

use crate::common::services::bus_service::BusService;

/// Publishes `SessionPhaseChanged` and its `WorkStateChanged` consequence,
/// stamped with one shared moment.
///
/// The working flag is derived from `new_phase` rather than passed in, so a
/// caller cannot announce a phase change and a working state that disagree.
pub async fn publish_phase_change(
    bus: &BusService,
    session_id: &SessionId,
    old_phase: PhaseKind,
    new_phase: PhaseKind,
) {
    let at = Timestamp::now();
    bus.publish(SessionPhaseChanged {
        session_id: session_id.clone(),
        old_phase,
        new_phase,
        at,
    })
    .await;
    bus.publish(WorkStateChanged {
        session_id: session_id.clone(),
        working: new_phase.is_working(),
        at,
    })
    .await;
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unreachable,
        reason = "test code"
    )]
    use super::*;
    use jinn_slices::bus::BusAudit;

    fn recording() -> (BusService, BusAudit) {
        BusService::new_recording()
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn idle_to_sending_publishes_working_true() {
        // Given a recording bus.
        let (bus, audit) = recording();

        // When publishing an Idle -> Sending phase change.
        let id = SessionId::new();
        publish_phase_change(&bus, &id, PhaseKind::Idle, PhaseKind::Sending).await;

        // Then the working state says the session is working.
        let events = audit.of_type::<WorkStateChanged>();
        assert_eq!(events.len(), 1, "{events:?}");
        assert!(events[0].working, "{events:?}");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn streaming_to_idle_publishes_working_false() {
        // Given a recording bus.
        let (bus, audit) = recording();

        // When publishing a Streaming -> Idle phase change.
        let id = SessionId::new();
        publish_phase_change(&bus, &id, PhaseKind::Streaming, PhaseKind::Idle).await;

        // Then the working state says the session stopped.
        let events = audit.of_type::<WorkStateChanged>();
        assert_eq!(events.len(), 1, "{events:?}");
        assert!(!events[0].working, "{events:?}");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn both_events_carry_the_same_moment() {
        // Given a recording bus.
        let (bus, audit) = recording();

        // When publishing a phase change.
        let id = SessionId::new();
        publish_phase_change(&bus, &id, PhaseKind::Idle, PhaseKind::Sending).await;

        // Then the phase event and the working event agree on the moment, so
        // every subscriber measures the same boundary.
        let phases = audit.of_type::<SessionPhaseChanged>();
        let work = audit.of_type::<WorkStateChanged>();
        assert_eq!(phases.len(), 1, "{phases:?}");
        assert_eq!(work.len(), 1, "{work:?}");
        assert_eq!(phases[0].at, work[0].at, "{phases:?} / {work:?}");
    }
}
