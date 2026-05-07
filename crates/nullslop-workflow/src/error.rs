//! Workflow error types.
//!
//! Defines [`WorkflowError`] and [`WorkflowErrorKind`] used across the workflow crate
//! for builder validation, state machine transitions, and guard evaluation failures.

use std::fmt;

use wherror::Error;

/// Error type for workflow operations.
///
/// Carries a [`WorkflowErrorKind`] that categorizes the failure. Used by the builder,
/// state machine, and guard evaluator to report domain-specific errors.
#[derive(Debug, Error)]
#[error(debug)]
pub struct WorkflowError {
    /// The error category.
    kind: WorkflowErrorKind,
}

impl WorkflowError {
    /// Creates a new workflow error with the given kind.
    pub fn new(kind: WorkflowErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the error kind.
    pub fn kind(&self) -> &WorkflowErrorKind {
        &self.kind
    }
}

impl fmt::Display for WorkflowErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateStepId(id) => write!(f, "duplicate step ID: {id}"),
            Self::StepNotFound(id) => write!(f, "step not found: {id}"),
            Self::EmptyWorkflow => write!(f, "workflow has no steps"),
            Self::MissingField(field) => write!(f, "missing required field: {field}"),
            Self::InvalidGuard(reason) => write!(f, "invalid guard: {reason}"),
            Self::ValidationFailed(reason) => write!(f, "validation failed: {reason}"),
        }
    }
}

/// Categories of workflow errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowErrorKind {
    /// A step with this ID already exists.
    DuplicateStepId(String),
    /// A step with this ID was not found.
    StepNotFound(String),
    /// The workflow has no steps.
    EmptyWorkflow,
    /// Required field is missing.
    MissingField(String),
    /// Invalid guard predicate.
    InvalidGuard(String),
    /// Validation failed during build.
    ValidationFailed(String),
}
