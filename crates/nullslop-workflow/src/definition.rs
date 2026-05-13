//! Workflow definition types.
//!
//! Core data model for workflows. All types derive `Serialize` and `Deserialize`
//! for JSON persistence with chat sessions. Rust types are the source of truth —
//! the JSON format is whatever serde produces.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::guard::GuardExpr;

/// Capability level for model selection within a workflow step.
///
/// Each step declares a hint rather than a specific model. The hint resolves
/// at runtime based on available providers and user configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum ModelHint {
    /// A small, cheap model (e.g., local Ollama).
    #[serde(rename = "small")]
    Small,
    /// A mid-range model.
    #[serde(rename = "medium")]
    Medium,
    /// A powerful, expensive model.
    #[serde(rename = "large")]
    Large,
    /// An explicit model identifier, bypassing hint resolution.
    #[serde(rename = "exact")]
    Exact {
        /// The model ID to use directly.
        id: String,
    },
}

/// The definition of a step's output artifact.
///
/// Outputs describe what a step produces. They serve three purposes:
/// UI display, context assembly for downstream steps, and guard verification
/// via content hashing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind")]
pub enum StepOutputDef {
    /// A file on disk. Content-hashed for invalidation tracking.
    #[serde(rename = "file")]
    File {
        /// Human-readable label for the UI.
        label: String,
        /// File path (may contain `{{var}}` template variables).
        path: String,
    },
    /// A text summary captured at step completion.
    #[serde(rename = "summary")]
    Summary {
        /// Human-readable label for the UI.
        label: String,
        /// The summary value (may contain `{{var}}` template variables).
        value: String,
    },
    /// A generic artifact for future extensibility.
    #[serde(rename = "artifact")]
    Artifact {
        /// Human-readable label for the UI.
        label: String,
        /// Description of the artifact.
        description: String,
    },
}

impl StepOutputDef {
    /// Returns the label for this output.
    pub fn label(&self) -> &str {
        match self {
            Self::File { label, .. }
            | Self::Summary { label, .. }
            | Self::Artifact { label, .. } => label,
        }
    }
}

/// The definition of a single step within a workflow.
///
/// Each step is an isolated LLM interaction with its own model hint, tools,
/// guards, and outputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepDef {
    /// Unique identifier for this step.
    pub id: String,
    /// Human-readable title for the UI.
    pub title: String,
    /// Instructions for the LLM (becomes part of the system prompt).
    pub instructions: String,
    /// Model capability level for this step.
    pub model_hint: ModelHint,
    /// Whether the user must approve before proceeding.
    #[serde(default)]
    pub checkpoint: bool,
    /// Whether this step requires user input before execution.
    #[serde(default)]
    pub requires_user_input: bool,
    /// Tools the LLM may use during this step.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Guards that verify step completion.
    #[serde(default)]
    pub guards: GuardExpr,
    /// Output descriptors for this step.
    #[serde(default)]
    pub outputs: Vec<StepOutputDef>,
    /// Step IDs this step depends on. Not enforced in Phase 1.
    #[serde(default)]
    pub depends_on: Vec<String>,
}

/// The definition of a complete workflow.
///
/// A workflow is a state machine with ordered steps. Each step is dispatched
/// to an LLM model determined by its `model_hint`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDef {
    /// Workflow schema version.
    pub version: u32,
    /// Unique name for this workflow.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Model hint to model ID mappings.
    #[serde(default)]
    pub model_overrides: HashMap<String, String>,
    /// Global template variables.
    #[serde(default)]
    pub globals: HashMap<String, String>,
    /// Ordered list of step definitions.
    pub steps: Vec<StepDef>,
}

#[cfg(test)]
mod tests {
    use super::*;








    /// Parses a minimal `StepDef` JSON (no optional fields) for default-checking tests.
    fn parse_minimal_step_def() -> StepDef {
        let json = r#"{
            "id": "step-1",
            "title": "Test",
            "instructions": "Do it",
            "model_hint": {"type": "small"}
        }"#;
        serde_json::from_str(json).unwrap()
    }

    #[rstest::rstest]
    #[case::checkpoint("checkpoint", true)]
    #[case::requires_user_input("requires_user_input", true)]
    #[case::tools_empty("tools_empty", true)]
    #[case::outputs_empty("outputs_empty", true)]
    #[case::depends_on_empty("depends_on_empty", true)]
    #[case::guards_none("guards_none", true)]
    fn step_def_missing_optional_uses_default(#[case] field: &str, #[case] expected: bool) {
        // Given a StepDef deserialized from JSON without optional fields.
        let step = parse_minimal_step_def();

        // Then the field uses its default value.
        let actual = match field {
            "checkpoint" => !step.checkpoint,
            "requires_user_input" => !step.requires_user_input,
            "tools_empty" => step.tools.is_empty(),
            "outputs_empty" => step.outputs.is_empty(),
            "depends_on_empty" => step.depends_on.is_empty(),
            "guards_none" => step.guards == GuardExpr::None,
            _ => panic!("unknown field: {field}"),
        };
        assert_eq!(
            actual, expected,
            "field {field}: expected {expected}, got {actual}"
        );
    }
}
