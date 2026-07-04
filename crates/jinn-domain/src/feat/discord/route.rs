//! Phase-aware routing: pick the right bus command for an inbound message.
//!
//! Jinn's chat-input layer already applies this rule on the keyboard side: when
//! the session is `Idle`, a user message is enqueued (starts a new turn); when
//! it is mid-turn (`Streaming`/`Sending`), the message is *steered* — delivered
//! to the model as soon as possible without waiting for the turn to end. The
//! Discord bot is just another user, so it replicates the same branch.

use crate::feat::session::phase_machine::PhaseKind;

/// Which bus command an inbound Discord message should be sent as, given the
/// current session phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteDecision {
    /// Session is idle — the message starts a fresh turn via `EnqueueUserMessage`.
    Enqueue,
    /// Session is mid-turn — the message is steered via `SubmitSteeringMessage`.
    Steer,
}

/// Map a session's current phase to a routing decision.
///
/// `Idle` → [`RouteDecision::Enqueue`]; any mid-turn phase (`Sending`,
/// `Streaming`) → [`RouteDecision::Steer`].
#[must_use]
pub fn route_decision(phase: PhaseKind) -> RouteDecision {
    match phase {
        PhaseKind::Idle => RouteDecision::Enqueue,
        PhaseKind::Sending | PhaseKind::Streaming => RouteDecision::Steer,
    }
}

#[cfg(test)]
mod tests {
    use super::{RouteDecision, route_decision};
    use crate::feat::session::phase_machine::PhaseKind;

    #[test]
    fn idle_phase_routes_to_enqueue() {
        // Given an idle session phase.
        // When routing.
        let decision = route_decision(PhaseKind::Idle);
        // Then the message is enqueued (starts a new turn).
        assert_eq!(decision, RouteDecision::Enqueue);
    }

    #[test]
    fn streaming_phase_routes_to_steer() {
        // Given a streaming session phase.
        // When routing.
        let decision = route_decision(PhaseKind::Streaming);
        // Then the message is steered (delivered mid-turn).
        assert_eq!(decision, RouteDecision::Steer);
    }

    #[test]
    fn sending_phase_routes_to_steer() {
        // Given a sending session phase.
        // When routing.
        let decision = route_decision(PhaseKind::Sending);
        // Then the message is steered.
        assert_eq!(decision, RouteDecision::Steer);
    }
}
