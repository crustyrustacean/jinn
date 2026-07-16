//! Typed hook dispatch — the bridge between domain hook contexts and the
//! WIT-typed component exports.
//!
//! ## Two paths
//!
//! - **Async dispatch** ([`dispatch_async_hook`]) — takes a typed
//!   [`HookCtx`](jinn_domain::feat::plugin_dispatch::HookCtx) and maps each
//!   variant to its WIT record before calling the typed component export.
//!   No JSON crosses this boundary.
//! - **Sync dispatch** ([`dispatch_sync_hook`]) — the render-thread sync hooks
//!   still arrive as `serde_json::Value` through the `PluginSyncHooks` seam.
//!   They are mapped to WIT records here, and their typed results are mapped
//!   back to JSON for the TUI call sites.
//!
//! ## Per-instance identity injection
//!
//! The domain fires one ctx to all instances of a hook; it cannot know each
//! instance's `plugin_name` / `instance_id` at fire time. Each `StoredInstance`
//! knows its own identity, so the dispatch overrides those fields after
//! building the WIT record — authoritative identity comes from the store, not
//! the ctx.

use serde_json::Value;

use jinn_core_types::SessionId;
use jinn_domain::feat::plugin_dispatch::HookCtx;
use jinn_domain::feat::plugin_dispatch::plugin_ctx::{
    AttachHookCtx, SessionHookCtx, TaskListHookCtx, ToolHookCtx, TriggerHookCtx, TurnEndHookCtx,
};

use crate::bindings::jinn::plugin::types::{
    AttachCtx, BadgeCtx, BadgeDirective, BadgeSegment, InterceptOutcome, KeybindResult,
    KeybindTriggerCtx, SessionCtx, SessionPreviewCtx, SubmitInterceptCtx, TaskListCtx, ThemeStyle,
    ToolCtx, TriggerCtx, TurnEndCtx,
};

// ─── Domain ctx → WIT record (async path) ───────────────────────────────

fn session_ctx_from(c: &SessionHookCtx, plugin_name: &str, instance_id: &str) -> SessionCtx {
    SessionCtx {
        session_id: c.session_id.to_string(),
        parent_session_id: c.parent_session_id.as_ref().map(|s| s.to_string()),
        instance_id: instance_id.to_owned(),
        plugin_name: plugin_name.to_owned(),
    }
}

fn turn_end_ctx_from(c: &TurnEndHookCtx, plugin_name: &str, instance_id: &str) -> TurnEndCtx {
    TurnEndCtx {
        session_id: c.session_id.to_string(),
        parent_session_id: c.parent_session_id.as_ref().map(|s| s.to_string()),
        instance_id: instance_id.to_owned(),
        plugin_name: plugin_name.to_owned(),
        turn: c.turn,
    }
}

fn attach_ctx_from(c: &AttachHookCtx, plugin_name: &str, instance_id: &str) -> AttachCtx {
    AttachCtx {
        session_id: c.session_id.to_string(),
        instance_id: instance_id.to_owned(),
        plugin_name: plugin_name.to_owned(),
    }
}

fn task_list_ctx_from(c: &TaskListHookCtx, plugin_name: &str, instance_id: &str) -> TaskListCtx {
    TaskListCtx {
        session_id: c.session_id.to_string(),
        instance_id: instance_id.to_owned(),
        plugin_name: plugin_name.to_owned(),
        task_list: c.task_list.clone(),
        completed: c.completed,
        total: c.total,
        is_complete: c.is_complete,
    }
}

fn trigger_ctx_from(c: &TriggerHookCtx, plugin_name: &str, instance_id: &str) -> TriggerCtx {
    TriggerCtx {
        session_id: c.session_id.to_string(),
        parent_session_id: c.parent_session_id.as_ref().map(|s| s.to_string()),
        instance_id: instance_id.to_owned(),
        plugin_name: plugin_name.to_owned(),
        text: c.text.clone(),
    }
}

fn tool_ctx_from(c: &ToolHookCtx, plugin_name: &str, instance_id: &str) -> ToolCtx {
    ToolCtx {
        session_id: c.session_id.to_string(),
        parent_session_id: c.parent_session_id.as_ref().map(|s| s.to_string()),
        instance_id: instance_id.to_owned(),
        plugin_name: plugin_name.to_owned(),
    }
}

// ─── Sync path: JSON field readers ──────────────────────────────────────
// The sync render hooks arrive as JSON through PluginSyncHooks. These read
// the snake_case keys the domain writes. Defensive: missing fields degrade
// to defaults rather than panicking the dispatch.

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
    ctx.get(key)
        .and_then(Value::as_u64)
        .map(|u| u as u32)
        .unwrap_or(0)
}

fn get_bool(ctx: &Value, key: &str) -> bool {
    ctx.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn instance_id_or_default(ctx: &Value) -> String {
    get_str(ctx, "instance_id")
}

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

// ─── Typed result → serde_json::Value (SYNC path only) ───────────────
//
// This JSON layer exists ONLY on the sync seam (badges / keybind trigger /
// submit intercept), which the typed-seam plan (Phase 6) explicitly scoped
// as out-of-scope. The async seam (on_turn_end, on_attach, run_tool, etc.)
// is fully typed — no JSON. The sync PluginSyncHooks trait still returns
// Vec<Value> because the render-thread call sites (badges.rs, app.rs,
// handler.rs) read results as JSON. Typing the sync seam is tracked
// separately; until then, these helpers convert the typed WIT return types
// back into the JSON the trait contract requires. `command_to_json` is
// reachable only from InterceptOutcome::Replace here (and from the
// DynamicCommand bus envelope in command_dispatch.rs — a domain-wide
// name+Value design, not a WASM-boundary concern).

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

pub fn keybind_result_to_json(r: KeybindResult) -> Value {
    match r {
        KeybindResult::Run => serde_json::json!({ "run_action": true }),
        KeybindResult::Skip => serde_json::json!({ "run_action": false }),
    }
}

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

// ─── Async typed dispatch ───────────────────────────────────────────────
// Each dispatcher resolves the typed Guest, matches the hook name against
// its typed export, maps the HookCtx variant to the WIT record, injects
// per-instance identity, then drives the call under store.run_concurrent.

/// Dispatch a well-known async lifecycle hook. No-op if the export is absent.
pub async fn dispatch_async_hook(
    inst: &mut crate::store::StoredInstance,
    hook: &str,
    ctx: &HookCtx,
) -> wasmtime::Result<()> {
    let guest = inst.typed_guest()?;
    let inst_ctx = inst.ctx();
    let plugin_name = inst_ctx.plugin_name.as_str();
    let instance_id = inst_ctx.instance_id.to_string();

    match hook {
        "on_app_started" => {
            let HookCtx::Session(c) = ctx else {
                return Ok(());
            };
            let wit = session_ctx_from(c, plugin_name, &instance_id);
            inst.store_mut()
                .run_concurrent(async |a| guest.call_on_app_started(a, wit).await)
                .await??;
        }
        "on_session_created" => {
            let HookCtx::Session(c) = ctx else {
                return Ok(());
            };
            let wit = session_ctx_from(c, plugin_name, &instance_id);
            inst.store_mut()
                .run_concurrent(async |a| guest.call_on_session_created(a, wit).await)
                .await??;
        }
        "on_user_submit" => {
            let HookCtx::Session(c) = ctx else {
                return Ok(());
            };
            let wit = session_ctx_from(c, plugin_name, &instance_id);
            inst.store_mut()
                .run_concurrent(async |a| guest.call_on_user_submit(a, wit).await)
                .await??;
        }
        "on_turn_end" => {
            let HookCtx::TurnEnd(c) = ctx else {
                return Ok(());
            };
            let wit = turn_end_ctx_from(c, plugin_name, &instance_id);
            inst.store_mut()
                .run_concurrent(async |a| guest.call_on_turn_end(a, wit).await)
                .await??;
        }
        "on_attach" => {
            let HookCtx::Attach(c) = ctx else {
                return Ok(());
            };
            let wit = attach_ctx_from(c, plugin_name, &instance_id);
            inst.store_mut()
                .run_concurrent(async |a| guest.call_on_attach(a, wit).await)
                .await??;
        }
        "on_detach" => {
            let HookCtx::Attach(c) = ctx else {
                return Ok(());
            };
            let wit = attach_ctx_from(c, plugin_name, &instance_id);
            inst.store_mut()
                .run_concurrent(async |a| guest.call_on_detach(a, wit).await)
                .await??;
        }
        "on_task_list_updated" => {
            let HookCtx::TaskList(c) = ctx else {
                return Ok(());
            };
            let wit = task_list_ctx_from(c, plugin_name, &instance_id);
            inst.store_mut()
                .run_concurrent(async |a| guest.call_on_task_list_updated(a, wit).await)
                .await??;
        }
        // Plugin-defined async hook: routed through run-trigger(action, ctx).
        _ => {
            let HookCtx::Trigger(c) = ctx else {
                tracing::warn!(
                    hook,
                    "plugin-defined hook fired with non-Trigger ctx; skipping"
                );
                return Ok(());
            };
            let action = hook.to_owned();
            let wit = trigger_ctx_from(c, plugin_name, &instance_id);
            inst.store_mut()
                .run_concurrent(async |a| guest.call_run_trigger(a, action, wit).await)
                .await??;
        }
    }
    Ok(())
}

/// Dispatch a plugin-defined async tool (`run-tool`). Returns the tool's
/// result string (fed back to the LLM).
pub async fn dispatch_run_tool(
    inst: &mut crate::store::StoredInstance,
    name: &str,
    args: &str,
    session_id: &SessionId,
    parent_session_id: Option<&SessionId>,
) -> wasmtime::Result<String> {
    let guest = inst.typed_guest()?;
    let name = name.to_owned();
    let args = args.to_owned();
    let inst_ctx = inst.ctx();
    let wit = ToolCtx {
        session_id: session_id.to_string(),
        parent_session_id: parent_session_id.map(|s| s.to_string()),
        instance_id: inst_ctx.instance_id.to_string(),
        plugin_name: inst_ctx.plugin_name.clone(),
    };

    tracing::debug!(%name, %args, "dispatch_run_tool: calling component run-tool");
    inst.store_mut()
        .run_concurrent(async |a| guest.call_run_tool(a, name, args, wit).await)
        .await?
}

// ─── Sync render-hook dispatch ──────────────────────────────────────────
// The render thread calls hooks synchronously through PluginSyncHooks.
// These still arrive as JSON and return typed results converted to JSON.

pub fn dispatch_sync_hook(
    inst: &mut crate::store::StoredInstance,
    hook: &str,
    ctx_json: &Value,
) -> wasmtime::Result<Option<Value>> {
    let guest = inst.typed_guest()?;
    let inst_ctx = inst.ctx();
    let plugin_name = inst_ctx.plugin_name.clone();
    let instance_id = inst_ctx.instance_id.to_string();
    match hook {
        "on_chat_input_badges_render" => {
            let mut ctx = BadgeCtx {
                session_id: get_str(ctx_json, "session_id"),
                active_session_id: get_str(ctx_json, "active_session_id"),
                instance_id: instance_id.clone(),
                plugin_name: plugin_name.clone(),
                mode: get_str(ctx_json, "mode"),
                theme_styles: theme_styles_list(ctx_json),
            };
            ctx.plugin_name = plugin_name.clone();
            ctx.instance_id = instance_id.clone();
            guest
                .call_on_chat_input_badges_render(inst.store_mut(), &ctx)?
                .map(badge_directive_to_json)
                .map(Ok)
                .transpose()
        }
        "on_keybind_trigger" => {
            let mut ctx = KeybindTriggerCtx {
                session_id: get_str(ctx_json, "session_id"),
                instance_id: instance_id.clone(),
                plugin_name: plugin_name.clone(),
                hook: get_str(ctx_json, "hook"),
                text: get_str(ctx_json, "text"),
                keybound_plugin: get_str(ctx_json, "keybound_plugin"),
            };
            ctx.plugin_name = plugin_name.clone();
            ctx.instance_id = instance_id.clone();
            guest
                .call_on_keybind_trigger(inst.store_mut(), &ctx)?
                .map(keybind_result_to_json)
                .map(Ok)
                .transpose()
        }
        "on_submit_intercept" => {
            let mut ctx = SubmitInterceptCtx {
                session_id: get_str(ctx_json, "session_id"),
                instance_id: instance_id.clone(),
                plugin_name: plugin_name.clone(),
                input_text: get_str(ctx_json, "input_text"),
            };
            ctx.plugin_name = plugin_name.clone();
            ctx.instance_id = instance_id.clone();
            guest
                .call_on_submit_intercept(inst.store_mut(), &ctx)?
                .map(intercept_outcome_to_json)
                .map(Ok)
                .transpose()
        }
        "on_session_preview" => {
            let mut ctx = SessionPreviewCtx {
                session_id: get_str(ctx_json, "session_id"),
                instance_id: instance_id.clone(),
                plugin_name: plugin_name.clone(),
            };
            ctx.plugin_name = plugin_name.clone();
            ctx.instance_id = instance_id.clone();
            guest
                .call_on_session_preview(inst.store_mut(), &ctx)?
                .map(|s| serde_json::json!({ "session_id": s }))
                .map(Ok)
                .transpose()
        }
        _ => Ok(None),
    }
}
