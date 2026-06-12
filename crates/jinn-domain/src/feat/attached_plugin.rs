//! Per-session plugin attachment model.
//!
//! Replaces the old `AttachedWorkflow` machinery. Plugins attach to a session
//! by name; the dispatcher (`PluginDispatchActor`) loads them into a per-session
//! Lua state and fires their hooks at lifecycle events.
//!
//! See `crates/jinn-plugin/src/lib.rs` for the four access patterns.

use serde::{Deserialize, Serialize};

use crate::protocol::SessionId;

/// A plugin attached to a session.
///
/// Stored in `SessionCore::attached_plugins`. Persists across restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachedPlugin {
    /// Plugin name — selects which `init.lua` to load. Matches the plugin
    /// directory name under `plugins/attachable/`.
    pub name: String,
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
}
