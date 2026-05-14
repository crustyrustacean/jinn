//! Workflow engine — state machines, guard predicates, and incremental builders.
//!
//! This crate defines the core data model for the nullslop workflow system.
//! Workflows are state machines with ordered steps, each dispatched to LLM
//! models based on capability hints. Steps have composable guard predicates
//! for completion verification and content-hash-based invalidation for
//! efficient re-execution on jump-back.
//!
//! This crate has **no dependency on the bus, component system, or TUI** — it
//! is a pure domain library.
//!
//! # Organization
//!
//! - [`definition`] — `WorkflowDef`, `StepDef`, `ModelHint`, `StepOutputDef`
//! - [`guard`] — `GuardPredicate`, `GuardExpr`, `GuardResult`, evaluation engine
//! - [`template`] — `{{var}}` template variable resolution
//! - [`state`] — `StepStatus`, `WorkflowState`, step transitions
//! - [`builder`] — `WorkflowBuilder` with incremental validation
//! - [`hash`] — SHA-256 content hashing for file outputs

pub mod builder;
#[cfg(test)]
mod builder_tests;
pub mod definition;
pub mod guard;
pub mod hash;
pub mod state;

#[cfg(test)]
mod guard_tests;

#[cfg(test)]
mod state_tests;
pub mod template;

pub use builder::WorkflowBuilder;
pub use builder::{WorkflowError, WorkflowErrorKind};
pub use definition::{ModelHint, StepDef, StepOutputDef, WorkflowDef};
pub use guard::{
    DefaultGuardEvaluator, GuardEvaluator, GuardExpr, GuardFailure, GuardFileSystem,
    GuardPredicate, GuardResult, GuardShell, RealFileSystem, RealShell,
};
pub use hash::file_content_hash;
pub use state::{StepState, StepStatus, WorkflowState};
pub use template::{build_variable_map, resolve_template};
