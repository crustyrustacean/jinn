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
--- Wrapped in pcall so an error degrades to an error chat entry instead
--- of aborting the whole fire (run_hooks_fire propagates a raised error, skipping
--- any plugins scheduled after this one).
---
--- @param ctx PluginCtx
function M.on_enrich(ctx)
    local ok = pcall(function()
        if ctx.text == "" then
            return
        end

        ctx.merge_plugin_data({ status = "enriching" })

        local result = ctx.request("llm_oneshot", {
            session_id = ctx.session_id,
            system = ENRICH_PROMPT,
            prompt = ctx.text,
            persist = false,       -- one-shot enrichment is transient; never write to the store
            disable_tool_loop = true, -- enrichment is a pure text rewrite; never run tool loops
            timeout_ms = 30000,    -- bound a genuinely stuck model; hard-cancels the one-shot session
        }, {
            -- Named task so on_keybind_trigger can cancel this in-flight
            -- request on a retap. Scoped per-session via string concat (the
            -- enrichment plugin is global but operates per-session).
            task = "enrich:" .. ctx.session_id,
        })

        if not result.ok then
            -- A cancel is intentional (retap); suppress the error entry.
            -- Other errors surface so the user knows enrichment failed.
            if result.error ~= "cancelled" then
                ctx.emit("push_chat_entry", {
                    session_id = ctx.session_id,
                    kind = { error = result.error },
                })
            end
        elseif result.value.text ~= "" then
            ctx.emit("set_chat_input", {
                session_id = ctx.session_id,
                text = result.value.text,
            })
        end

        ctx.merge_plugin_data({ status = "idle" })
    end)

    if not ok then
        -- Restore idle and surface the failure without crashing the fire.
        ctx.merge_plugin_data({ status = "idle" })
        ctx.emit("push_chat_entry", {
            session_id = ctx.session_id,
            kind = { error = "enrichment failed" },
        })
    end
end


--- Sync hook fired by Intent::TriggerPlugin before the async on_enrich fire.
--- Lets the plugin veto the fire and cancel an in-flight enrichment instead.
---
--- Self-selects on ctx.plugin_name: only this plugin's keybind is answered.
--- Returns {fire=false} when enriching (cancels the in-flight request);
--- {fire=true} otherwise (proceed to fire on_enrich).
---
---@param ctx OnKeybindTriggerCtx
---@return table? result `{ fire = bool }` or nil
function M.on_keybind_trigger(ctx)
    -- Only answer for our own keybind.
    if ctx.plugin_name ~= "prompt_enrichment" then return end

    if (ctx.plugin_data or {}).status == "enriching" then
        ctx.cancel("enrich:" .. ctx.session_id)
        return { fire = false }
    end
    return { fire = true }
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
    local pd = ctx.plugin_data or {}
    if pd.status == "enriching" then
        return {
            slot = "input_badge",
            segments = {
                { text = "[",       style = "muted_text" },
                { text = "Working", style = "streaming" },
                { text = "]",       style = "muted_text" },
            },
        }
    end

    local e_style = (ctx.mode == "input") and "accent_action" or "muted_text"
    return {
        slot = "input_badge",
        segments = {
            { text = "[",      style = "muted_text" },
            { text = "E",      style = e_style },
            { text = "nrich", style = "muted_text" },
            { text = "]",      style = "muted_text" },
        },
    }
end

return M
