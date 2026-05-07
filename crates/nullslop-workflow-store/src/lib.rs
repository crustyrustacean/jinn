//! Workflow definition store — global persistence for committed workflows.
//!
//! Provides [`WorkflowStore`] as the async trait for workflow definition
//! persistence, [`FileWorkflowStore`] as the per-file JSON backend, and
//! [`WorkflowStoreService`] as the shared service wrapper.
//!
//! Each workflow is stored as `<name>.json` in `~/.config/nullslop/workflows/`.

mod workflow_store;

pub use workflow_store::{
    FileWorkflowStore, WorkflowStore, WorkflowStoreError, WorkflowStoreService,
};
