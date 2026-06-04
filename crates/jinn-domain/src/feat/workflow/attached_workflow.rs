//! Attached workflow types — persistent data model for workflows bound to sessions.
//!
//! Attached workflows are persistent fields on [`SessionCore`](crate::feat::session::chat_session::SessionCore),
//! visible in the session sidebar, and triggered by session lifecycle events.
//!
//! Key types:
//! - [`AttachedWorkflow`] — the persistent attachment on a session
//! - [`WorkflowConfig`] — identifies which Lua plugin to run and with what data
//! - [`WorkflowTrigger`] — when the workflow fires
//! - [`OneShotKind`] — keybind toggle keys for one-shot workflows

use serde::{Deserialize, Serialize};

/// Unique identifier for a workflow attachment.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct WorkflowId(String);

impl WorkflowId {
    /// Create a new unique workflow ID.
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7().to_string())
    }
}

impl std::fmt::Display for WorkflowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Default for WorkflowId {
    fn default() -> Self {
        Self::new()
    }
}

/// A workflow that is persistently attached to a session.
///
/// Stored in `SessionCore::attached_workflows`. Persists across restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachedWorkflow {
    /// Unique ID for this attachment.
    pub id: WorkflowId,
    /// The configuration (plugin name + data) for this workflow.
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

/// The workflow configuration — identifies a Lua plugin and provides optional data.
///
/// The `script` field is the plugin directory name (e.g., `"judge_fail"`).
/// The `data` field is arbitrary JSON injected into the Lua `ctx` at spawn time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowConfig {
    /// Plugin name — selects which `init.lua` to run.
    pub script: String,
    /// Arbitrary data injected into the Lua ctx at spawn time.
    #[serde(default)]
    pub data: serde_json::Value,
}

impl WorkflowConfig {
    /// Returns the script name as the human-readable label.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.script
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

/// Execution state of an attached workflow.
///
/// Persisted as part of `SessionCore`. On crash/restart, `Running` is reset to `Ready`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[derive(Default)]
pub enum AttachedWorkflowState {
    /// Ready to fire on next trigger.
    #[default]
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
            Self::Consensus => WorkflowConfig {
                script: "judge_fail".to_owned(),
                data: serde_json::json!({}),
            },
            Self::Judge => WorkflowConfig {
                script: "judge_fail".to_owned(),
                data: serde_json::json!({}),
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
            WorkflowConfig {
                script: "judge_fail".to_owned(),
                data: serde_json::json!({}),
            },
            WorkflowTrigger::TurnEnd,
        );
        assert!(aw.enabled);
        assert!(matches!(aw.state, AttachedWorkflowState::Ready));
    }

    // --- Test 2: workflow_config_label_returns_script_name ---

    #[rstest::rstest]
    fn workflow_config_label_returns_script_name() {
        let config = WorkflowConfig {
            script: "my_plugin".to_owned(),
            data: serde_json::json!({}),
        };
        assert_eq!(config.label(), "my_plugin");
    }

    // --- Test 3: one_shot_kind_default_config_has_judge_fail_script ---

    #[rstest::rstest]
    fn one_shot_kind_default_config_has_judge_fail_script() {
        let cfg = OneShotKind::Judge.default_config();
        assert_eq!(cfg.script, "judge_fail");
    }

    // --- Test 4: pending_one_shots_default_empty ---

    #[rstest::rstest]
    fn pending_one_shots_default_empty() {
        let map: std::collections::HashMap<OneShotKind, WorkflowConfig> =
            std::collections::HashMap::new();
        assert!(map.is_empty());
    }

    // --- Test 5: attached_workflow_serialize_deserialize_roundtrip ---

    #[rstest::rstest]
    fn attached_workflow_serialize_deserialize_roundtrip() {
        let aw = AttachedWorkflow::new(
            WorkflowConfig {
                script: "my_plugin".to_owned(),
                data: serde_json::json!({"key": "value"}),
            },
            WorkflowTrigger::TurnEndOneShot,
        );
        let json = serde_json::to_string(&aw).expect("serialize");
        let back: AttachedWorkflow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(aw.id, back.id);
        assert_eq!(aw.enabled, back.enabled);
        assert!(matches!(back.state, AttachedWorkflowState::Ready));
        assert_eq!(back.config.script, "my_plugin");
    }

    // --- Test 6: workflow_config_serialize_deserialize_roundtrip ---

    #[rstest::rstest]
    fn workflow_config_serialize_deserialize_roundtrip() {
        let config = WorkflowConfig {
            script: "test_plugin".to_owned(),
            data: serde_json::json!({"n": 3}),
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let back: WorkflowConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(config.script, back.script);
    }

    // --- Test 7: workflow_trigger_serialize_deserialize_roundtrip ---

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
            let json2 = serde_json::to_string(&back).expect("re-serialize");
            assert_eq!(json, json2);
        }
    }

    // --- Test 8: one_shot_hashmap_insert_remove_toggles ---

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

    // --- Test 9: attached_workflow_state_serialize_deserialize_roundtrip ---

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

    // --- Test 10: new_initializes_label_from_config ---

    #[rstest::rstest]
    fn new_initializes_label_from_config() {
        let aw = AttachedWorkflow::new(
            WorkflowConfig {
                script: "my_plugin".to_owned(),
                data: serde_json::json!({}),
            },
            WorkflowTrigger::TurnEnd,
        );
        assert_eq!(aw.label, "my_plugin");
    }

    // --- Test 11: label_or_default_returns_label_when_nonempty ---

    #[rstest::rstest]
    fn label_or_default_returns_label_when_nonempty() {
        let mut aw = AttachedWorkflow::new(
            WorkflowConfig {
                script: "my_plugin".to_owned(),
                data: serde_json::json!({}),
            },
            WorkflowTrigger::TurnEnd,
        );
        aw.label = "My Custom Name".to_owned();
        assert_eq!(aw.label_or_default(), "My Custom Name");
    }

    // --- Test 12: label_or_default_falls_back_to_config_label_when_empty ---

    #[rstest::rstest]
    fn label_or_default_falls_back_to_config_label_when_empty() {
        let mut aw = AttachedWorkflow::new(
            WorkflowConfig {
                script: "judge_fail".to_owned(),
                data: serde_json::json!({}),
            },
            WorkflowTrigger::TurnEnd,
        );
        // Simulate old persisted data: label is empty.
        aw.label = String::new();
        assert_eq!(aw.label_or_default(), "judge_fail");
    }

    // --- Test 13: deserialize_without_label_field_uses_default ---

    #[rstest::rstest]
    fn deserialize_without_label_field_uses_default() {
        let aw = AttachedWorkflow::new(
            WorkflowConfig {
                script: "my_plugin".to_owned(),
                data: serde_json::json!({}),
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
        assert_eq!(back.label_or_default(), "my_plugin");
    }

    // --- Test 14: workflow_id_default_creates_unique ---

    #[rstest::rstest]
    fn workflow_id_default_creates_unique() {
        let id1 = WorkflowId::default();
        let id2 = WorkflowId::default();
        assert_ne!(id1, id2);
    }
}
