-- description: Prompt enrichment — rewrite the draft via an LLM one-shot on submit
-- Prompt enrichment plugin for jinn.
--
-- Declares <M-e> to toggle an "enrich" flag. When the flag is on, submitting the
-- chat input is intercepted (blocked) and the draft is sent to an async hook
-- that runs an LLM one-shot, then writes the enriched text back into the input
-- box. A badge ("E") shows the armed state and a spinner ("✨") shows in-flight
-- enrichment.

local M = {}

--- System prompt for the enrichment LLM call. Hard-coded per the plan.
local ENRICH_PROMPT = [[
Rewrite the following user input into a clearer, more detailed prompt.
Preserve the user's intent and add specificity where helpful.
Return ONLY the rewritten prompt, with no preamble or explanation.]]

--- Plugin keybind declaration. Consumed at load time by bind_plugin_keybinds.
--- `action` names the async hook to fire; `description` feeds the which-key help popup.
M.keybinds = {
    {
        scope = "Input",
        keys = "<M-e>",
        action = "on_toggle_enrich",
        description = "toggle prompt enrichment",
    },
}

--- Normalize the plugin_data table, ensuring the expected fields exist.
--- plugin_data is a full-replace store, so we always read the current value
--- (defaulting to a fresh table) and write back a complete object.
---
---@param current any The current plugin_data (may be nil on first call).
---@return table data A table with enrich_enabled (bool) and status (string).
local function normalize(current)
    local d = current or {}
    if type(d) ~= "table" then
        d = {}
    end
    if type(d.enrich_enabled) ~= "boolean" then
        d.enrich_enabled = false
    end
    if type(d.status) ~= "string" then
        d.status = "idle"
    end
    return d
end

--- Async hook fired by Intent::TriggerPlugin when <M-e> is pressed.
--- Toggles the enrich_enabled flag in plugin_data. Because this runs on the
--- async VM and on_render / on_submit_intercept run on the sync VM, the
--- updated value is visible to both via the shared plugin_data store.
---
---@param ctx PluginCtx
function M.on_toggle_enrich(ctx)
    local d = normalize(ctx.plugin_data)
    d.enrich_enabled = not d.enrich_enabled
    if not d.enrich_enabled then
        d.status = "idle"
    end
    ctx.set_plugin_data(d)
end

--- Sync hook fired by IntentHandler::handle before the submit intent's commands
--- are dispatched. When the flag is on, emit the async handoff and block.
---
--- Returns `{ action = "block" }` to drop the submit commands, or
--- `{ action = "pass" }` for normal submit.
---
--- Runs on the render-thread Lua state. Cannot call ctx.request.
---
---@param ctx OnSubmitInterceptCtx
---@return table outcome `{ action = "block" | "pass" }`
function M.on_submit_intercept(ctx)
    local d = normalize(ctx.plugin_data)
    if d.enrich_enabled then
        ctx.emit("fire_async_hook", {
            hook = "on_enrich",
            session_id = ctx.session_id,
            text = ctx.input_text,
        })
        return { action = "block" }
    end
    return { action = "pass" }
end

--- Async hook fired via the generic fire_async_hook handoff. Runs an LLM
--- one-shot (history-less, inheriting the session's provider+model), then
--- writes the enriched text back into the chat input.
---
--- A generation counter guards against stale overwrites: if the user re-triggers
--- enrichment (or toggles the flag off) while this call is in-flight, the
--- result is dropped. All work is wrapped in pcall so an error degrades to
--- clearing the spinner without crashing anything.
---
---@param ctx PluginCtx
function M.on_enrich(ctx)
    local ok, err = pcall(function()
        local d = normalize(ctx.plugin_data)
        if not d.enrich_enabled then
            -- Flag was toggled off before the async handoff landed. Bail.
            return
        end

        -- Capture the generation so we can detect supersession mid-flight.
        local gen = (d.generation or 0) + 1
        d.generation = gen
        d.status = "enriching"
        ctx.set_plugin_data(d)

        local result = ctx.request("llm_oneshot", {
            session_id = ctx.session_id,
            system = ENRICH_PROMPT,
            prompt = ctx.text,
            persist = false, -- one-shot enrichment is transient; never write to the store
            disable_tool_loop = true, -- enrichment is a pure text rewrite; never run tool loops
            timeout_ms = 30000, -- bound a genuinely stuck model; hard-cancels the one-shot session
        })

        -- Re-read plugin_data in case it changed during the await (re-trigger).
        local cur = normalize(ctx.plugin_data)
        if not cur.enrich_enabled or cur.generation ~= gen then
            -- Superseded or disabled: drop the stale result.
            cur.status = "idle"
            ctx.set_plugin_data(cur)
            return
        end

        if result and type(result.text) == "string" and result.text ~= "" then
            ctx.emit("set_chat_input", {
                session_id = ctx.session_id,
                text = result.text,
            })
        end

        cur.status = "idle"
        ctx.set_plugin_data(cur)
    end)

    if not ok then
        -- Never leave the spinner stuck on. Always restore idle on any error.
        local cur = normalize(ctx.plugin_data)
        cur.status = "idle"
        ctx.set_plugin_data(cur)
        -- Surface the error for debugging without crashing the actor.
        if ctx.emit then
            ctx.emit("push_chat_entry", {
                session_id = ctx.session_id,
                kind = { transient = "enrichment failed" },
            })
        end
    end
end

--- Sync hook fired by the chat-input renderer. Returns a single badge
--- directive for the consistent chat-input badge location, or nil when
--- the enrichment toggle is disarmed (no badge to draw).
---
---@param ctx OnChatInputBadgesRenderCtx
---@return table? directive `{ slot, text, style? }` or nil
function M.on_chat_input_badges_render(ctx)
    local d = normalize(ctx.plugin_data)
    if d.enrich_enabled then
        return { slot = "input_badge", text = "E", style = "yellow" }
    end
    return nil
end

return M
