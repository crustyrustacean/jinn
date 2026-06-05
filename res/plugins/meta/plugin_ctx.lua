---Type annotations for plugin authors.
---Not loaded by the runtime; consumed by lua-language-server for autocomplete
---and diagnostics.
---
---To get editor support, install lua-language-server (LuaLS) and add this
---file's directory to your workspace's `workspace.library` setting, e.g. in
---`.luarc.json` at the project root:
---
---    {
---        "workspace.library": ["./res/plugins/meta"]
---    }
---
---Plugin authors then annotate their hook functions with the matching ctx
---class to get field-aware autocomplete and diagnostics.

-- ─── Base ctx (shared by all hooks) ────────────────────────────────

---@class PluginCtx
---@field session_id string The current session ID.
---@field plugin_name string Name of this plugin (for self-targeting actions like `disable_plugin`).
---@field plugin_data any This plugin's persistent state.
---@field emit fun(verb: PluginVerb, data: table) Fire-and-forget emit a domain command.
---@field request fun(name: string, data: table): any Blocking request to a named handler.
---@field set_plugin_data fun(data: any) Replace this plugin's persistent state.

-- ─── Per-hook ctx classes ──────────────────────────────────────────
--
-- All hook ctx classes inherit the fields above. Hook-specific extra fields
-- are declared on the subclass. If a hook has no extra fields today, its
-- class is empty (still useful for editor feedback when a plugin author
-- mistypes a hook name).

---@class OnAppStartedCtx : PluginCtx
---Fires once at app startup. No hook-specific fields.

---@class OnSessionCreatedCtx : PluginCtx
---Fires when a new session is created. No hook-specific fields.

---@class OnTurnEndCtx : PluginCtx
---Fires when a session transitions to Idle (turn complete).
---Future: may expose `last_user_message` and `last_assistant_message`.

---@class OnUserSubmitCtx : PluginCtx
---Fires when the user submits a message (session transitions to Sending).
---Future: may expose `text` (the submitted text).

-- ─── Verb payload shapes ───────────────────────────────────────────

---@alias PluginVerb
---| '"push_chat_entry"'
---| '"enqueue_user_message"'
---| '"disable_plugin"'

---@class PushChatEntryPayload
---@field session_id string
---@field kind ChatEntryKind

---@class ChatEntryKind
---@field system? string
---@field transient? string

---@class EnqueueUserMessagePayload
---@field session_id string
---@field text string

---@class DisablePluginPayload
---@field session_id string
---@field plugin_name string
