//! Global workflow registry.
//!
//! Maps workflow names to builder functions that produce [`WorkflowGraph`] instances.
//! Workflows register themselves during app startup via [`register_workflow`].

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::RwLock;

use nullslop_workflow::graph::WorkflowGraph;

/// A function that builds a workflow graph.
pub type WorkflowBuilder = fn(String) -> WorkflowGraph;

struct Registry(HashMap<&'static str, WorkflowBuilder>);

static REGISTRY: OnceLock<RwLock<Registry>> = OnceLock::new();

fn registry() -> &'static RwLock<Registry> {
    REGISTRY.get_or_init(|| RwLock::new(Registry(HashMap::new())))
}

/// Register a workflow builder under the given name.
///
/// Call this during app startup (e.g., in `actor_wiring.rs`).
///
/// # Panics
///
/// Panics if the workflow registry lock is poisoned.
pub fn register_workflow(name: &'static str, builder: WorkflowBuilder) {
    registry()
        .write()
        .expect("workflow registry lock poisoned")
        .0
        .insert(name, builder);
}

/// Look up a workflow builder by name.
///
/// Returns `None` if no workflow with the given name has been registered.
///
/// # Panics
///
/// Panics if the workflow registry lock is poisoned.
#[must_use]
pub fn get_workflow(name: &str) -> Option<WorkflowBuilder> {
    registry()
        .read()
        .expect("workflow registry lock poisoned")
        .0
        .get(name)
        .copied()
}
