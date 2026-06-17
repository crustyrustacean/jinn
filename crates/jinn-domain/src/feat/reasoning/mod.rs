//! Reasoning effort — how hard a reasoning-capable model thinks before answering.
//!
//! This module re-exports the [`ReasoningEffort`] / [`ReasoningConfig`] types
//! (defined in `jinn-provider`, the lowest crate in the dependency chain) and
//! provides [`resolve_effort`], the pure policy that picks the effective effort
//! from a session override and the global default.
//!
//! The types live in `jinn-provider` so the OpenAI-compatible request builder
//! can emit them without `jinn-domain` reaching down into provider internals.

pub use jinn_provider::{ReasoningConfig, ReasoningEffort};

/// Resolves the effective reasoning effort for a request.
///
/// A session override always wins. When there is no override, the global
/// default applies. When both are unset, `None` is returned — meaning "send no
/// effort field and let the provider decide" (OpenRouter still requests
/// reasoning tokens via `{ "enabled": true }`).
#[must_use]
pub fn resolve_effort(
    session_override: Option<ReasoningEffort>,
    global_default: Option<ReasoningEffort>,
) -> Option<ReasoningEffort> {
    session_override.or(global_default)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::*;

    #[test]
    fn session_override_wins_over_global_default() {
        // Given a session override of Low and a global default of High.
        // When resolving.
        // Then the session override (Low) is used.
        assert_eq!(
            resolve_effort(Some(ReasoningEffort::Low), Some(ReasoningEffort::High)),
            Some(ReasoningEffort::Low)
        );
    }

    #[test]
    fn global_default_used_when_session_override_absent() {
        // Given no session override and a global default of High.
        // When resolving.
        // Then the global default (High) is used.
        assert_eq!(
            resolve_effort(None, Some(ReasoningEffort::High)),
            Some(ReasoningEffort::High)
        );
    }

    #[test]
    fn none_when_both_session_and_global_unset() {
        // Given no session override and no global default.
        // When resolving.
        // Then None is returned (send no effort field).
        assert_eq!(resolve_effort(None, None), None);
    }
}
