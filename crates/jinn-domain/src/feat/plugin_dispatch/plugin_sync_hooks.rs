//! Plugin sync hooks trait — direct render-thread access to plugin hooks.
//!
//! Non-`Send` counterpart to [`super::PluginSyncCall`]. Where `PluginSyncCall`
//! channels to the async VM (`Send + Sync`), this trait calls hooks directly on
//! the render-thread Lua state via `SyncPlugins`. Implemented by
//! `jinn_plugin::SyncPlugins`.
//!
//! # Why a separate trait
//!
//! [`super::PluginSyncCall`] runs hooks on the *async* plugin VM by channeling
//! from the calling actor thread. Sync interception and badge rendering must run
//! on the *render thread's* `sync_lua` state (the same Lua state that reads the
//! toggle flag), with zero channel hops. That state is `!Send`
//! (`mlua::Lua` is `!Send`), so this trait is intentionally non-`Send`.

use serde::de::DeserializeOwned;
use serde_json::Value;

/// Call plugin hooks synchronously, directly on the caller's thread.
///
/// Non-`Send`: implementations own a `!Send` Lua state and must only be called
/// from the render thread. Returns non-nil hook values as raw JSON; callers
/// deserialize via [`call_hooks_typed`].
///
/// Implemented by `jinn_plugin::SyncPlugins`.
pub trait PluginSyncHooks {
    /// Call all hooks for the given name, returning non-nil results as JSON.
    fn call_hooks(&self, hook: &str, ctx: &Value) -> Vec<Value>;
}

/// Typed wrapper over [`PluginSyncHooks::call_hooks`].
///
/// Deserializes each returned value into `T`. Malformed returns are silently
/// dropped with a `warn!` log — a buggy plugin degrades rather than panicking
/// the render thread.
#[must_use]
pub fn call_hooks_typed<T: DeserializeOwned>(
    plugins: &dyn PluginSyncHooks,
    hook: &str,
    ctx: &Value,
) -> Vec<T> {
    plugins
        .call_hooks(hook, ctx)
        .into_iter()
        .filter_map(|v| match serde_json::from_value::<T>(v) {
            Ok(t) => Some(t),
            Err(e) => {
                tracing::warn!(hook, error = %e, "plugin returned malformed value; dropped");
                None
            }
        })
        .collect()
}

/// Outcome returned by an interception hook (`on_submit_intercept`).
///
/// Lua returns one of:
/// - `{ action = "block" }` — drop the pending commands.
/// - `{ action = "pass" }` — leave commands unchanged.
/// - `{ action = "replace", commands = { ... } }` — swap the pending commands.
#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
#[serde(tag = "action")]
pub enum InterceptOutcome {
    /// Drop the pending commands.
    #[serde(rename = "block")]
    Block,
    /// Leave the pending commands unchanged.
    #[serde(rename = "pass")]
    Pass,
    /// Replace the pending commands with the given JSON-encoded commands.
    #[serde(rename = "replace")]
    Replace { commands: Vec<Value> },
}

/// Declarative render directive from `on_chat_input_badges_render`.
///
/// Rust owns the slot layout and a constrained style vocabulary; the plugin
/// declares `slot` and a list of styled segments drawn left-to-right within it.
#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
pub struct BadgeDirective {
    /// Named location, e.g. `"input_badge"`.
    pub slot: String,
    /// Styled segments drawn left-to-right within the slot.
    #[serde(default)]
    pub segments: Vec<BadgeSegment>,
}

/// One styled run within a [`BadgeDirective`].
///
/// `style` names a constrained vocabulary entry resolved by the host against
/// the active theme (e.g. `"accent_action"`, `"muted_text"`, `"yellow"`);
/// `None` yields the default style.
#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
pub struct BadgeSegment {
    /// Text for this run.
    pub text: String,
    /// Optional constrained style name.
    #[serde(default)]
    pub style: Option<String>,
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

    /// Stub backend returning a fixed list of JSON values.
    struct Stub {
        values: Vec<Value>,
    }

    impl PluginSyncHooks for Stub {
        fn call_hooks(&self, _hook: &str, _ctx: &Value) -> Vec<Value> {
            self.values.clone()
        }
    }

    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct Pair {
        a: u32,
        b: String,
    }

    #[test]
    fn typed_wrapper_deserializes_all_valid_returns() {
        let stub = Stub {
            values: vec![
                serde_json::json!({ "a": 1, "b": "x" }),
                serde_json::json!({ "a": 2, "b": "y" }),
            ],
        };
        let got: Vec<Pair> = call_hooks_typed(&stub, "any", &Value::Null);
        assert_eq!(
            got,
            vec![
                Pair {
                    a: 1,
                    b: "x".to_owned()
                },
                Pair {
                    a: 2,
                    b: "y".to_owned()
                },
            ]
        );
    }

    #[test]
    fn typed_wrapper_drops_malformed_returns() {
        let stub = Stub {
            values: vec![
                serde_json::json!({ "a": 1, "b": "ok" }),
                serde_json::json!({ "a": "not-a-number" }), // malformed
                serde_json::json!({ "a": 3, "b": "z" }),
            ],
        };
        let got: Vec<Pair> = call_hooks_typed(&stub, "any", &Value::Null);
        assert_eq!(
            got,
            vec![
                Pair {
                    a: 1,
                    b: "ok".to_owned()
                },
                Pair {
                    a: 3,
                    b: "z".to_owned()
                },
            ]
        );
    }

    #[test]
    fn typed_wrapper_empty_when_all_malformed() {
        let stub = Stub {
            values: vec![Value::Null, serde_json::json!("string")],
        };
        let got: Vec<Pair> = call_hooks_typed(&stub, "any", &Value::Null);
        assert!(got.is_empty());
    }

    #[test]
    fn intercept_outcome_deserializes_block_pass_replace() {
        let block: InterceptOutcome =
            serde_json::from_value(serde_json::json!({ "action": "block" })).unwrap();
        assert_eq!(block, InterceptOutcome::Block);

        let pass: InterceptOutcome =
            serde_json::from_value(serde_json::json!({ "action": "pass" })).unwrap();
        assert_eq!(pass, InterceptOutcome::Pass);

        let replace: InterceptOutcome = serde_json::from_value(serde_json::json!({
            "action": "replace",
            "commands": [{ "verb": "noop" }]
        }))
        .unwrap();
        let InterceptOutcome::Replace { commands } = replace else {
            panic!("expected Replace");
        };
        assert_eq!(commands.len(), 1);
    }

    #[test]
    fn badge_directive_deserializes_segments_with_styles() {
        let d: BadgeDirective = serde_json::from_value(serde_json::json!({
            "slot": "input_badge",
            "segments": [
                { "text": "[", "style": "muted_text" },
                { "text": "E", "style": "accent_action" },
                { "text": "nrich]" }
            ]
        }))
        .unwrap();
        assert_eq!(d.slot, "input_badge");
        assert_eq!(d.segments.len(), 3);
        assert_eq!(d.segments[0].text, "[");
        assert_eq!(d.segments[0].style.as_deref(), Some("muted_text"));
        assert_eq!(d.segments[1].text, "E");
        assert_eq!(d.segments[1].style.as_deref(), Some("accent_action"));
        assert_eq!(d.segments[2].text, "nrich]");
        assert!(d.segments[2].style.is_none());
    }

    #[test]
    fn badge_directive_defaults_segments_to_empty_when_absent() {
        let d: BadgeDirective =
            serde_json::from_value(serde_json::json!({ "slot": "input_badge" })).unwrap();
        assert!(d.segments.is_empty());
    }

    #[test]
    fn trait_is_object_safe() {
        // Compiles only if the trait is object-safe.
        let stub: &dyn PluginSyncHooks = &Stub { values: vec![] };
        let _ = stub.call_hooks("any", &Value::Null);
    }
}
