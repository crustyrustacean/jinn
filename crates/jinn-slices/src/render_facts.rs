//! Application-facts render context — the read side of overlay views.
//!
//! The canonical view context ([`ViewCx`](crate::view::ViewCx)) passes a
//! slice payload plus the theme. Overlay slices that fold application
//! facts (totals, lifecycle labels) need one more thing: read-only
//! application state. Passing `&AppState` here would drag the kernel's
//! state type into every slice crate, so slices instead read the facts
//! composition supplies as data ([`AppFact`]).
//!
//! This is the data-carrying equivalent of the kernel's `RenderCtx`:
//! same role, no kernel dependency. A slice crate renders from
//! [`RenderFacts`] and never sees `AppState`.

use crate::slices::Slices;
use jinn_theme::Theme;
use std::collections::HashMap;
use std::sync::Arc;

/// One application fact offered to slice views.
///
/// Opaque-on-purpose: values are pre-formatted strings keyed by a
/// dotted fact name (e.g. `session.prune-pending`). Slices declare the
/// names they consume; composition owns the semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppFact {
    /// The dotted fact name.
    pub key: &'static str,
    /// The rendered value.
    pub value: String,
}

/// The read context for overlay views: theme, facts, and the slices
/// registry (for resolving the slice's own cell).
#[derive(Clone, Debug)]
pub struct RenderFacts {
    /// The active theme.
    pub theme: Theme,
    /// Application facts by dotted name.
    pub facts: Arc<HashMap<&'static str, String>>,
    /// The slices registry (shared, read-only at render time).
    pub slices: Slices,
}

impl RenderFacts {
    /// Assembles a facts context over a theme and the live registry.
    #[must_use]
    pub fn new(theme: Theme, slices: &Slices) -> Self {
        Self {
            theme,
            facts: Arc::new(HashMap::new()),
            slices: slices.clone(),
        }
    }

    /// Looks up an application fact by name.
    #[must_use]
    pub fn fact(&self, key: &str) -> Option<&str> {
        self.facts.get(key).map(String::as_str)
    }

    /// Seeds facts (composition builds these per frame; tests seed
    /// directly).
    pub fn set_facts<F: IntoIterator<Item = AppFact>>(&mut self, facts: F) {
        let map = Arc::make_mut(&mut self.facts);
        for fact in facts {
            map.insert(fact.key, fact.value);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test code")]

    use super::AppFact;
    use super::RenderFacts;
    use crate::slices::Slices;
    use jinn_theme::default_theme;

    #[rstest::rstest]
    #[test]
    fn fact_lookup_returns_seeded_value() {
        // Given a facts context with one seeded fact.
        let slices = Slices::new();
        let mut facts = RenderFacts::new(default_theme(), &slices);
        facts.set_facts([AppFact {
            key: "session.prune-pending",
            value: "12 tok".to_owned(),
        }]);

        // When looking the fact up.
        let value = facts.fact("session.prune-pending");

        // Then the seeded value comes back.
        assert_eq!(value, Some("12 tok"));
    }

    #[rstest::rstest]
    #[test]
    fn fact_lookup_misses_unseeded_names() {
        // Given a facts context with no facts.
        let slices = Slices::new();
        let facts = RenderFacts::new(default_theme(), &slices);

        // When looking up any name.
        let value = facts.fact("session.prune-pending");

        // Then nothing comes back.
        assert!(value.is_none());
    }
}
