//! Incremental workflow builder with validation.
//!
//! The builder is used by creation tools (Phase 4) to construct a [`WorkflowDef`]
//! incrementally through small, focused calls. Each call validates its input
//! and rejects invalid data immediately. The final [`build`](WorkflowBuilder::build)
//! runs full cross-validation before producing the definition.

use std::collections::HashMap;
use std::fmt;

use error_stack::Report;
use wherror::Error;

use crate::definition::{ModelHint, StepDef, StepOutputDef, WorkflowDef};
use crate::guard::{GuardExpr, GuardPredicate};

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

/// Incremental workflow builder with validation.
///
/// Created via [`WorkflowBuilder::new`], evolved through method calls, and
/// finalized via [`build`](WorkflowBuilder::build). Each method validates its
/// input immediately (e.g., rejects duplicate step IDs). The builder produces
/// a human-readable [`preview`](WorkflowBuilder::preview) at any point for review.
#[derive(Debug)]
pub struct WorkflowBuilder {
    /// Workflow name.
    name: Option<String>,
    /// Human-readable description.
    description: Option<String>,
    /// Schema version.
    version: u32,
    /// Model hint to model ID mappings.
    model_overrides: HashMap<String, String>,
    /// Global template variables.
    globals: HashMap<String, String>,
    /// Ordered list of step definitions.
    steps: Vec<StepDef>,
}

impl Default for WorkflowBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkflowBuilder {
    /// Creates a new, empty workflow builder.
    pub fn new() -> Self {
        Self {
            name: None,
            description: None,
            version: 1,
            model_overrides: HashMap::new(),
            globals: HashMap::new(),
            steps: Vec::new(),
        }
    }

    /// Sets the workflow name and description.
    ///
    /// Rejects empty names.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowErrorKind::MissingField`] if the name is empty.
    pub fn create(
        &mut self,
        name: String,
        description: String,
    ) -> Result<(), Report<WorkflowError>> {
        if name.trim().is_empty() {
            return Err(
                Report::new(WorkflowError::new(WorkflowErrorKind::MissingField(
                    "name".to_owned(),
                )))
                .attach("workflow name must not be empty"),
            );
        }
        self.name = Some(name);
        self.description = Some(description);
        Ok(())
    }

    /// Adds a global variable.
    pub fn add_global(&mut self, key: String, value: String) {
        self.globals.insert(key, value);
    }

    /// Sets a model override mapping.
    pub fn set_model_override(&mut self, hint: String, model_id: String) {
        self.model_overrides.insert(hint, model_id);
    }

    /// Adds a step to the workflow.
    ///
    /// Rejects duplicate step IDs.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowErrorKind::DuplicateStepId`] if a step with this ID
    /// already exists.
    pub fn add_step(&mut self, step: StepDef) -> Result<(), Report<WorkflowError>> {
        if self.steps.iter().any(|s| s.id == step.id) {
            return Err(
                Report::new(WorkflowError::new(WorkflowErrorKind::DuplicateStepId(
                    step.id.clone(),
                )))
                .attach("a step with this ID already exists"),
            );
        }
        self.steps.push(step);
        Ok(())
    }

    /// Adds a guard predicate to an existing step.
    ///
    /// If the step has no guards, the predicate becomes its sole guard.
    /// If the step already has a single predicate, both are wrapped in `All`.
    /// If the step already has an `All` guard, the predicate is appended.
    /// Otherwise, the existing guard and the new predicate are wrapped in `All`.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowErrorKind::StepNotFound`] if no step with this ID exists.
    pub fn add_guard(
        &mut self,
        step_id: &str,
        guard: GuardPredicate,
    ) -> Result<(), Report<WorkflowError>> {
        let step = self
            .steps
            .iter_mut()
            .find(|s| s.id == step_id)
            .ok_or_else(|| {
                Report::new(WorkflowError::new(WorkflowErrorKind::StepNotFound(
                    step_id.to_owned(),
                )))
            })?;

        step.guards = combine_guards(std::mem::take(&mut step.guards), guard);
        Ok(())
    }

    /// Adds an output descriptor to an existing step.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowErrorKind::StepNotFound`] if no step with this ID exists.
    pub fn add_output(
        &mut self,
        step_id: &str,
        output: StepOutputDef,
    ) -> Result<(), Report<WorkflowError>> {
        let step = self
            .steps
            .iter_mut()
            .find(|s| s.id == step_id)
            .ok_or_else(|| {
                Report::new(WorkflowError::new(WorkflowErrorKind::StepNotFound(
                    step_id.to_owned(),
                )))
            })?;

        step.outputs.push(output);
        Ok(())
    }

    /// Builds a human-readable preview of the current draft.
    ///
    /// Shows the workflow name, description, steps with their flags and guards,
    /// globals, and model overrides.
    pub fn preview(&self) -> String {
        let mut lines = Vec::new();

        // Header.
        let name = self.name.as_deref().unwrap_or("(unnamed)");
        lines.push(format!("Workflow: {name}"));
        let desc = self.description.as_deref().unwrap_or("(no description)");
        lines.push(format!("Description: {desc}"));
        lines.push(String::new());

        // Steps.
        if self.steps.is_empty() {
            lines.push("(no steps)".to_owned());
        } else {
            lines.push("Steps:".to_owned());
            for (i, step) in self.steps.iter().enumerate() {
                let num = i + 1;
                let mut flags = Vec::new();
                let hint_name = model_hint_label(&step.model_hint);
                flags.push(hint_name.to_owned());
                if step.checkpoint {
                    flags.push("checkpoint".to_owned());
                }
                if step.requires_user_input {
                    flags.push("user-input".to_owned());
                }
                let flags_str = flags.join(", ");

                lines.push(format!("  {num}. {} [{flags_str}]", step.id));

                // Guards.
                let guard_str = format_guard_expr(&step.guards);
                if !guard_str.is_empty() {
                    lines.push(format!("     Guards: {guard_str}"));
                }

                // Outputs.
                if !step.outputs.is_empty() {
                    let output_labels: Vec<String> = step
                        .outputs
                        .iter()
                        .map(|o| format!("{} ({})", o.label(), output_kind_label(o)))
                        .collect();
                    lines.push(format!("     Outputs: {}", output_labels.join(", ")));
                }
            }
        }

        lines.push(String::new());

        // Globals.
        if !self.globals.is_empty() {
            let keys: Vec<&str> = self.globals.keys().map(String::as_str).collect();
            lines.push(format!("Globals: {}", keys.join(", ")));
        }

        // Model overrides.
        if !self.model_overrides.is_empty() {
            let mappings: Vec<String> = self
                .model_overrides
                .iter()
                .map(|(k, v)| format!("{k} → {v}"))
                .collect();
            lines.push(format!("Model overrides: {}", mappings.join(", ")));
        }

        lines.join("\n")
    }

    /// Validates and produces the final [`WorkflowDef`].
    ///
    /// Rejects workflows with missing name/description, no steps, or other
    /// validation failures.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Name is missing ([`WorkflowErrorKind::MissingField`])
    /// - Description is missing ([`WorkflowErrorKind::MissingField`])
    /// - No steps have been added ([`WorkflowErrorKind::EmptyWorkflow`])
    pub fn build(self) -> Result<WorkflowDef, Report<WorkflowError>> {
        let name = self.name.ok_or_else(|| {
            Report::new(WorkflowError::new(WorkflowErrorKind::MissingField(
                "name".to_owned(),
            )))
        })?;

        let description = self.description.ok_or_else(|| {
            Report::new(WorkflowError::new(WorkflowErrorKind::MissingField(
                "description".to_owned(),
            )))
        })?;

        if self.steps.is_empty() {
            return Err(Report::new(WorkflowError::new(
                WorkflowErrorKind::EmptyWorkflow,
            )));
        }

        Ok(WorkflowDef {
            version: self.version,
            name,
            description,
            model_overrides: self.model_overrides,
            globals: self.globals,
            steps: self.steps,
        })
    }

    /// Validates the current draft without consuming it.
    ///
    /// Checks the same invariants as [`build`](WorkflowBuilder::build) (name present,
    /// description present, at least one step) but does not produce a [`WorkflowDef`].
    ///
    /// # Errors
    ///
    /// Returns an error if name is missing, description is missing, or no steps exist.
    pub fn validate(&self) -> Result<(), Report<WorkflowError>> {
        if self.name.is_none() {
            return Err(Report::new(WorkflowError::new(
                WorkflowErrorKind::MissingField("name".to_owned()),
            )));
        }
        if self.description.is_none() {
            return Err(Report::new(WorkflowError::new(
                WorkflowErrorKind::MissingField("description".to_owned()),
            )));
        }
        if self.steps.is_empty() {
            return Err(Report::new(WorkflowError::new(
                WorkflowErrorKind::EmptyWorkflow,
            )));
        }
        Ok(())
    }

    /// Discards the draft. This consumes the builder.
    pub fn abort(self) {
        // Intentionally consumed. No further action needed.
    }
}

/// Combines an existing guard expression with a new predicate.
fn combine_guards(existing: GuardExpr, new_pred: GuardPredicate) -> GuardExpr {
    let new = GuardExpr::Predicate(new_pred);
    match existing {
        GuardExpr::None => new,
        GuardExpr::Predicate(old_pred) => GuardExpr::All {
            all: vec![GuardExpr::Predicate(old_pred), new],
        },
        GuardExpr::All { mut all } => {
            all.push(new);
            GuardExpr::All { all }
        }
        other => GuardExpr::All {
            all: vec![other, new],
        },
    }
}

/// Returns a short label for a model hint.
fn model_hint_label(hint: &ModelHint) -> &str {
    match hint {
        ModelHint::Small => "small",
        ModelHint::Medium => "medium",
        ModelHint::Large => "large",
        ModelHint::Exact { .. } => "exact",
    }
}

/// Returns a short label for an output kind.
fn output_kind_label(output: &StepOutputDef) -> &str {
    match output {
        StepOutputDef::File { .. } => "file",
        StepOutputDef::Summary { .. } => "summary",
        StepOutputDef::Artifact { .. } => "artifact",
    }
}

/// Formats a guard expression for display in the preview.
fn format_guard_expr(expr: &GuardExpr) -> String {
    match expr {
        GuardExpr::None => String::new(),
        GuardExpr::Predicate(p) => format_predicate(p),
        GuardExpr::All { all } => {
            let inner: Vec<String> = all.iter().map(format_guard_expr).collect();
            format!("all({})", inner.join(", "))
        }
        GuardExpr::Any { any } => {
            let inner: Vec<String> = any.iter().map(format_guard_expr).collect();
            format!("any({})", inner.join(", "))
        }
        GuardExpr::Not { not } => {
            format!("not({})", format_guard_expr(not))
        }
    }
}

/// Formats a single guard predicate for display.
fn format_predicate(pred: &GuardPredicate) -> String {
    match pred {
        GuardPredicate::FileExists { path } => format!("file_exists({path})"),
        GuardPredicate::DirExists { path } => format!("dir_exists({path})"),
        GuardPredicate::FileHashMatches { path } => format!("file_hash_matches({path})"),
        GuardPredicate::CommandSucceeds { command } => format!("command_succeeds({command})"),
        GuardPredicate::OutputMatches { command, pattern } => {
            format!("output_matches({command}, {pattern})")
        }
        GuardPredicate::ValueSet { variable } => format!("value_set({variable})"),
    }
}
