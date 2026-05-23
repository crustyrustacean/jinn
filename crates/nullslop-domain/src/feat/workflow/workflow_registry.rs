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
    builders: HashMap<&'static str, WorkflowBuilder>,
}

impl WorkflowRegistry {
    /// Creates a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a workflow builder under the given name.
    pub fn register(&mut self, name: &'static str, builder: WorkflowBuilder) {
        self.builders.insert(name, builder);
    }

    /// Look up a workflow builder by name.
    ///
    /// Returns `None` if no workflow with the given name has been registered.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<WorkflowBuilder> {
        self.builders.get(name).copied()
    }
}
