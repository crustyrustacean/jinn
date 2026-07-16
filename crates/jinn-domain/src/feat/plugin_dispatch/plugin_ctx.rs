//! Domain-owned typed hook contexts and plugin commands.
//!
//! These are the single source of truth for the shapes that cross the
//! `PluginFire` / `PluginSyncHooks` trait seams. The domain builds these
//! records directly; `jinn-wasm-host` maps them to/from the generated WIT
//! types at its boundary (the mapping is structural and bidirectional).
//!
//! Every record here mirrors a `record` in `wit/jinn.wit`, and `PluginCommand`
//! mirrors the `command` variant — the two must stay in sync (the propagation
//! table in `AGENTS.md` governs this).
//!
//! # Why not use the generated WIT types directly?
//!
//! The domain crate cannot depend on `jinn-wasm-host` (circular dependency),
//! and the WIT-generated types live there. These domain records decouple the
//! seam from the runtime: a future non-WASM backend would implement the same
//! traits against the same records without touching the WIT layer.

use crate::protocol::SessionId;

// ─── Hook contexts ──────────────────────────────────────────────────────

/// Context for session-scoped lifecycle hooks (`on_app_started`,
/// `on_session_created`, `on_user_submit`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHookCtx {
    /// The session the hook fires for.
    pub session_id: SessionId,
    /// Parent session id, if this is a child session. `None` for top-level.
    pub parent_session_id: Option<SessionId>,
    /// This plugin instance's stable id.
    pub instance_id: String,
    /// This plugin's name.
    pub plugin_name: String,
}

/// Context for `on_turn_end`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnEndHookCtx {
    pub session_id: SessionId,
    pub parent_session_id: Option<SessionId>,
    pub instance_id: String,
    pub plugin_name: String,
    pub turn: u32,
}

/// Context for `on_attach` / `on_detach`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachHookCtx {
    pub session_id: SessionId,
    pub instance_id: String,
    pub plugin_name: String,
}

/// Context for `on_task_list_updated`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskListHookCtx {
    pub session_id: SessionId,
    pub instance_id: String,
    pub plugin_name: String,
    /// The task list rendered as text (phases + tasks + blockers).
    pub task_list: String,
    pub completed: u32,
    pub total: u32,
    /// True only when the list is non-empty AND no phase has pending work.
    pub is_complete: bool,
}

/// Context for a plugin-defined trigger hook (`run_trigger`), fired by the
/// keybind's `action` string (e.g. `on_enrich`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerHookCtx {
    pub session_id: SessionId,
    pub parent_session_id: Option<SessionId>,
    pub instance_id: String,
    pub plugin_name: String,
    /// The current chat-input draft at trigger time.
    pub text: String,
}

/// Context for a plugin-defined tool handler (`run_tool`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolHookCtx {
    pub session_id: SessionId,
    pub parent_session_id: Option<SessionId>,
    pub instance_id: String,
    pub plugin_name: String,
}

/// Context for `on_chat_input_badges_render`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadgeHookCtx {
    pub session_id: SessionId,
    /// The session currently in focus (preview target).
    pub active_session_id: SessionId,
    pub instance_id: String,
    pub plugin_name: String,
    /// Current scope mode as a lowercase string (`"input"`, `"normal"`, …).
    pub mode: String,
    /// Theme style names available for segment styling.
    pub theme_styles: Vec<String>,
}

/// Context for `on_keybind_trigger`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeybindTriggerHookCtx {
    pub session_id: SessionId,
    pub instance_id: String,
    pub plugin_name: String,
    /// The hook name to fire (from the keybind's `action`).
    pub hook: String,
    /// The current chat-input draft at trigger time.
    pub text: String,
    /// The plugin whose keybind was pressed (self-select hint).
    pub keybound_plugin: String,
}

/// Context for `on_submit_intercept`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitInterceptHookCtx {
    pub session_id: SessionId,
    pub instance_id: String,
    pub plugin_name: String,
    /// The current chat-input draft at submit time.
    pub input_text: String,
}

/// Context for `on_session_preview`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPreviewHookCtx {
    pub session_id: SessionId,
    pub instance_id: String,
    pub plugin_name: String,
}

/// Discriminated union of all async hook contexts.
///
/// The hook *name* is carried separately by the fire calls (because
/// plugin-defined triggers like `on_enrich` are runtime-resolved by string),
/// but the *payload shape* is one of these variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookCtx {
    Session(SessionHookCtx),
    TurnEnd(TurnEndHookCtx),
    Attach(AttachHookCtx),
    TaskList(TaskListHookCtx),
    Trigger(TriggerHookCtx),
    Tool(ToolHookCtx),
}

impl HookCtx {
    /// Inject the parent session edge for hooks that carry one
    /// (`Session`, `TurnEnd`, `Trigger`). Hooks without a parent field are
    /// left untouched.
    pub fn set_parent_session_id(&mut self, parent: SessionId) {
        match self {
            Self::Session(c) => c.parent_session_id = Some(parent),
            Self::TurnEnd(c) => c.parent_session_id = Some(parent),
            Self::Trigger(c) => c.parent_session_id = Some(parent),
            Self::Attach(_) | Self::TaskList(_) | Self::Tool(_) => {}
        }
    }
}
// ─── Plugin commands (the `emit` surface) ───────────────────────────────

/// A command a plugin emits back to the host via `ctx.emit`.
///
/// Mirrors the `command` variant in `wit/jinn.wit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginCommand {
    PushChatEntry(PushChatEntryCmd),
    EnqueueUserMessage(EnqueueUserMessageCmd),
    SetChatInput(SetChatInputCmd),
    SetChatInputEnabled(SetChatInputEnabledCmd),
    DisablePlugin(DisablePluginCmd),
    EnablePlugin(EnablePluginCmd),
    SetManagedSession(SetManagedSessionCmd),
    ResetSession(ResetSessionCmd),
    FireAsyncHook(FireAsyncHookCmd),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushChatEntryCmd {
    pub session_id: SessionId,
    pub kind: PushEntryKind,
}

/// Kind of chat entry a plugin pushes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushEntryKind {
    /// A system-generated message (status updates, greetings).
    System(String),
    /// A transient indicator (auto-dismissed, not sent to the LLM).
    Transient(String),
    /// An error message displayed prominently.
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnqueueUserMessageCmd {
    pub session_id: SessionId,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetChatInputCmd {
    pub session_id: SessionId,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetChatInputEnabledCmd {
    pub session_id: SessionId,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisablePluginCmd {
    pub session_id: SessionId,
    pub plugin_name: String,
    pub instance_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnablePluginCmd {
    pub session_id: SessionId,
    pub plugin_name: String,
    pub instance_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetManagedSessionCmd {
    pub session_id: SessionId,
    pub plugin_name: String,
    pub instance_id: String,
    pub managed_session_id: SessionId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetSessionCmd {
    pub session_id: SessionId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FireAsyncHookCmd {
    pub session_id: SessionId,
    pub hook: String,
    pub text: String,
}
