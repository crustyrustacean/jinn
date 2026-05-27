//! Workflow registry — maps workflow names to builder functions.
//!
//! Instance-based registry following the [`BuiltinRegistry`](crate::feat::session_lifecycle::builtin::BuiltinRegistry) pattern.
//! Created during startup, populated via [`register_all_workflows`], and injected
//! into [`WorkflowActorDeps`](crate::feat::workflow::workflow_actor::WorkflowActorDeps).

use std::collections::HashMap;

use nullslop_workflow::graph::WorkflowGraph;

/// A function that builds a workflow graph.
pub type WorkflowBuilder = fn() -> WorkflowGraph;

/// Instance-based workflow registry.
///
/// Maps workflow names to builder functions. Created once during startup,
/// populated via [`register()`](Self::register), and injected into the
/// workflow actor via deps. No globals.
#[derive(Debug, Default)]
pub struct WorkflowRegistry {
    builders: HashMap<String, WorkflowBuilder>,
}

impl WorkflowRegistry {
    /// Creates a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a workflow builder under the given name.
    pub fn register(&mut self, name: impl Into<String>, builder: WorkflowBuilder) {
        self.builders.insert(name.into(), builder);
    }

    /// Look up a workflow builder by name.
    ///
    /// Returns `None` if no workflow with the given name has been registered.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<WorkflowBuilder> {
        self.builders.get(name).copied()
    }

    /// Returns all registered workflow names in sorted order.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.builders.keys().cloned().collect();
        names.sort();
        names
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]

    use super::*;
    use crate::feat::workflow::example::add_numbers;

    fn trivial_graph() -> WorkflowGraph {
        add_numbers::build_add_numbers()
    }

    #[rstest::rstest]
    fn new_registry_is_empty() {
        // Given a new registry.
        let registry = WorkflowRegistry::new();

        // Then it has no entries.
        assert!(registry.names().is_empty());
        assert!(registry.get("anything").is_none());
    }

    #[rstest::rstest]
    fn register_adds_builder() {
        // Given an empty registry.
        let mut registry = WorkflowRegistry::new();

        // When registering a workflow.
        registry.register("test-workflow", trivial_graph);

        // Then get returns the builder.
        assert!(registry.get("test-workflow").is_some());
    }

    #[rstest::rstest]
    fn register_overwrites_previous() {
        // Given a registry with one entry.
        let mut registry = WorkflowRegistry::new();
        registry.register("dup", trivial_graph);

        // When registering again under the same name.
        registry.register("dup", trivial_graph);

        // Then get still returns a builder.
        assert!(registry.get("dup").is_some());
    }

    #[rstest::rstest]
    fn get_returns_none_for_unknown() {
        // Given a registry with one entry.
        let mut registry = WorkflowRegistry::new();
        registry.register("known", trivial_graph);

        // Then unknown names return None.
        assert!(registry.get("unknown").is_none());
        assert!(registry.get("").is_none());
    }

    #[rstest::rstest]
    fn names_returns_sorted_unique_names() {
        // Given a registry with multiple entries.
        let mut registry = WorkflowRegistry::new();
        registry.register("charlie", trivial_graph);
        registry.register("alpha", trivial_graph);
        registry.register("bravo", trivial_graph);

        // When reading names.
        let names = registry.names();

        // Then they are sorted.
        assert_eq!(names, vec!["alpha", "bravo", "charlie"]);
    }

    #[rstest::rstest]
    fn names_returns_real_strings() {
        // Given a registry with a real entry.
        let mut registry = WorkflowRegistry::new();
        registry.register("real-workflow", trivial_graph);

        // When reading names.
        let names = registry.names();

        // Then each name is the actual registered name.
        assert_eq!(names.len(), 1);
        assert_eq!(names[0], "real-workflow");
        assert!(!names[0].is_empty());
        assert_ne!(names[0], "xyzzy");
    }
}
