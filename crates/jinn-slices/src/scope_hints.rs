//! Per-scope render hints — what a slice wants the chat chrome to do
//! while its scope is focused.
//!
//! The chat chrome (the border between main and sidebar, the bottom
//! line, the audit popup) has to answer three questions about whatever
//! slice owns the focused scope: which accent to draw with, whether
//! that accent is a distinct "acting" one, and whether a lower
//! overlay should be suppressed.
//!
//! Those answers belong to the slice, not to the chrome. A chrome
//! that hard-codes them has to learn a slice's scope *name* to
//! special-case it, and every new scope that wants a different
//! accent is another string comparison in the composition layer. A
//! scope registers a [`ScopeRenderHint`] at activation and the chrome
//! reads the hint.
//!
//! The precedent is `FocusScope::is_sidebar` and
//! `SidebarScopeExt::sidebar_section`: a slice names its own scopes,
//! and the mechanism is data.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::slice_scope::SliceScopeId;

/// Which accent the chat chrome draws with while `scope` is focused.
///
/// The chat chrome has exactly two visual states per accent — a
/// "focused" one and an "unfocused" one — plus one extra state a
/// slice may claim for itself, so a slice asks for one of three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Accent {
    /// The slice is the focus owner but claims no accent of its own:
    /// the chrome draws its focused accent.
    Focused,
    /// The slice is *acting* rather than merely focused (the sidebar's
    /// resize handle, which is a drag in progress): the chrome draws
    /// a distinct accent that reads as transient.
    Acting,
    /// The slice is not the focus owner: the chrome draws its
    /// unfocused accent.
    Unfocused,
}

/// What the chat chrome should do while a scope is focused.
///
/// Registered per scope at activation. The default — no registration
/// for the focused scope — is [`Accent::Unfocused`] and
/// `suppresses_lower_overlay: false`, which is what every scope that
/// never registers behaves as today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeRenderHint {
    /// The accent the chrome draws while this scope is focused.
    pub accent: Accent,
    /// Whether a lower overlay that would otherwise paint (the chat
    /// log's audit popup) stands down while this scope is focused.
    pub suppresses_lower_overlay: bool,
}

impl Default for ScopeRenderHint {
    fn default() -> Self {
        Self {
            accent: Accent::Unfocused,
            suppresses_lower_overlay: false,
        }
    }
}

impl ScopeRenderHint {
    /// A hint for a scope that is the focus owner and claims the
    /// chrome's focused accent.
    #[must_use]
    pub fn focused() -> Self {
        Self {
            accent: Accent::Focused,
            suppresses_lower_overlay: false,
        }
    }

    /// A hint for a scope that is mid-action and claims its own
    /// transient accent.
    #[must_use]
    pub fn acting() -> Self {
        Self {
            accent: Accent::Acting,
            suppresses_lower_overlay: false,
        }
    }

    /// Marks this hint as also standing a lower overlay down while its
    /// scope is focused.
    #[must_use]
    pub fn suppressing_lower_overlay(mut self) -> Self {
        self.suppresses_lower_overlay = true;
        self
    }
}

/// The registry of per-scope render hints, keyed by scope.
#[derive(Clone, Debug, Default)]
pub struct ScopeHints {
    hints: Arc<RwLock<HashMap<SliceScopeId, ScopeRenderHint>>>,
}

impl ScopeHints {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the hint for `scope`, replacing any previous one.
    pub fn register(&self, scope: SliceScopeId, hint: ScopeRenderHint) {
        self.hints.write().insert(scope, hint);
    }

    /// Returns the hint registered for `scope`.
    ///
    /// An unregistered scope yields the default hint, so a scope that
    /// never registers behaves as an ordinary unfocused scope rather
    /// than as a wiring failure.
    #[must_use]
    pub fn hint(&self, scope: &SliceScopeId) -> ScopeRenderHint {
        self.hints.read().get(scope).copied().unwrap_or_default()
    }

    /// The scopes that have a hint, sorted for stable display.
    #[must_use]
    pub fn registered(&self) -> Vec<SliceScopeId> {
        let mut scopes: Vec<SliceScopeId> = self.hints.read().keys().cloned().collect();
        scopes.sort();
        scopes
    }
}

#[cfg(test)]
mod tests {
    use super::Accent;
    use super::ScopeHints;
    use super::ScopeRenderHint;
    use crate::SliceScopeId;

    fn scope(slice: &str, name: &str) -> SliceScopeId {
        SliceScopeId::new(slice, name)
    }

    #[rstest::rstest]
    #[test]
    fn unregistered_scope_reads_as_the_default_hint() {
        // Given a registry with no registration for a scope.
        let hints = ScopeHints::new();
        let target = scope("test", "unregistered");

        // When reading the hint.
        let hint = hints.hint(&target);

        // Then it is the default: unfocused, no suppression.
        assert_eq!(hint, ScopeRenderHint::default());
        assert_eq!(hint.accent, Accent::Unfocused);
        assert!(!hint.suppresses_lower_overlay);
    }

    #[rstest::rstest]
    #[test]
    fn register_then_resolve_roundtrips_scope() {
        // Given a registry with a hint registered for a scope.
        let hints = ScopeHints::new();
        let target = scope("test", "acting");
        hints.register(target.clone(), ScopeRenderHint::acting());

        // When reading the hint back.
        let hint = hints.hint(&target);

        // Then the accent round-trips.
        assert_eq!(hint.accent, Accent::Acting);
        // And a sibling scope is unaffected.
        assert_eq!(
            hints.hint(&scope("test", "other")).accent,
            Accent::Unfocused
        );
    }

    #[rstest::rstest]
    #[test]
    fn suppressing_hint_carries_both_fields() {
        // Given a focused hint that also stands a lower overlay down.
        let hint = ScopeRenderHint::focused().suppressing_lower_overlay();

        // When reading both fields.
        // Then the accent is focused and suppression is on.
        assert_eq!(hint.accent, Accent::Focused);
        assert!(hint.suppresses_lower_overlay);
    }

    #[rstest::rstest]
    #[test]
    fn reregister_replaces_the_previous_hint() {
        // Given a registry with one hint for a scope.
        let hints = ScopeHints::new();
        let target = scope("test", "moving");
        hints.register(target.clone(), ScopeRenderHint::focused());

        // When re-registering a different hint for the same scope.
        hints.register(target.clone(), ScopeRenderHint::acting());

        // Then the later hint wins.
        assert_eq!(hints.hint(&target).accent, Accent::Acting);
        // And the scope still appears once in the enumeration.
        assert_eq!(hints.registered(), vec![target]);
    }
}
