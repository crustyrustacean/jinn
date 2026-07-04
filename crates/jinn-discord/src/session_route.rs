//! Session routing helpers — shared between the gateway message handler and
//! tests.
//!
//! The pure routing decision (phase → enqueue vs steer) lives in
//! [`jinn_domain::feat::discord::route`]. This module adds the composite
//! inbound-message classifier that also accounts for thread binding and
//! session presence, so the gateway handler stays a thin adapter and the
//! decision logic is unit-testable without constructing Discord types.

use jinn_domain::feat::discord::route::{RouteDecision, route_decision};
use jinn_domain::feat::session::phase_machine::PhaseKind;

/// The outcome of classifying an inbound Discord message against the bot's
/// binding and session state.
///
/// Produced by [`classify_inbound`]; consumed by `handle_inbound_message` to
/// decide which (if any) bus command to publish and whether to reply on
/// Discord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundOutcome {
    /// No thread→session mapping exists for this channel. Do nothing — no
    /// Discord reply, no publish. `/new` is the only entry point, and the
    /// `/new` wizard's own `MessageCollector` reads its replies from a
    /// separate source, so silence here avoids racing the wizard.
    UnboundNoOp,
    /// The thread is bound and the session is present in `State` and idle.
    /// Publish `EnqueueUserMessage` to start a new turn.
    Enqueue,
    /// The thread is bound and the session is present and mid-turn. Publish
    /// `SubmitSteeringMessage` to deliver the message to the model ASAP.
    Steer,
    /// The thread is bound but the session is not in `State` (e.g. manually
    /// archived, or startup load failed). Publish `SessionLoadRequested` and
    /// ask the user to resend — never enqueue against a missing session, which
    /// would create a throwaway that the subsequent load overwrites.
    LoadMissing,
}

/// Classify an inbound message given its binding and session presence.
///
/// - `bound` — whether a thread→session mapping exists for the channel.
/// - `phase` — the bound session's current phase, or `None` if the session is
///   not present in `State`.
///
/// Decision table:
///
/// | bound | phase              | outcome      |
/// |-------|--------------------|--------------|
/// | false | _                  | `UnboundNoOp`|
/// | true  | None               | `LoadMissing`|
/// | true  | Some(Idle)         | `Enqueue`    |
/// | true  | Some(Sending/...)  | `Steer`      |
#[must_use]
pub fn classify_inbound(bound: bool, phase: Option<PhaseKind>) -> InboundOutcome {
    if !bound {
        return InboundOutcome::UnboundNoOp;
    }
    match phase {
        None => InboundOutcome::LoadMissing,
        Some(p) => match route_decision(p) {
            RouteDecision::Enqueue => InboundOutcome::Enqueue,
            RouteDecision::Steer => InboundOutcome::Steer,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{InboundOutcome, classify_inbound};
    use jinn_domain::feat::session::phase_machine::PhaseKind;

    #[test]
    fn unbound_channel_is_no_op() {
        // Given an unbound channel (no thread→session mapping).
        // When classifying.
        let outcome = classify_inbound(false, None);
        // Then it is a silent no-op regardless of phase.
        assert_eq!(outcome, InboundOutcome::UnboundNoOp);
    }

    #[test]
    fn unbound_channel_ignores_phase() {
        // Given an unbound channel even if a phase were somehow provided.
        // When classifying.
        let outcome = classify_inbound(false, Some(PhaseKind::Idle));
        // Then it is still a silent no-op.
        assert_eq!(outcome, InboundOutcome::UnboundNoOp);
    }

    #[test]
    fn bound_idle_session_enqueues() {
        // Given a bound channel with an idle session present.
        // When classifying.
        let outcome = classify_inbound(true, Some(PhaseKind::Idle));
        // Then the message should enqueue (start a new turn).
        assert_eq!(outcome, InboundOutcome::Enqueue);
    }

    #[test]
    fn bound_streaming_session_steers() {
        // Given a bound channel with a streaming session.
        // When classifying.
        let outcome = classify_inbound(true, Some(PhaseKind::Streaming));
        // Then the message should steer (deliver mid-turn).
        assert_eq!(outcome, InboundOutcome::Steer);
    }

    #[test]
    fn bound_sending_session_steers() {
        // Given a bound channel with a sending session.
        // When classifying.
        let outcome = classify_inbound(true, Some(PhaseKind::Sending));
        // Then the message should steer.
        assert_eq!(outcome, InboundOutcome::Steer);
    }

    #[test]
    fn bound_missing_session_loads() {
        // Given a bound channel whose session is not in State.
        // When classifying.
        let outcome = classify_inbound(true, None);
        // Then it requests a load and asks the user to resend.
        assert_eq!(outcome, InboundOutcome::LoadMissing);
    }
}
