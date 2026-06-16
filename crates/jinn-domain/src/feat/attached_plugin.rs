//! Per-session plugin attachment model.
//!
//! Replaces the old `AttachedWorkflow` machinery. Plugins attach to a session
//! by name; the dispatcher (`PluginDispatchActor`) loads them into a per-session
//! Lua state and fires their hooks at lifecycle events.
//!
//! Each attachment gets a stable [`PluginInstanceId`] so that two attachments
//! of the *same* plugin (e.g. a panel of judges) are distinguishable everywhere
//! identity matters: hook firing, plugin data scoping, and cross-instance
//! coordination.
//!
//! See `crates/jinn-plugin/src/lib.rs` for the four access patterns.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::protocol::SessionId;

/// Stable unique identifier for a single attached-plugin *instance*.
///
/// Generated when an [`AttachedPlugin`] is created and persisted with it, so
/// the identity survives restarts. Two attachments of the same plugin name get
/// distinct ids. Old persisted data lacking the field hydrates a fresh id via
/// `#[serde(default)]`.
///
/// Stored as an opaque string and derives equality/hashing so it can be used
/// as a `HashMap` key (the per-session hooks map and the plugin-data store both
/// key on it).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PluginInstanceId(String);

impl PluginInstanceId {
    /// Generate a new unique instance id using UUID v7.
    #[must_use]
    pub fn new() -> Self {
        Self(format!("i-{}", Uuid::now_v7()))
    }
}

impl Default for PluginInstanceId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for PluginInstanceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A plugin attached to a session.
///
/// Stored in `SessionCore::attached_plugins`. Persists across restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachedPlugin {
    /// Plugin name — selects which `init.lua` to load. Matches the plugin
    /// directory name under `plugins/attachable/`.
    pub name: String,
    /// Stable unique identity of this attachment. Two attachments of the same
    /// plugin name get distinct ids. Used to key the per-session hooks map and
    /// the plugin-data store so duplicate instances are isolated and fire
    /// independently. Old persisted data lacking the field hydrates a fresh id
    /// via `#[serde(default)]`.
    #[serde(default = "PluginInstanceId::new")]
    pub instance_id: PluginInstanceId,
    /// User-editable display label. Defaults to the plugin name at construction.
    /// Empty string falls back to `name` (backward-compat for persisted data
    /// that lacks the `label` field).
    #[serde(default)]
    pub label: String,
    /// Whether this attachment is active. `false` = skip on fire.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Current execution state. Reset to `Idle` on actor activate (crash rehydration).
    #[serde(default)]
    pub run_state: PluginRunState,
    /// Session ID managed by this plugin (e.g. a judge's child session).
    /// Written by the plugin via `ctx.emit('set_managed_session', ...)`.
    /// Used by the sidebar to preview and activate the plugin's session.
    #[serde(default)]
    pub managed_session_id: Option<SessionId>,
}

fn default_true() -> bool {
    true
}

impl AttachedPlugin {
    /// Construct a new attachment with `enabled = true`, `run_state = Idle`,
    /// and `label = name`.
    #[must_use]
    pub fn new<S>(name: S) -> Self
    where
        S: Into<String>,
    {
        let name = name.into();
        let label = name.clone();
        Self {
            name,
            instance_id: PluginInstanceId::new(),
            label,
            enabled: true,
            run_state: PluginRunState::Idle,
            managed_session_id: None,
        }
    }
    /// Display label, falling back to the plugin name when empty.
    #[must_use]
    pub fn label_or_name(&self) -> &str {
        if self.label.is_empty() {
            &self.name
        } else {
            &self.label
        }
    }
}

/// Execution state of an attached plugin.
///
/// Persisted as part of `SessionCore`. On crash/restart, `Running` is reset to `Idle`
/// by the dispatcher's rehydrate step.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PluginRunState {
    /// Idle — not currently firing.
    #[default]
    Idle,
    /// Currently executing a hook.
    Running,
    /// Last hook finished without error.
    Completed,
    /// Last hook errored.
    Failed {
        /// Error description (developer-facing).
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::*;

    #[rstest::rstest]
    fn new_attachment_starts_enabled_idle_with_name_label() {
        let p = AttachedPlugin::new("judge_fail");
        assert_eq!(p.name, "judge_fail");
        assert_eq!(p.label, "judge_fail");
        assert!(p.enabled);
        assert!(matches!(p.run_state, PluginRunState::Idle));
    }

    #[rstest::rstest]
    fn label_or_name_returns_label_when_nonempty() {
        let mut p = AttachedPlugin::new("judge_fail");
        p.label = "Custom Title".to_owned();
        assert_eq!(p.label_or_name(), "Custom Title");
    }

    #[rstest::rstest]
    fn label_or_name_falls_back_to_name_when_empty() {
        let mut p = AttachedPlugin::new("judge_fail");
        p.label = String::new();
        assert_eq!(p.label_or_name(), "judge_fail");
    }

    #[rstest::rstest]
    fn deserialize_without_label_uses_empty_default() {
        let p = AttachedPlugin::new("judge_fail");
        let mut v = serde_json::to_value(&p).expect("serialize");
        v.as_object_mut().expect("object").remove("label");
        let back: AttachedPlugin = serde_json::from_value(v).expect("deserialize");
        assert!(back.label.is_empty());
        assert_eq!(back.label_or_name(), "judge_fail");
    }

    #[rstest::rstest]
    fn deserialize_without_enabled_defaults_true() {
        let p = AttachedPlugin::new("x");
        let mut v = serde_json::to_value(&p).expect("serialize");
        v.as_object_mut().expect("object").remove("enabled");
        let back: AttachedPlugin = serde_json::from_value(v).expect("deserialize");
        assert!(back.enabled, "missing enabled field should default to true");
    }

    #[rstest::rstest]
    fn deserialize_without_run_state_defaults_idle() {
        let p = AttachedPlugin::new("x");
        let mut v = serde_json::to_value(&p).expect("serialize");
        v.as_object_mut().expect("object").remove("run_state");
        let back: AttachedPlugin = serde_json::from_value(v).expect("deserialize");
        assert!(matches!(back.run_state, PluginRunState::Idle));
    }

    #[rstest::rstest]
    fn run_state_roundtrip() {
        let states = vec![
            PluginRunState::Idle,
            PluginRunState::Running,
            PluginRunState::Completed,
            PluginRunState::Failed {
                reason: "boom".to_owned(),
            },
        ];
        for s in states {
            let j = serde_json::to_string(&s).expect("s");
            let back: PluginRunState = serde_json::from_str(&j).expect("d");
            let j2 = serde_json::to_string(&back).expect("s2");
            assert_eq!(j, j2);
        }
    }

    #[rstest::rstest]
    fn new_attachment_generates_nonempty_instance_id() {
        // Given two newly-constructed attachments.
        let a = AttachedPlugin::new("judge");
        let b = AttachedPlugin::new("judge");

        // Then each has a non-empty instance id and they differ.
        assert!(!a.instance_id.to_string().is_empty());
        assert!(!b.instance_id.to_string().is_empty());
        assert_ne!(a.instance_id, b.instance_id);
    }

    #[rstest::rstest]
    fn instance_id_survives_serde_roundtrip() {
        // Given an attachment with a generated instance id.
        let p = AttachedPlugin::new("judge");
        let original_id = p.instance_id.clone();

        // When serializing and deserializing.
        let j = serde_json::to_string(&p).expect("serialize");
        let back: AttachedPlugin = serde_json::from_str(&j).expect("deserialize");

        // Then the instance id is preserved.
        assert_eq!(back.instance_id, original_id);
    }

    #[rstest::rstest]
    fn instance_id_missing_in_json_hydrates_fresh_id() {
        // Given a serialized attachment with the instance_id field removed
        // (simulating persisted data from before this field existed).
        let p = AttachedPlugin::new("judge");
        let mut v = serde_json::to_value(&p).expect("serialize");
        v.as_object_mut().expect("object").remove("instance_id");

        // When deserializing.
        let back: AttachedPlugin = serde_json::from_value(v).expect("deserialize");

        // Then a fresh (non-empty) id is generated and differs from the original.
        assert!(!back.instance_id.to_string().is_empty());
        assert_ne!(back.instance_id, p.instance_id);
    }
}
