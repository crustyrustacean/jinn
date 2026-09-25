//! The `HistoryWorker` trait - pluggable heuristics for history mutation.
//!
//! Each worker inspects a snapshot of the session history and optionally
//! produces a batch of mutations. Workers run outside any lock.
use std::sync::Arc;

use jinn_core_types::ChatEntry;
use jinn_core_types::HistoryMutation;
use jinn_core_types::SessionId;
/// A pluggable history mutation heuristic.
///
/// Each worker inspects a snapshot of the session history and optionally
/// produces a batch of mutations. Workers are run outside any lock,
/// so heuristic evaluation (including LLM calls) never blocks writes.
#[async_trait::async_trait]
pub trait HistoryWorker: Send + Sync + 'static {
    /// Human-readable name for logging and diagnostics.
    fn name(&self) -> &str;

    /// Inspect the history snapshot and optionally produce mutations.
    ///
    /// Called outside any lock. The `history` parameter is a shared snapshot
    /// (via `Arc<[ChatEntry]>`) cloned once by the snapshot actor. The
    /// `session_id` identifies which session triggered the evaluation.
    async fn evaluate(
        &self,
        session_id: &SessionId,
        history: Arc<[ChatEntry]>,
    ) -> Vec<HistoryMutation>;
}

/// Reads a strategy's own auto-prune subsection, live, at the point of use.
///
/// `select` projects the whole auto-prune config down to whatever the
/// strategy needs — its own subsection, or a subsection paired with a
/// value derived from a sibling (the anchored strategy needs the
/// trivial-assistant floor it is defined relative to). It returns `None`
/// to mean "not applicable this pass", which is how a switched-off
/// strategy reports itself.
///
/// A disabled strategy is therefore an inert no-op costing one read, not
/// an absent worker — which is what makes enablement symmetric under
/// `reload`: turning a strategy on *or* off is observed by an
/// already-constructed worker on its very next pass.
pub fn strategy_section<T, F>(layer: &jinn_config::ConfigLayer, select: F) -> Option<T>
where
    F: FnOnce(&jinn_preferences_config::schemas::AutoPruneConfig) -> Option<T>,
{
    let config = layer
        .get::<jinn_preferences_config::schemas::AutoPruneConfig>()
        .ok()?;
    select(&config)
}

/// An auto-prune subsection that can be switched on or off.
pub trait StrategyToggle {
    /// Whether the strategy is enabled.
    fn is_enabled(&self) -> bool;
}

/// Each auto-prune subsection is a toggle plus its tuning; the layer
/// reads `enabled` uniformly, so the trait impls are mechanical.
macro_rules! impl_strategy_toggle {
    ($($ty:ident),* $(,)?) => {$(
        impl StrategyToggle for jinn_preferences_config::schemas::$ty {
            fn is_enabled(&self) -> bool { self.enabled }
        }
    )*};
}

impl_strategy_toggle!(
    ReadEditAutoPruneConfig,
    EditReadAutoPruneConfig,
    BrokenEditAutoPruneConfig,
    DoubleEditAutoPruneConfig,
    ConsecutiveReadsAutoPruneConfig,
    ToolAgeWindowAutoPruneConfig,
    TodoAutoPruneConfig,
    TrivialAssistantAutoPruneConfig,
    AnchoredAssistantAutoPruneConfig,
    RegexAutoPruneConfig,
);

/// A layer seeded with `document`, for strategy tests.
#[cfg(test)]
pub(crate) fn test_layer(document: &str) -> jinn_config::ConfigLayer {
    let parsed = document.parse().expect("test TOML parses");
    jinn_config::ConfigLayer::load(std::sync::Arc::new(
        jinn_config::InMemoryConfigStorage::new(parsed),
    ))
    .expect("layer loads")
}

/// A layer with one auto-prune subsection enabled and the given body
/// appended to it, e.g. `("read_edit", "min_age = 5")`.
#[cfg(test)]
pub(crate) fn layer_with_strategy<T>(umbrella_path: &str, args: T) -> jinn_config::ConfigLayer
where
    T: std::fmt::Display,
{
    test_layer(&format!(
        "[context_curation.auto_prune.{umbrella_path}]\nenabled = true\n{args}"
    ))
}

#[cfg(test)]
mod live_read_tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use super::{layer_with_strategy, test_layer};
    use std::sync::Arc;

    /// A strategy reads its own subsection at the point of use, so a
    /// `reload` after construction is observed by the very next pass.
    #[rstest::rstest]
    #[test]
    fn a_constructed_worker_observes_a_reload_in_both_directions() {
        // Given a worker built while its strategy is switched on.
        let layer = layer_with_strategy("todo", format!("min_age = 0\n"));
        let worker = crate::strategies::TodoAutoPruneWorker {
            layer: layer.clone(),
            config: Default::default(),
        };
        assert!(worker.section().is_some(), "the strategy starts enabled");

        // When the strategy is switched off behind the worker's back and
        // the document re-read.
        let mut doc = layer.document_text();
        doc = doc.replace("enabled = true", "enabled = false");
        layer
            .use_storage(Arc::new(jinn_config::InMemoryConfigStorage::new(
                doc.parse().expect("parses"),
            )))
            .expect("re-reads");

        // Then the already-constructed worker sees it off.
        assert!(
            !worker.section().is_some(),
            "a reload turns the strategy off for a worker built earlier"
        );
    }

    /// The asymmetry this design exists to prevent: gating at wiring time
    /// would make a disabled strategy an *absent* worker, leaving nothing
    /// to switch on later.
    #[rstest::rstest]
    #[test]
    fn a_disabled_strategy_is_present_rather_than_absent() {
        // Given a worker whose strategy is off from the start.
        let layer = test_layer("[context_curation.auto_prune.todo]\nenabled = false\n");
        let worker = crate::strategies::TodoAutoPruneWorker {
            layer: layer.clone(),
            config: Default::default(),
        };

        // When the strategy is switched on.
        layer
            .use_storage(Arc::new(jinn_config::InMemoryConfigStorage::new(
                "[context_curation.auto_prune.todo]\nenabled = true\n"
                    .parse()
                    .expect("parses"),
            )))
            .expect("re-reads");

        // Then the same worker instance is live — it was never removed.
        assert!(worker.section().is_some());
    }
}
