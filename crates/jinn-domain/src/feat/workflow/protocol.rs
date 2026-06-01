//! Workflow protocol types - commands and events.

pub mod command;
pub mod event;

pub use command::{
    AttachWorkflow, CancelWorkflow, DetachWorkflow, InitWorkflow,
    LoadWorkflowPickerEntries, RerunFromNode, StartWorkflow, ToggleWorkflow, TriggerWorkflow,
};
pub use event::{
    AttachedWorkflowCompleted, WorkflowAttached, WorkflowCompleted, WorkflowDetached,
    WorkflowInitialized, WorkflowNodeStatusChanged, WorkflowStarted, WorkflowToggled,
};
