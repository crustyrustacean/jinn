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

---
---@class OnSubmitInterceptCtx : PluginCtx
---Sync hook fired by `IntentHandler::handle` after a **submit-family** intent
---(`Intent::SubmitMessage`) resolves but before its commands are dispatched.
---Plugins can block or replace the commands. Return `{ action = "block" }` to
---drop the commands, or `{ action = "pass" }` for pass-through. (A third
---outcome, `{ action = "replace", commands = {...} }`, swaps in new commands.)
---Runs on the render-thread (sync) Lua state — cannot call `ctx.request`.
---
---@field input_text string The current chat-input draft at submit time.

---
---@class OnChatInputBadgesRenderCtx : PluginCtx
---Sync hook fired by the chat-input renderer each frame. A plugin returns a
---single badge directive (`{ slot, text, style? }`) drawn into the consistent
---chat-input badge location, or `nil` when it has nothing to draw. Runs on the
---render-thread (sync) Lua state.
---
---@field active_session_id string The session currently in focus.

-- ─── Verb payload shapes ───────────────────────────────────────────

---@alias PluginVerb
---| '"push_chat_entry"'
---| '"enqueue_user_message"'
---| '"disable_plugin"'
---| '"fire_async_hook"'
---| '"set_chat_input"'

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

---@class FireAsyncHookPayload
---Generic handoff from the sync VM to the async VM. The plugin actor resolves
---the `session_id`, then fires the named async `hook` with this payload as its
---ctx. Enables sync hooks to kick off coroutine-capable work (e.g. LLM calls).
---
---@field hook string The async hook name to fire (e.g. `"on_enrich"`).
---@field session_id string The session scope for the async hook.
---@field text? string Optional payload forwarded into the async hook ctx.

---@class SetChatInputPayload
---Replaces the chat input box text for the given session.
---
---@field session_id string
---@field text string The replacement text.

--- ─── ctx.request contracts ───────────────────────────────────────
---
---`ctx.request(name, data)` is a blocking coroutine call resolved by a oneshot.
---Named contracts:
---
---@alias PluginRequestName
---| '"llm_oneshot"'

---@class LlmOneshotRequest
---History-less one-shot LLM call. Inherits the session's provider+model
---configuration; sends NO chat history. Used for transformations like prompt
---enrichment that don't need conversational context.
---
---@field session_id string The session whose provider+model to inherit.
---@field system string The system prompt.
---@field prompt string The user prompt (e.g. the draft to enrich).
---@field persist boolean|nil If true, the one-shot session is written to the
---  SQLite store; defaults to false (transient). Set true for reviewable
---  runs (e.g. a judge/eval).
---@field disable_tool_loop boolean|nil If true, the one-shot session runs with no
---  declared tools and tool_loop_disabled set, so the model cannot enter a
---  tool loop. If false (default), the one-shot inherits the full tool
---  catalog the session would normally see.
---@field timeout_ms number|nil Hard upper bound on the await, in milliseconds.
---  Defaults to 30000. On expiry, the one-shot session is hard-cancelled
---  (no background token burn) and the request returns an error to the hook.
---  SQLite store; defaults to false (transient). Set true for reviewable
---  runs (e.g. a judge/eval).
---@class LlmOneshotResponse
---@field text string The model's response text.
