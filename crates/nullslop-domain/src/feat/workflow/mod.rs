//! Workflow integration module.
//!
//! Bridges the [`nullslop_workflow`] engine to the domain layer, providing:
//! - [`DomainNodeContext`] — implements [`NodeContext`] with LLM access
//! - [`LlmNode`] — a workflow node that calls the LLM
//! - [`WorkflowActor`] — bridges actor bus events to workflow execution
//! - [`WorkflowMap`] / [`WorkflowState`] — runtime workflow state in [`AppState`]
//! - [`WorkflowRegistry`] — global named workflow registry

pub mod domain_node_context;
pub mod example_workflows;
pub mod llm_node;
pub mod protocol;
pub mod workflow_actor;
pub mod workflow_registry;
pub mod workflow_state;

pub use domain_node_context::DomainNodeContext;
pub use llm_node::LlmNode;
pub use workflow_actor::WorkflowActor;
pub use workflow_registry::{get_workflow, register_workflow};
pub use workflow_state::{WorkflowId, WorkflowMap, WorkflowState};
