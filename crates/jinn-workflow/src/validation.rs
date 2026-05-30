//! Graph validation diagnostics.
//!
//! Provides types for reporting graph validation issues that are not hard errors
//! but may indicate the graph won't behave as intended. Used by
//! [`WorkflowGraph::validate`](crate::graph::WorkflowGraph::validate).

/// Severity of a validation diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationSeverity {
    /// A warning - the graph is valid but may not behave as intended.
    Warning,
}

/// A single validation diagnostic.
#[derive(Debug, Clone)]
pub struct ValidationDiagnostic {
    /// The severity of this diagnostic.
    pub severity: ValidationSeverity,
    /// Human-readable description of the issue.
    pub message: String,
}
