//! Workflow protocol types - commands and events.

pub mod command;
pub mod event;

pub use command::{
    AttachWorkflow, DetachWorkflow, FireBeforeTurn, ToggleWorkflow, TriggerWorkflow,
};
pub use event::{AttachedWorkflowCompleted, WorkflowAttached, WorkflowDetached, WorkflowToggled};
