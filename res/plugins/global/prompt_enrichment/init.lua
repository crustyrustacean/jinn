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

--- Async hook fired via Intent::TriggerPlugin when <M-e> is pressed.
--- Runs an LLM one-shot on the current draft, then writes the enriched text
--- back into the chat input.
---
--- Wrapped in pcall so an error degrades to a transient chat entry instead
--- of aborting the whole fire (run_hooks_fire propagates a raised error, skipping
--- any plugins scheduled after this one).
---
--- @param ctx PluginCtx
function M.on_enrich(ctx)
    local ok = pcall(function()
        if ctx.text == "" then
            return
        end

        local d = ctx.get_plugin_data()
        d.status = "enriching"
        ctx.set_plugin_data(d)

        local result = ctx.request("llm_oneshot", {
            session_id = ctx.session_id,
            system = ENRICH_PROMPT,
            prompt = ctx.text,
            persist = false,       -- one-shot enrichment is transient; never write to the store
            disable_tool_loop = true, -- enrichment is a pure text rewrite; never run tool loops
            timeout_ms = 30000,    -- bound a genuinely stuck model; hard-cancels the one-shot session
        })

        if type(result) == "table" and type(result.text) == "string" and result.text ~= "" then
            ctx.emit("set_chat_input", {
                session_id = ctx.session_id,
                text = result.text,
            })
        end

        local cur = ctx.get_plugin_data()
        cur.status = "idle"
        ctx.set_plugin_data(cur)
    end)

    if not ok then
        -- Restore idle and surface the failure without crashing the fire.
        local cur = ctx.get_plugin_data()
        cur.status = "idle"
        ctx.set_plugin_data(cur)
        ctx.emit("push_chat_entry", {
            session_id = ctx.session_id,
            kind = { transient = "enrichment failed" },
        })
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
