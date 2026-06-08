-- description: Prompt enrichment — rewrite the draft via an LLM one-shot on tap
-- Prompt enrichment plugin for jinn.
--
-- Declares <M-e> to run an LLM one-shot enrichment on the current draft
-- immediately: the draft is sent to an async hook that rewrites it via an
-- LLM one-shot, then writes the enriched text back into the input box. A
-- badge ("[Enrich]") is always drawn as a hotkey legend.

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
        action = "on_enrich",
        description = "enrich prompt",
    },
}

--- Normalize the plugin_data table, ensuring the expected fields exist.
--- plugin_data is a full-replace store, so we always read the current value
--- (defaulting to a fresh table) and write back a complete object.
---
---@param current any The current plugin_data (may be nil on first call).
---@return table data A table with status (string).
local function normalize(current)
    local d = current or {}
    if type(d) ~= "table" then
        d = {}
    end
    if type(d.status) ~= "string" then
        d.status = "idle"
    end
    return d
end

--- Async hook fired via Intent::TriggerPlugin when <M-e> is pressed, and via the
--- generic fire_async_hook handoff. Runs an LLM one-shot (history-less,
--- inheriting the session's provider+model), then writes the enriched text
--- back into the chat input.
---
--- All work is wrapped in pcall so an error degrades to clearing the spinner
--- without crashing anything.
---
---@param ctx PluginCtx
function M.on_enrich(ctx)
    local ok, err = pcall(function()
        -- Empty draft: nothing to enrich. Bail before any LLM call.
        if type(ctx.text) ~= "string" or ctx.text == "" then
            return
        end

        local d = normalize(ctx.get_plugin_data())

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

        -- Read current plugin_data for the post-call status update.
        local cur = normalize(ctx.get_plugin_data())

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
        local cur = normalize(ctx.get_plugin_data())
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
--- directive for the consistent chat-input badge location, drawn as a
--- persistent hotkey legend.
---
--- The `E` is the bound key. It is colored with the hotkey accent only while
--- the user is in Input mode — the only mode in which <M-e> is actionable.
--- Outside Input mode it dims to the muted text color alongside the brackets.
--- This mode→style decision lives in the plugin; the host never applies
--- mode-aware styling to plugin content.
---
---@param ctx OnChatInputBadgesRenderCtx
---@return table? directive `{ slot, segments = { { text, style } } }` or nil
function M.on_chat_input_badges_render(ctx)
    local e_style = (ctx.mode == "input") and "accent_action" or "muted_text"
    return {
        slot = "input_badge",
        segments = {
            { text = "[",      style = "muted_text" },
            { text = "E",      style = e_style },
            { text = "nrich]", style = "muted_text" },
        },
    }
end

return M
