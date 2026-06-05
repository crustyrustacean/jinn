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
pub mod plugin_fire;
pub mod plugin_sync;
pub mod protocol;
pub mod workflow_controller_actor;

pub use attached_workflow::WorkflowId;
pub use domain_node_context::DomainNodeContext;
pub use plugin_fire::{PluginFire, PluginFireError, PluginFireService};
pub use plugin_sync::{PluginSyncCall, PluginSyncCallError, PluginSyncCallService};
pub use workflow_controller_actor::WorkflowControllerActorDeps;
