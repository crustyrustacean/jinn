//! Workflow domain: commands and events for managing structured multi-step workflows.
//!
//! Workflows are state machines with ordered steps, each dispatched to LLM models
//! based on capability hints. This module defines the wire types for loading,
//! advancing, jumping within, and aborting workflows.

mod command;
mod event;

pub use command::{AbortWorkflow, AdvanceStep, CompleteStep, JumpToStep, LoadWorkflow};
pub use event::{
    StepAwaitingInput, StepCompleted, StepStale, StepStarted, WorkflowCompleted, WorkflowLoaded,
};
