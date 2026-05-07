//! Incremental workflow builder with validation.
//!
//! The builder is used by creation tools (Phase 4) to construct a [`WorkflowDef`]
//! incrementally through small, focused calls. Each call validates its input
//! and rejects invalid data immediately. The final [`build`](WorkflowBuilder::build)
//! runs full cross-validation before producing the definition.

use std::collections::HashMap;

use error_stack::Report;

use crate::definition::{ModelHint, StepDef, StepOutputDef, WorkflowDef};
use crate::error::{WorkflowError, WorkflowErrorKind};
use crate::guard::{GuardExpr, GuardPredicate};

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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_step(id: &str) -> StepDef {
        StepDef {
            id: id.to_owned(),
            title: format!("Step {id}"),
            instructions: format!("Instructions for {id}"),
            model_hint: ModelHint::Small,
            checkpoint: false,
            requires_user_input: false,
            tools: vec![],
            guards: GuardExpr::None,
            outputs: vec![],
            depends_on: vec![],
        }
    }

    #[test]
    fn create_with_valid_name_succeeds() {
        // Given a new builder.
        let mut builder = WorkflowBuilder::new();

        // When creating with valid name and description.
        let result = builder.create("my-workflow".to_owned(), "A test".to_owned());

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[test]
    fn create_with_empty_name_fails() {
        let mut builder = WorkflowBuilder::new();
        let result = builder.create(String::new(), "desc".to_owned());
        assert!(result.is_err());
    }

    #[test]
    fn add_step_with_unique_id_succeeds() {
        let mut builder = WorkflowBuilder::new();
        builder
            .create("test".to_owned(), "desc".to_owned())
            .unwrap();

        let result = builder.add_step(make_step("step-1"));
        assert!(result.is_ok());
    }

    #[test]
    fn add_step_with_duplicate_id_fails() {
        let mut builder = WorkflowBuilder::new();
        builder
            .create("test".to_owned(), "desc".to_owned())
            .unwrap();
        builder.add_step(make_step("step-1")).unwrap();

        let result = builder.add_step(make_step("step-1"));
        assert!(result.is_err());
    }

    #[test]
    fn add_guard_for_existing_step_succeeds() {
        let mut builder = WorkflowBuilder::new();
        builder
            .create("test".to_owned(), "desc".to_owned())
            .unwrap();
        builder.add_step(make_step("step-1")).unwrap();

        let result = builder.add_guard(
            "step-1",
            GuardPredicate::FileExists {
                path: "/tmp/test".to_owned(),
            },
        );
        assert!(result.is_ok());
    }

    #[test]
    fn add_guard_for_unknown_step_fails() {
        let mut builder = WorkflowBuilder::new();
        builder
            .create("test".to_owned(), "desc".to_owned())
            .unwrap();

        let result = builder.add_guard(
            "nope",
            GuardPredicate::FileExists {
                path: "/tmp/test".to_owned(),
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn add_output_for_existing_step_succeeds() {
        let mut builder = WorkflowBuilder::new();
        builder
            .create("test".to_owned(), "desc".to_owned())
            .unwrap();
        builder.add_step(make_step("step-1")).unwrap();

        let result = builder.add_output(
            "step-1",
            StepOutputDef::File {
                label: "Output".to_owned(),
                path: "/tmp/out".to_owned(),
            },
        );
        assert!(result.is_ok());
    }

    #[test]
    fn add_output_for_unknown_step_fails() {
        let mut builder = WorkflowBuilder::new();
        builder
            .create("test".to_owned(), "desc".to_owned())
            .unwrap();

        let result = builder.add_output(
            "nope",
            StepOutputDef::File {
                label: "Output".to_owned(),
                path: "/tmp/out".to_owned(),
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn build_with_empty_steps_fails() {
        let mut builder = WorkflowBuilder::new();
        builder
            .create("test".to_owned(), "desc".to_owned())
            .unwrap();

        let result = builder.build();
        assert!(result.is_err());
    }

    #[test]
    fn build_with_valid_data_produces_workflow_def() {
        let mut builder = WorkflowBuilder::new();
        builder
            .create("my-workflow".to_owned(), "A test workflow".to_owned())
            .unwrap();
        builder.add_global("base_dir".to_owned(), "/tmp".to_owned());
        builder.add_step(make_step("step-1")).unwrap();

        let def = builder.build().unwrap();

        assert_eq!(def.name, "my-workflow");
        assert_eq!(def.description, "A test workflow");
        assert_eq!(def.globals.get("base_dir"), Some(&"/tmp".to_owned()));
        assert_eq!(def.steps.len(), 1);
        assert_eq!(def.steps.first().unwrap().id, "step-1");
    }

    #[test]
    fn build_without_name_fails() {
        let builder = WorkflowBuilder::new();
        let result = builder.build();
        assert!(result.is_err());
    }

    #[test]
    fn preview_produces_expected_format() {
        let mut builder = WorkflowBuilder::new();
        builder
            .create("test-workflow".to_owned(), "A test".to_owned())
            .unwrap();
        builder.add_global("dir".to_owned(), "/tmp".to_owned());
        builder.set_model_override("small".to_owned(), "ollama/phi3".to_owned());

        let mut step = make_step("create-dir");
        step.checkpoint = true;
        step.requires_user_input = true;
        builder.add_step(step).unwrap();

        builder
            .add_guard(
                "create-dir",
                GuardPredicate::FileExists {
                    path: "{{dir}}/notes.md".to_owned(),
                },
            )
            .unwrap();

        builder
            .add_output(
                "create-dir",
                StepOutputDef::File {
                    label: "Notes".to_owned(),
                    path: "{{dir}}/notes.md".to_owned(),
                },
            )
            .unwrap();

        let preview = builder.preview();

        assert!(preview.contains("Workflow: test-workflow"));
        assert!(preview.contains("create-dir"));
        assert!(preview.contains("checkpoint"));
        assert!(preview.contains("user-input"));
        assert!(preview.contains("file_exists({{dir}}/notes.md)"));
        assert!(preview.contains("Notes (file)"));
        assert!(preview.contains("Globals: dir"));
        assert!(preview.contains("Model overrides: small → ollama/phi3"));
    }

    #[test]
    fn full_builder_flow() {
        // Given a builder.
        let mut builder = WorkflowBuilder::new();

        // When building a complete workflow.
        builder
            .create(
                "video-workflow".to_owned(),
                "Music video workflow".to_owned(),
            )
            .unwrap();

        builder.add_global("video_dir".to_owned(), "/tmp/video".to_owned());
        builder.set_model_override("small".to_owned(), "ollama/phi3".to_owned());

        builder.add_step(make_step("setup")).unwrap();
        builder.add_step(make_step("render")).unwrap();

        builder
            .add_guard(
                "setup",
                GuardPredicate::FileExists {
                    path: "{{video_dir}}/config.json".to_owned(),
                },
            )
            .unwrap();

        builder
            .add_output(
                "setup",
                StepOutputDef::Summary {
                    label: "Config".to_owned(),
                    value: "done".to_owned(),
                },
            )
            .unwrap();

        // Then preview shows all the pieces.
        let preview = builder.preview();
        assert!(preview.contains("video-workflow"));
        assert!(preview.contains("setup"));
        assert!(preview.contains("render"));
        assert!(preview.contains("file_exists({{video_dir}}/config.json)"));
        assert!(preview.contains("Config (summary)"));

        // And build produces a valid WorkflowDef.
        let def = builder.build().unwrap();
        assert_eq!(def.name, "video-workflow");
        assert_eq!(def.steps.len(), 2);
        assert_eq!(
            def.steps.first().unwrap().guards,
            GuardExpr::Predicate(GuardPredicate::FileExists {
                path: "{{video_dir}}/config.json".to_owned(),
            })
        );
    }

    #[test]
    fn adding_multiple_guards_combines_with_all() {
        let mut builder = WorkflowBuilder::new();
        builder
            .create("test".to_owned(), "desc".to_owned())
            .unwrap();
        builder.add_step(make_step("step-1")).unwrap();

        builder
            .add_guard(
                "step-1",
                GuardPredicate::FileExists {
                    path: "/a".to_owned(),
                },
            )
            .unwrap();

        builder
            .add_guard(
                "step-1",
                GuardPredicate::FileExists {
                    path: "/b".to_owned(),
                },
            )
            .unwrap();

        let def = builder.build().unwrap();
        let step = def.steps.first().unwrap();

        // Should be wrapped in All.
        match &step.guards {
            GuardExpr::All { all } => {
                assert_eq!(all.len(), 2);
            }
            other => panic!("expected All, got {other:?}"),
        }
    }

    #[test]
    fn abort_consumes_builder() {
        let builder = WorkflowBuilder::new();
        builder.abort();
        // Builder is consumed; this test just verifies it compiles.
    }

    // --- validate() tests ---

    #[test]
    fn validate_succeeds_for_valid_draft() {
        // Given a builder with name, description, and a step.
        let mut builder = WorkflowBuilder::new();
        builder
            .create("my-workflow".to_owned(), "A test".to_owned())
            .unwrap();
        builder.add_step(make_step("step-1")).unwrap();

        // When validating.
        let result = builder.validate();

        // Then it succeeds.
        assert!(result.is_ok());
    }

    #[test]
    fn validate_fails_without_name() {
        // Given a builder with no name.
        let builder = WorkflowBuilder::new();

        // When validating.
        let result = builder.validate();

        // Then it fails with MissingField("name").
        let err = result.expect_err("should fail");
        let kind = err.current_context().kind();
        assert_eq!(*kind, WorkflowErrorKind::MissingField("name".to_owned()));
    }

    #[test]
    fn validate_fails_without_description() {
        // Given a builder with no description.
        let builder = WorkflowBuilder::new();

        // When validating.
        let result = builder.validate();

        // Then it fails. Name is checked first, so we get MissingField("name").
        let err = result.expect_err("should fail");
        assert!(matches!(
            err.current_context().kind(),
            WorkflowErrorKind::MissingField(_)
        ));
    }

    #[test]
    fn validate_fails_with_no_steps() {
        // Given a builder with name and description but no steps.
        let mut builder = WorkflowBuilder::new();
        builder
            .create("my-workflow".to_owned(), "A test".to_owned())
            .unwrap();

        // When validating.
        let result = builder.validate();

        // Then it fails with EmptyWorkflow.
        let err = result.expect_err("should fail");
        assert_eq!(
            *err.current_context().kind(),
            WorkflowErrorKind::EmptyWorkflow
        );
    }

    #[test]
    fn validate_does_not_consume_builder() {
        // Given a builder with name, description, and a step.
        let mut builder = WorkflowBuilder::new();
        builder
            .create("test".to_owned(), "desc".to_owned())
            .unwrap();
        builder.add_step(make_step("s1")).unwrap();

        // When validating.
        builder.validate().unwrap();

        // Then the builder is still usable.
        builder.add_step(make_step("s2")).unwrap();
        let def = builder.build().unwrap();
        assert_eq!(def.steps.len(), 2);
    }
}
