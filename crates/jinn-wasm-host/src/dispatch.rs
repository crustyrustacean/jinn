//! Typed hook dispatch — the bridge between the domain's `serde_json::Value`
//! ctx payloads and the WIT-typed component exports.
//!
//! ## Why this module exists
//!
//! The domain fires hooks by **string name** (`"on_turn_end"`, …) carrying a
//! `serde_json::Value` ctx. WASM components, however, export **typed** functions
//! resolved through the generated [`Guest`] accessor. Raw `get_func(name)` does
//! not reach them — the exports are kebab-cased (`jinn:plugin/hooks@0.1.0#on-turn-end`)
//! and async.
//!
//! This module owns the two conversions that make the typed boundary work:
//!
//! 1. **`ctx_json → typed record`** — per-hook builders that read the JSON keys
//!    the domain writes (`session_id`, `parent_session_id`, `task_list`, …) and
//!    construct the matching WIT record. Built once, used by both the async and
//!    sync paths.
//! 2. **`typed result → serde_json::Value`** — converts the typed return types
//!    (`BadgeDirective`, `KeybindResult`, `InterceptOutcome`) back to the JSON
//!    shape the `PluginSyncHooks` trait expects, so the TUI call sites are
//!    unchanged.
//!
//! ## Async vs sync dispatch
//!
//! Async hooks (lifecycle + `run-trigger`/`run-tool`) are driven via
//! `Store::run_concurrent`, which lends an [`Accessor`] to the typed
//! `call_*` methods. Sync render hooks call the typed methods directly with the
//! store context. Both resolve the same [`Guest`] accessor from
//! [`StoredInstance::typed_guest`](crate::store::StoredInstance::typed_guest).

use serde_json::Value;

use crate::bindings::jinn::plugin::types::{
    AttachCtx, BadgeCtx, BadgeDirective, BadgeSegment, InterceptOutcome, KeybindResult,
    KeybindTriggerCtx, SessionCtx, SessionPreviewCtx, SubmitInterceptCtx, TaskListCtx,
    ThemeStyle, ToolCtx, TriggerCtx, TurnEndCtx,
};

// ─── JSON field readers ────────────────────────────────────────────────
// The domain writes snake_case keys. These read them defensively — a missing
// or wrong-typed field degrades to a default rather than panicking the
// dispatch. Defensive parsing is intentional: the boundary survives a plugin
// that omits a field.

fn get_str(ctx: &Value, key: &str) -> String {
    ctx.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_default()
}

fn get_opt_str(ctx: &Value, key: &str) -> Option<String> {
    ctx.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn get_u32(ctx: &Value, key: &str) -> u32 {
    ctx.get(key).and_then(Value::as_u64).map(|u| u as u32).unwrap_or(0)
}

fn get_bool(ctx: &Value, key: &str) -> bool {
    ctx.get(key).and_then(Value::as_bool).unwrap_or(false)
}

// ─── ctx_json → typed record builders ─────────────────────────────���────
// One builder per hook ctx type. Each reads exactly the fields that hook's
// WIT record declares (see wit/jinn.wit).

/// Build a `SessionCtx` (used by `on-app-started`, `on-session-created`,
/// `on-user-submit`).
pub fn build_session_ctx(ctx: &Value) -> SessionCtx {
    SessionCtx {
        session_id: get_str(ctx, "session_id"),
        parent_session_id: get_opt_str(ctx, "parent_session_id"),
        instance_id: instance_id_or_default(ctx),
        plugin_name: get_str(ctx, "plugin_name"),
    }
}

/// Build a `TurnEndCtx` (used by `on-turn-end`).
pub fn build_turn_end_ctx(ctx: &Value) -> TurnEndCtx {
    TurnEndCtx {
        session_id: get_str(ctx, "session_id"),
        parent_session_id: get_opt_str(ctx, "parent_session_id"),
        instance_id: instance_id_or_default(ctx),
        plugin_name: get_str(ctx, "plugin_name"),
    }
}

/// Build an `AttachCtx` (used by `on-attach` / `on-detach`).
pub fn build_attach_ctx(ctx: &Value) -> AttachCtx {
    AttachCtx {
        session_id: get_str(ctx, "session_id"),
        instance_id: instance_id_or_default(ctx),
        plugin_name: get_str(ctx, "plugin_name"),
    }
}

/// Build a `TaskListCtx` (used by `on-task-list-updated`).
pub fn build_task_list_ctx(ctx: &Value) -> TaskListCtx {
    TaskListCtx {
        session_id: get_str(ctx, "session_id"),
        instance_id: instance_id_or_default(ctx),
        plugin_name: get_str(ctx, "plugin_name"),
        task_list: get_str(ctx, "task_list"),
        completed: get_u32(ctx, "completed"),
        total: get_u32(ctx, "total"),
        is_complete: get_bool(ctx, "is_complete"),
    }
}

/// Build a `TriggerCtx` (used by `run-trigger`).
pub fn build_trigger_ctx(ctx: &Value) -> TriggerCtx {
    TriggerCtx {
        session_id: get_str(ctx, "session_id"),
        parent_session_id: get_opt_str(ctx, "parent_session_id"),
        instance_id: instance_id_or_default(ctx),
        plugin_name: get_str(ctx, "plugin_name"),
        text: get_str(ctx, "text"),
    }
}

/// Build a `ToolCtx` (used by `run-tool`).
pub fn build_tool_ctx(ctx: &Value) -> ToolCtx {
    ToolCtx {
        session_id: get_str(ctx, "session_id"),
        parent_session_id: get_opt_str(ctx, "parent_session_id"),
        instance_id: instance_id_or_default(ctx),
        plugin_name: get_str(ctx, "plugin_name"),
    }
}

/// Build a `BadgeCtx` (used by `on-chat-input-badges-render`).
pub fn build_badge_ctx(ctx: &Value) -> BadgeCtx {
    BadgeCtx {
        session_id: get_str(ctx, "session_id"),
        active_session_id: get_str(ctx, "active_session_id"),
        instance_id: instance_id_or_default(ctx),
        plugin_name: get_str(ctx, "plugin_name"),
        mode: get_str(ctx, "mode"),
        theme_styles: theme_styles_list(ctx),
    }
}

/// Build a `KeybindTriggerCtx` (used by `on-keybind-trigger`).
pub fn build_keybind_trigger_ctx(ctx: &Value) -> KeybindTriggerCtx {
    KeybindTriggerCtx {
        session_id: get_str(ctx, "session_id"),
        instance_id: instance_id_or_default(ctx),
        plugin_name: get_str(ctx, "plugin_name"),
        hook: get_str(ctx, "hook"),
        text: get_str(ctx, "text"),
        keybound_plugin: get_str(ctx, "keybound_plugin"),
    }
}

/// Build a `SubmitInterceptCtx` (used by `on-submit-intercept`).
pub fn build_submit_intercept_ctx(ctx: &Value) -> SubmitInterceptCtx {
    SubmitInterceptCtx {
        session_id: get_str(ctx, "session_id"),
        instance_id: instance_id_or_default(ctx),
        plugin_name: get_str(ctx, "plugin_name"),
        input_text: get_str(ctx, "input_text"),
    }
}

/// Build a `SessionPreviewCtx` (used by `on-session-preview`).
pub fn build_session_preview_ctx(ctx: &Value) -> SessionPreviewCtx {
    SessionPreviewCtx {
        session_id: get_str(ctx, "session_id"),
        instance_id: instance_id_or_default(ctx),
        plugin_name: get_str(ctx, "plugin_name"),
    }
}

/// Extract the theme-style list from the ctx's `theme_styles` object.
///
/// The domain passes it as a JSON object `{name: name, ...}` (each field name
/// is also its value). WIT wants `list<theme-style>` where each entry is
/// `{name: string}`. Read the object keys; ignore malformed entries.
fn theme_styles_list(ctx: &Value) -> Vec<ThemeStyle> {
    ctx.get("theme_styles")
        .and_then(Value::as_object)
        .map(|m| {
            m.keys()
                .map(|name| ThemeStyle { name: name.clone() })
                .collect()
        })
        .unwrap_or_default()
}

/// The domain does not always know the per-instance id at fire time (global
/// plugins fire by name). Fall back to an empty string; the host-owned bag
/// layer keys globals by plugin name when `session_id` is `None`.
fn instance_id_or_default(ctx: &Value) -> String {
    get_str(ctx, "instance_id")
}

// ─── Typed result → serde_json::Value ───────────────────────────────���──
// The `PluginSyncHooks` trait returns `Vec<Value>`; convert the typed WIT
// results back. Only the sync render hooks produce values.

/// Convert a typed `BadgeDirective` to the JSON shape the TUI expects.
pub fn badge_directive_to_json(d: BadgeDirective) -> Value {
    let segments: Vec<Value> = d
        .segments
        .into_iter()
        .map(|s| {
            serde_json::json!({
                "text": s.text,
                "style": s.style,
            })
        })
        .collect();
    serde_json::json!({
        "slot": d.slot,
        "segments": segments,
    })
}

/// Convert a typed `KeybindResult` to the JSON shape the TUI expects.
pub fn keybind_result_to_json(r: KeybindResult) -> Value {
    match r {
        KeybindResult::Run => serde_json::json!({ "run_action": true }),
        KeybindResult::Skip => serde_json::json!({ "run_action": false }),
    }
}

/// Convert a typed `InterceptOutcome` to the JSON shape the TUI expects.
pub fn intercept_outcome_to_json(o: InterceptOutcome) -> Value {
    match o {
        InterceptOutcome::Pass => serde_json::json!({ "action": "pass" }),
        InterceptOutcome::Block => serde_json::json!({ "action": "block" }),
        InterceptOutcome::Replace(commands) => {
            let cmds: Vec<Value> = commands.into_iter().map(command_to_json).collect();
            serde_json::json!({ "action": "replace", "commands": cmds })
        }
    }
}

fn command_to_json(c: crate::bindings::jinn::plugin::types::Command) -> Value {
    use crate::bindings::jinn::plugin::types::{Command, PushEntryKind};
    match c {
        Command::EnqueueUserMessage(m) => serde_json::json!({
            "verb": "enqueue_user_message", "session_id": m.session_id, "text": m.text,
        }),
        Command::PushChatEntry(e) => {
            let (kind, payload) = match e.kind {
                PushEntryKind::System(s) => ("system", s),
                PushEntryKind::Transient(s) => ("transient", s),
                PushEntryKind::Error(s) => ("error", s),
            };
            serde_json::json!({
                "verb": "push_chat_entry", "session_id": e.session_id,
                "entry": serde_json::json!({ "kind": kind, "text": payload }),
            })
        }
        Command::SetChatInput(s) => serde_json::json!({
            "verb": "set_chat_input", "session_id": s.session_id, "text": s.text,
        }),
        Command::SetChatInputEnabled(s) => serde_json::json!({
            "verb": "set_chat_input_enabled", "session_id": s.session_id, "enabled": s.enabled,
        }),
        Command::ResetSession(s) => serde_json::json!({
            "verb": "reset_session", "session_id": s.session_id,
        }),
        Command::DisablePlugin(d) => serde_json::json!({
            "verb": "disable_plugin", "session_id": d.session_id,
            "plugin_name": d.plugin_name, "instance_id": d.instance_id,
        }),
        Command::EnablePlugin(e) => serde_json::json!({
            "verb": "enable_plugin", "session_id": e.session_id,
            "plugin_name": e.plugin_name, "instance_id": e.instance_id,
        }),
        Command::FireAsyncHook(f) => serde_json::json!({
            "verb": "fire_async_hook", "session_id": f.session_id,
            "hook": f.hook, "text": f.text,
        }),
        Command::SetManagedSession(s) => serde_json::json!({
            "verb": "set_managed_session", "session_id": s.session_id,
            "plugin_name": s.plugin_name, "instance_id": s.instance_id,
            "managed_session_id": s.managed_session_id,
        }),
    }
}

/// A segment helper used by the badge-result builder (mirrors PDK ergonomics).
pub fn segment(text: impl Into<String>, style: Option<String>) -> BadgeSegment {
    BadgeSegment {
        text: text.into(),
        style,
    }
}

// ─── Async typed dispatch ──────────────────────────────────��───────────
// Each dispatcher resolves the typed Guest, extracts the `Copy` `TypedFunc`
// for the hook (None => optional-hook skip), then drives it under
// `store.run_concurrent`, which lends an `Accessor` to `TypedFunc::call`.

/// Dispatch a well-known async lifecycle hook. No-op if the export is absent.
pub async fn dispatch_async_hook(
    inst: &mut crate::store::StoredInstance,
    hook: &str,
    ctx_json: &Value,
) -> wasmtime::Result<()> {
    let guest = inst.typed_guest()?;
    match hook {
        "on_app_started" => {
            let ctx = build_session_ctx(ctx_json);
            inst.store_mut()
                .run_concurrent(async |a| guest.call_on_app_started(a, ctx).await)
                .await??;
        }
        "on_session_created" => {
            let ctx = build_session_ctx(ctx_json);
            inst.store_mut()
                .run_concurrent(async |a| guest.call_on_session_created(a, ctx).await)
                .await??;
        }
        "on_user_submit" => {
            let ctx = build_session_ctx(ctx_json);
            inst.store_mut()
                .run_concurrent(async |a| guest.call_on_user_submit(a, ctx).await)
                .await??;
        }
        "on_turn_end" => {
            let ctx = build_turn_end_ctx(ctx_json);
            inst.store_mut()
                .run_concurrent(async |a| guest.call_on_turn_end(a, ctx).await)
                .await??;
        }
        "on_attach" => {
            let ctx = build_attach_ctx(ctx_json);
            inst.store_mut()
                .run_concurrent(async |a| guest.call_on_attach(a, ctx).await)
                .await??;
        }
        "on_detach" => {
            let ctx = build_attach_ctx(ctx_json);
            inst.store_mut()
                .run_concurrent(async |a| guest.call_on_detach(a, ctx).await)
                .await??;
        }
        "on_task_list_updated" => {
            let ctx = build_task_list_ctx(ctx_json);
            inst.store_mut()
                .run_concurrent(async |a| guest.call_on_task_list_updated(a, ctx).await)
                .await??;
        }
        // Plugin-defined async hook: routed through run-trigger(action, ctx).
        _ => {
            let action = hook.to_owned();
            let ctx = build_trigger_ctx(ctx_json);
            inst.store_mut()
                .run_concurrent(async |a| guest.call_run_trigger(a, action, ctx).await)
                .await??;
        }
    }
    Ok(())
}

/// Dispatch a plugin-defined async tool (`run-tool`).
pub async fn dispatch_run_tool(
    inst: &mut crate::store::StoredInstance,
    name: &str,
    args: &str,
    ctx_json: &Value,
) -> wasmtime::Result<()> {
    let guest = inst.typed_guest()?;
    let name = name.to_owned();
    let args = args.to_owned();
    let ctx = build_tool_ctx(ctx_json);
    inst.store_mut()
        .run_concurrent(async |a| guest.call_run_tool(a, name, args, ctx).await)
        .await??;
    Ok(())
}

// ─── Sync render-hook dispatch ──────────────────────────────────────────
// The render thread calls hooks synchronously through `PluginSyncHooks`.
// Each well-known sync hook resolves to its typed `call_*` method and returns
// an `Option<Value>`: `None` means the plugin produced no directive (absent or
// opted-out); `Some(v)` is the typed result converted to the JSON shape the
// `PluginSyncHooks` trait expects.

/// Dispatch a sync render hook by name, returning the typed result as JSON.
//
// Returns `Ok(None)` when the plugin produced no directive. Trap/errors are
// returned as `Err` so the caller can log and degrade.
pub fn dispatch_sync_hook(
    inst: &mut crate::store::StoredInstance,
    hook: &str,
    ctx_json: &Value,
) -> wasmtime::Result<Option<Value>> {
    let guest = inst.typed_guest()?;
    match hook {
        "on_chat_input_badges_render" => {
            let ctx = build_badge_ctx(ctx_json);
            guest
                .call_on_chat_input_badges_render(inst.store_mut(), &ctx)?
                .map(badge_directive_to_json)
                .map(Ok)
                .transpose()
        }
        "on_keybind_trigger" => {
            let ctx = build_keybind_trigger_ctx(ctx_json);
            guest
                .call_on_keybind_trigger(inst.store_mut(), &ctx)?
                .map(keybind_result_to_json)
                .map(Ok)
                .transpose()
        }
        "on_submit_intercept" => {
            let ctx = build_submit_intercept_ctx(ctx_json);
            guest
                .call_on_submit_intercept(inst.store_mut(), &ctx)?
                .map(intercept_outcome_to_json)
                .map(Ok)
                .transpose()
        }
        "on_session_preview" => {
            let ctx = build_session_preview_ctx(ctx_json);
            guest
                .call_on_session_preview(inst.store_mut(), &ctx)?
                .map(|s| serde_json::json!({ "session_id": s }))
                .map(Ok)
                .transpose()
        }
        _ => Ok(None),
    }
}
