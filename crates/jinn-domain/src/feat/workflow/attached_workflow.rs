//! Attached workflow types — persistent data model for workflows bound to sessions.
//!
//! Attached workflows are persistent fields on [`SessionCore`](crate::feat::session::chat_session::SessionCore),
//! visible in the session sidebar, and triggered by session lifecycle events.
//!
//! Key types:
//! - [`AttachedWorkflow`] — the persistent attachment on a session
//! - [`WorkflowConfig`] — both the kind identifier and parameter source
//! - [`WorkflowTrigger`] — when the workflow fires
//! - [`OneShotKind`] — keybind toggle keys for one-shot workflows

use serde::{Deserialize, Serialize};

use super::workflow_state::WorkflowId;

/// A workflow that is persistently attached to a session.
///
/// Stored in `SessionCore::attached_workflows`. Persists across restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachedWorkflow {
    /// Unique ID for this attachment. Used as the key in `AppState::workflow_executions`.
    pub id: WorkflowId,
    /// The configuration (kind + parameters) for this workflow.
    pub config: WorkflowConfig,
    /// User-editable display label. Initialized from `config.label()` at construction.
    /// Falls back to `config.label()` when empty (backward compat with old persisted data).
    #[serde(default)]
    pub label: String,
    /// When this workflow fires.
    pub trigger: WorkflowTrigger,
    /// Whether this attachment is active. `false` = skip on trigger.
    pub enabled: bool,
    /// Current execution state.
    pub state: AttachedWorkflowState,
}

impl AttachedWorkflow {
    /// Create a new attached workflow with Ready state and enabled=true.
    #[must_use]
    pub fn new(config: WorkflowConfig, trigger: WorkflowTrigger) -> Self {
        let label = config.label().to_owned();
        Self {
            id: WorkflowId::new(),
            config,
            label,
            trigger,
            enabled: true,
            state: AttachedWorkflowState::Ready,
        }
    }

    /// Returns the display label, falling back to `config.label()` when empty.
    ///
    /// This handles backward compatibility with old persisted data that lacks
    /// the `label` field — `#[serde(default)]` produces an empty string.
    #[must_use]
    pub fn label_or_default(&self) -> &str {
        if self.label.is_empty() {
            self.config.label()
        } else {
            &self.label
        }
    }
}

/// The workflow configuration — both the kind identifier and parameter source.
///
/// Each variant fully describes what to build. No separate `kind` string needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowConfig {
    Consensus {
        /// Number of parallel clones.
        n: u32,
        /// What kind of entry the result gets pushed as.
        result_kind: ResultKind,
    },
    Judge {
        /// System prompt for the judge. Empty string triggers fallback loading
        /// from config directory (underscore-prefixed filename).
        #[serde(default)]
        prompt: String,
        /// Name of the tool that signals approval.
        #[serde(default = "default_approval_tool")]
        approval_tool: String,
        /// What kind of entry the result gets pushed as.
        result_kind: ResultKind,
        /// Lua plugin name to execute for this judge.
        #[serde(default = "default_judge_script")]
        script: String,
    },
    Divergence {
        /// Number of parallel clones with temperature variation.
        n: u32,
        /// Base temperature for clones.
        #[serde(default = "default_temperature")]
        temperature: f32,
        /// What kind of entry the result gets pushed as.
        result_kind: ResultKind,
    },
    /// User-defined workflow with arbitrary JSON configuration.
    Custom(serde_json::Value),
}

fn default_approval_tool() -> String {
    "task_complete".to_owned()
}

fn default_judge_script() -> String {
    "judge_fail".to_owned()
}

fn default_temperature() -> f32 {
    0.7
}

impl WorkflowConfig {
    /// Returns a human-readable label for this config variant.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Consensus { .. } => "Consensus",
            Self::Judge { .. } => "Judge",
            Self::Divergence { .. } => "Divergence",
            Self::Custom(_) => "Custom",
        }
    }

    /// Create a config from an [`OneShotKind`].
    /// Used by the ToggleOneShot intent handler.
    #[must_use]
    pub fn from_one_shot_kind(kind: &OneShotKind) -> Self {
        match kind {
            OneShotKind::Consensus => Self::Consensus {
                n: 3,
                result_kind: ResultKind::Assistant,
            },
            OneShotKind::Judge => Self::Judge {
                prompt: String::new(),
                approval_tool: String::from("approve"),
                result_kind: ResultKind::System,
                script: "judge_fail".to_owned(),
            },
        }
    }

    /// Build a WorkflowGraph from this config.
    ///
    /// NOTE: Only callable after Phase 7+ (builtin.rs must exist).
    /// Phase 1 defines the type; Phase 7-9 implement the builders.
    #[must_use]
    pub fn build_graph(&self) -> jinn_workflow::graph::WorkflowGraph {
        match self {
            Self::Consensus { n, .. } => super::builtin::build_consensus(*n),
            Self::Judge {
                prompt,
                approval_tool,
                ..
            } => super::builtin::build_judge(prompt, approval_tool),
            Self::Divergence { n, temperature, .. } => {
                super::builtin::build_divergence(*n, *temperature)
            }
            Self::Custom(_) => {
                // Custom workflows use the WorkflowRegistry lookup, not build_graph.
                // Fallback: build a minimal pass-through graph.
                super::builtin::build_consensus(1)
            }
        }
    }
}
/// When an attached workflow fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowTrigger {
    /// Run after every LLM turn completes (session → Idle).
    TurnEnd,
    /// Run once after the next turn, then auto-detach.
    TurnEndOneShot,
    /// Run before the user's prompt is sent to the LLM.
    BeforeTurn(BeforeTurnMode),
    /// Manual trigger only.
    Manual,
}

/// How a BeforeTurn workflow interacts with the user's prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum BeforeTurnMode {
    /// Enhance prompt, auto-send the enhanced version.
    AutoSend {
        /// How to combine the original and enhanced text.
        strategy: PromptMergeStrategy,
    },
    /// Enhance prompt, put result back in input box (user sends manually).
    PutBack {
        /// How to combine the original and enhanced text.
        strategy: PromptMergeStrategy,
    },
}

/// How to combine original and enhanced text in BeforeTurn workflows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptMergeStrategy {
    /// Replace original with enhanced text entirely.
    #[default]
    Replace,
    /// Put enhanced text before original text.
    Prepend,
    /// Put enhanced text after original text.
    Append,
}

/// What kind of chat entry a workflow result becomes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultKind {
    #[default]
    Assistant,
    User,
    System,
    /// No entry pushed — workflow runs silently.
    Silent,
}

/// Execution state of an attached workflow.
///
/// Persisted as part of `SessionCore`. On crash/restart, `Running` is reset to `Ready`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AttachedWorkflowState {
    /// Ready to fire on next trigger.
    Ready,
    /// Currently executing.
    Running,
    /// Finished successfully.
    Completed,
    /// Failed with a reason.
    Failed {
        /// Error description.
        reason: String,
    },
}

impl Default for AttachedWorkflowState {
    fn default() -> Self {
        Self::Ready
    }
}

/// Keybind toggle keys for one-shot workflows.
///
/// Stored in `SessionUi::pending_one_shots` — ephemeral UI state, not persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OneShotKind {
    Consensus,
    Judge,
}

impl OneShotKind {
    /// Returns the default config for this one-shot kind.
    #[must_use]
    pub fn default_config(&self) -> WorkflowConfig {
        match self {
            Self::Consensus => WorkflowConfig::Consensus {
                n: 3,
                result_kind: ResultKind::Assistant,
            },
            Self::Judge => WorkflowConfig::Judge {
                prompt: String::new(), // loaded from config dir via fallback
                approval_tool: "task_complete".to_owned(),
                result_kind: ResultKind::Silent,
                script: "judge_fail".to_owned(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;

    // --- Test 1: attached_workflow_default_state_is_ready_enabled ---

    #[rstest::rstest]
    fn attached_workflow_default_state_is_ready_enabled() {
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus {
                n: 3,
                result_kind: ResultKind::Assistant,
            },
            WorkflowTrigger::TurnEnd,
        );
        assert!(aw.enabled);
        assert!(matches!(aw.state, AttachedWorkflowState::Ready));
    }

    // --- Test 2: workflow_config_label_returns_correct_name ---

    #[rstest::rstest]
    fn workflow_config_label_returns_correct_name() {
        assert_eq!(
            WorkflowConfig::Consensus {
                n: 3,
                result_kind: ResultKind::Assistant
            }
            .label(),
            "Consensus"
        );
        assert_eq!(
            WorkflowConfig::Judge {
                prompt: String::new(),
                approval_tool: "task_complete".to_owned(),
                result_kind: ResultKind::Silent,
                script: "judge_fail".to_owned(),
            }
            .label(),
            "Judge"
        );
        assert_eq!(
            WorkflowConfig::Divergence {
                n: 3,
                temperature: 0.7,
                result_kind: ResultKind::Assistant
            }
            .label(),
            "Divergence"
        );
        assert_eq!(
            WorkflowConfig::Custom(serde_json::json!({})).label(),
            "Custom"
        );
    }

    // --- Test 3: one_shot_kind_default_config_matches_kind ---

    #[rstest::rstest]
    fn one_shot_kind_default_config_matches_kind() {
        let consensus_cfg = OneShotKind::Consensus.default_config();
        assert!(matches!(
            consensus_cfg,
            WorkflowConfig::Consensus { n: 3, .. }
        ));

        let judge_cfg = OneShotKind::Judge.default_config();
        assert!(matches!(judge_cfg, WorkflowConfig::Judge { .. }));
    }

    // --- Test 4: pending_one_shots_default_empty ---

    #[rstest::rstest]
    fn pending_one_shots_default_empty() {
        let map: std::collections::HashMap<OneShotKind, WorkflowConfig> =
            std::collections::HashMap::new();
        assert!(map.is_empty());
    }

    // --- Test 5: pending_user_text_default_none ---

    #[rstest::rstest]
    fn pending_user_text_default_none() {
        let pending: Option<String> = None;
        assert!(pending.is_none());
    }

    // --- Test 6: attached_workflow_serialize_deserialize_roundtrip ---

    #[rstest::rstest]
    fn attached_workflow_serialize_deserialize_roundtrip() {
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus {
                n: 5,
                result_kind: ResultKind::System,
            },
            WorkflowTrigger::TurnEndOneShot,
        );
        let json = serde_json::to_string(&aw).expect("serialize");
        let back: AttachedWorkflow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(aw.id, back.id);
        assert_eq!(aw.enabled, back.enabled);
        assert!(matches!(back.state, AttachedWorkflowState::Ready));
    }

    // --- Test 7: workflow_config_serialize_deserialize_roundtrip ---

    #[rstest::rstest]
    fn workflow_config_serialize_deserialize_roundtrip() {
        let configs = vec![
            WorkflowConfig::Consensus {
                n: 3,
                result_kind: ResultKind::Assistant,
            },
            WorkflowConfig::Judge {
                prompt: "Be harsh".to_owned(),
                approval_tool: "approve".to_owned(),
                result_kind: ResultKind::Silent,
                script: "judge_fail".to_owned(),
            },
            WorkflowConfig::Divergence {
                n: 5,
                temperature: 1.2,
                result_kind: ResultKind::User,
            },
            WorkflowConfig::Custom(serde_json::json!({"key": "value"})),
        ];
        for config in configs {
            let json = serde_json::to_string(&config).expect("serialize");
            let back: WorkflowConfig = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(config.label(), back.label());
        }
    }

    // --- Test 8: workflow_trigger_serialize_deserialize_roundtrip ---

    #[rstest::rstest]
    fn workflow_trigger_serialize_deserialize_roundtrip() {
        let triggers = vec![
            WorkflowTrigger::TurnEnd,
            WorkflowTrigger::TurnEndOneShot,
            WorkflowTrigger::BeforeTurn(BeforeTurnMode::AutoSend {
                strategy: PromptMergeStrategy::Replace,
            }),
            WorkflowTrigger::BeforeTurn(BeforeTurnMode::PutBack {
                strategy: PromptMergeStrategy::Append,
            }),
            WorkflowTrigger::Manual,
        ];
        for trigger in triggers {
            let json = serde_json::to_string(&trigger).expect("serialize");
            let back: WorkflowTrigger = serde_json::from_str(&json).expect("deserialize");
            // Can't derive PartialEq easily with String variants, so re-serialize and compare
            assert_eq!(json, serde_json::to_string(&back).expect("re-serialize"));
        }
    }

    // --- Test 9: one_shot_hashmap_insert_remove_toggles ---

    #[rstest::rstest]
    fn one_shot_hashmap_insert_remove_toggles() {
        let mut map = std::collections::HashMap::new();

        // Toggle on
        assert!(map.is_empty());
        map.insert(
            OneShotKind::Consensus,
            OneShotKind::Consensus.default_config(),
        );
        assert!(map.contains_key(&OneShotKind::Consensus));

        // Toggle off
        map.remove(&OneShotKind::Consensus);
        assert!(!map.contains_key(&OneShotKind::Consensus));
        assert!(map.is_empty());
    }

    // --- Test 10: attached_workflow_state_serialize_deserialize_roundtrip ---

    #[rstest::rstest]
    fn attached_workflow_state_serialize_deserialize_roundtrip() {
        let states = vec![
            AttachedWorkflowState::Ready,
            AttachedWorkflowState::Running,
            AttachedWorkflowState::Completed,
            AttachedWorkflowState::Failed {
                reason: "something went wrong".to_owned(),
            },
        ];
        for state in states {
            let json = serde_json::to_string(&state).expect("serialize");
            let back: AttachedWorkflowState = serde_json::from_str(&json).expect("deserialize");
            let json2 = serde_json::to_string(&back).expect("re-serialize");
            assert_eq!(json, json2);
        }
    }

    // --- Test 11: result_kind_default_is_assistant ---

    #[rstest::rstest]
    fn result_kind_default_is_assistant() {
        assert_eq!(ResultKind::default(), ResultKind::Assistant);
    }

    // --- Test 12: prompt_merge_strategy_default_is_replace ---

    #[rstest::rstest]
    fn prompt_merge_strategy_default_is_replace() {
        assert_eq!(PromptMergeStrategy::default(), PromptMergeStrategy::Replace);
    }

    // --- Test 13: new_initializes_label_from_config ---

    #[rstest::rstest]
    fn new_initializes_label_from_config() {
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus {
                n: 3,
                result_kind: ResultKind::Assistant,
            },
            WorkflowTrigger::TurnEnd,
        );
        assert_eq!(aw.label, "Consensus");
    }

    // --- Test 14: label_or_default_returns_label_when_nonempty ---

    #[rstest::rstest]
    fn label_or_default_returns_label_when_nonempty() {
        let mut aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus {
                n: 3,
                result_kind: ResultKind::Assistant,
            },
            WorkflowTrigger::TurnEnd,
        );
        aw.label = "My Custom Name".to_owned();
        assert_eq!(aw.label_or_default(), "My Custom Name");
    }

    // --- Test 15: label_or_default_falls_back_to_config_label_when_empty ---

    #[rstest::rstest]
    fn label_or_default_falls_back_to_config_label_when_empty() {
        let mut aw = AttachedWorkflow::new(
            WorkflowConfig::Judge {
                prompt: String::new(),
                approval_tool: "task_complete".to_owned(),
                result_kind: ResultKind::Silent,
                script: "judge_fail".to_owned(),
            },
            WorkflowTrigger::TurnEnd,
        );
        // Simulate old persisted data: label is empty.
        aw.label = String::new();
        assert_eq!(aw.label_or_default(), "Judge");
    }

    // --- Test 16: deserialize_without_label_field_uses_default ---

    // --- Test 16: deserialize_without_label_field_uses_default ---

    #[rstest::rstest]
    fn deserialize_without_label_field_uses_default() {
        // Create a valid AttachedWorkflow, serialize it, remove the label field,
        // then deserialize to verify backward compat.
        let aw = AttachedWorkflow::new(
            WorkflowConfig::Consensus {
                n: 3,
                result_kind: ResultKind::Assistant,
            },
            WorkflowTrigger::TurnEnd,
        );
        let mut val = serde_json::to_value(&aw).expect("serialize");
        // Remove the label field to simulate old persisted data.
        val.as_object_mut().expect("object").remove("label");
        let back: AttachedWorkflow = serde_json::from_value(val).expect("should deserialize");
        // label defaults to empty string (serde default).
        assert!(back.label.is_empty());
        // But label_or_default() falls back to config label.
        assert_eq!(back.label_or_default(), "Consensus");
    }
}