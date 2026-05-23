//! Workflow integration module.
//!
//! Bridges the [`nullslop_workflow`] engine to the domain layer, providing:
//! - [`DomainNodeContext`] — implements [`NodeContext`] with LLM access
//! - [`LlmNode`] — a workflow node that calls the LLM
//! - [`WorkflowActor`] — bridges actor bus events to workflow execution
//! - [`WorkflowMap`] / [`WorkflowState`] — runtime workflow state in [`AppState`]
//! - [`WorkflowRegistry`] — instance-based named workflow registry

pub mod domain_node_context;
pub mod example;
pub mod node;
pub mod protocol;
pub mod workflow_actor;
pub mod workflow_registry;
pub mod workflow_state;

pub use domain_node_context::DomainNodeContext;
pub use node::LlmNode;
pub use workflow_actor::WorkflowActor;
pub use workflow_registry::WorkflowRegistry;
pub use workflow_state::{WorkflowId, WorkflowMap, WorkflowState};

/// Registers all built-in workflows into the given registry.
///
/// Call once during startup (e.g., when assembling actor deps).
/// Mirrors the [`register_all_ui_elements`](crate::common::register_all_ui_elements) pattern.
pub fn register_all_workflows(registry: &mut WorkflowRegistry) {
    example::register(registry);
}
