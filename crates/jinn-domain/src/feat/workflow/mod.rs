//! Workflow integration module.
//!
//! Bridges Lua workflow execution to the domain layer, providing:
//! - [`DomainNodeContext`] - LLM access for Lua `ctx.llm()` capability
//! - [`WorkflowControllerActor`] - orchestrates attached workflow lifecycle
//! - [`WorkflowId`] - unique identifier for workflow attachments
//! - [`WorkflowConfig`] - identifies a Lua plugin and its data

pub mod attached_workflow;
pub mod domain_node_context;
pub mod picker_entry;
pub mod protocol;
pub mod workflow_controller_actor;
pub mod plugin_fire;
pub mod plugin_sync;

pub use attached_workflow::WorkflowId;
pub use domain_node_context::DomainNodeContext;
pub use workflow_controller_actor::WorkflowControllerActorDeps;
pub use plugin_fire::PluginFire;
pub use plugin_sync::PluginSyncCall;
