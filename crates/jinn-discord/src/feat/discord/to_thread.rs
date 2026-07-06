//! Pure decision logic for the "to-thread" reverse flow (jinn → Discord).
//!
//! The gateway's [`GatewayRequest::CreateThreadForSession`] handler must decide
//! whether to actually create a new Discord thread or reject the request because
//! the session is already bound to one. That decision is a pure function of one
//! boolean — "is there an existing mapping for this session?" — so it lives here,
//! free of serenity / DB / I/O, where it can be unit-tested in isolation.

use jinn_domain::feat::discord::protocol::{CreateThreadReason, GatewayRequest};

/// The gateway's verdict for a `to-thread` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToThreadDecision {
    /// No existing binding — the gateway may proceed to create a thread.
    Proceed,
    /// The session is already bound to a thread — reject without creating.
    AlreadyBound,
}

/// Pure routing decision: should the gateway proceed with thread creation?
///
/// Returns [`ToThreadDecision::AlreadyBound`] when a mapping already exists for
/// the session (whether created by `/new` or a prior `to-thread`). This avoids
/// orphaning the previous Discord thread and prevents duplicate threads when the
/// user mashes `gdc`. The decision itself carries no I/O so it can be tested
/// directly.
///
/// # Errors
///
/// This function is infallible. The error reporting (`CreateThreadReason`)
/// is derived downstream from the decision plus the side-effecting steps.
pub fn to_thread_decision(existing_binding: bool) -> ToThreadDecision {
    if existing_binding {
        ToThreadDecision::AlreadyBound
    } else {
        ToThreadDecision::Proceed
    }
}

/// Map a decision plus a refusal context to the reason surfaced to the user.
///
/// Only the `AlreadyBound` decision produces a reason today; the proceed path
/// produces reasons from the side-effecting steps (serenity / mapping write),
/// not from this decision.
#[must_use]
pub fn refusal_reason(decision: &ToThreadDecision) -> Option<CreateThreadReason> {
    match decision {
        ToThreadDecision::Proceed => None,
        ToThreadDecision::AlreadyBound => Some(CreateThreadReason::AlreadyBound),
    }
}

/// Convenience: does the given request name a `to-thread` create request?
#[must_use]
pub fn is_create_thread_request(req: &GatewayRequest) -> bool {
    matches!(req, GatewayRequest::CreateThreadForSession { .. })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use super::*;

    #[rstest::rstest]
    #[case(true, ToThreadDecision::AlreadyBound)]
    #[case(false, ToThreadDecision::Proceed)]
    fn decision_matches_existing_binding(
        #[case] existing_binding: bool,
        #[case] expected: ToThreadDecision,
    ) {
        // Given a binding-exists flag.
        // When computing the decision.
        let decision = to_thread_decision(existing_binding);
        // Then it matches the expectation.
        assert_eq!(decision, expected);
    }

    #[test]
    fn refusal_reason_is_some_only_when_already_bound() {
        // Given an AlreadyBound decision.
        // When asking for its refusal reason.
        let reason = refusal_reason(&ToThreadDecision::AlreadyBound);
        // Then it surfaces AlreadyBound.
        assert_eq!(reason, Some(CreateThreadReason::AlreadyBound));
    }

    #[test]
    fn refusal_reason_is_none_when_proceeding() {
        // Given a Proceed decision.
        // When asking for its refusal reason.
        let reason = refusal_reason(&ToThreadDecision::Proceed);
        // Then there is no reason to surface.
        assert!(reason.is_none());
    }

    // ── Regression: re-`gdc` on an already-bound session ────────────────────
    //
    // Two origin cases converge on the same decision (both surface as
    // `get_thread_by_session` returning Some). Downstream of the decision the
    // gateway takes the early-return in `handle_create_thread` before any
    // serenity create / mapping write, so no new thread is created and the
    // existing mapping is left untouched. These tests pin that invariant at the
    // pure decision seam — the only layer reachable without a live Discord
    // connection.

    #[test]
    fn regression_session_bound_via_new_then_gdc_refuses_without_rebind() {
        // Given a session already bound via `/new` (a mapping exists).
        let existing_binding = true;

        // When the gateway computes the to-thread decision.
        let decision = to_thread_decision(existing_binding);

        // Then it refuses, and carries the AlreadyBound reason — the gate that
        // short-circuits thread creation and leaves the existing mapping intact.
        assert_eq!(decision, ToThreadDecision::AlreadyBound);
        assert_eq!(
            refusal_reason(&decision),
            Some(CreateThreadReason::AlreadyBound)
        );
    }

    #[test]
    fn regression_session_bound_via_prior_gdc_then_gdc_again_refuses_without_rebind() {
        // Given a session already bound via a prior `gdc` (a mapping exists).
        let existing_binding = true;

        // When the gateway computes the to-thread decision.
        let decision = to_thread_decision(existing_binding);

        // Then it refuses — identical to the `/new`-origin case, since the
        // decision is a function of binding existence, not its origin.
        assert_eq!(decision, ToThreadDecision::AlreadyBound);
        assert_eq!(
            refusal_reason(&decision),
            Some(CreateThreadReason::AlreadyBound)
        );
    }
}
