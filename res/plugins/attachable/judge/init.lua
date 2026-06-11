-- description: Judge plugin — evaluates assistant responses after each turn using a child LLM session
--
-- Spawns an automated child session with `session_query` access. The child LLM
-- reviews the origin session's last assistant response and calls either
-- `judgment_passed()` or `judgment_failed(message)` to route results back.
--
-- The child session is reset before each evaluation (clean slate every turn).
-- The plugin stores `judge_session_id` and `origin_session_id` in plugin_data
-- so tool handlers and the sync preview hook can find the right sessions.

local M = {}

local SYSTEM_PROMPT = [[
You are a response quality judge. You have access to a `session_query` tool
that lets you inspect the origin session's conversation history.

The origin session UUID is: %s

When calling session_query, pass this exact UUID as the session_id parameter.

After reviewing the last assistant response, call exactly one of:
  - `judgment_passed()` if the response is satisfactory
  - `judgment_failed(message)` if the response has problems, explaining what went wrong

Be thorough. Check for accuracy, completeness, and relevance.
Use `session_query` with action "get_recent" to inspect the conversation.
]]

-- ─── Plugin-defined tools ──────────────────────────────────────────

M.tools = {
    {
        name = "judgment_passed",
        description = "Call when the assistant's response passes evaluation.",
        parameters = {},
        handler = function(ctx)
            local pd = ctx.get_plugin_data() or {}
            ctx.emit("push_chat_entry", {
                session_id = pd.origin_session_id,
                kind = { transient = "✓ Judgment passed" },
            })
            ctx.emit("disable_plugin", {
                session_id = pd.origin_session_id,
                plugin_name = ctx.plugin_name,
            })
        end,
    },
    {
        name = "judgment_failed",
        description = "Call when the assistant's response fails evaluation.",
        parameters = {
            { name = "message", type = "string", description = "Why the response failed" },
        },
        handler = function(ctx, args)
            local pd = ctx.get_plugin_data() or {}
            ctx.emit("enqueue_user_message", {
                session_id = pd.origin_session_id,
                text = "✗ Judgment failed: " .. tostring(args.message),
            })
        end,
    },
}

-- ─── Hooks ─────────────────────────────────────────────────────────

---@param ctx OnTurnEndCtx
function M.on_turn_end(ctx)
    -- Push a transient status indicator to the origin session.
    ctx.emit("push_chat_entry", {
        session_id = ctx.session_id,
        kind = { transient = "⚖ Judge evaluating..." },
    })

    pd = ctx.get_plugin_data() or {}
    local judge_id = pd.judge_session_id

    if not judge_id then
        -- First time: create the child session.
        local result = ctx.request("create_session", {
            parent_session_id = ctx.session_id,
            automated = true,
            persist = true,
        })
        if not result.ok then
            ctx.emit("push_chat_entry", {
                session_id = ctx.session_id,
                kind = { error = "judge: failed to create session: " .. tostring(result.error) },
            })
            return
        end
        judge_id = result.value.session_id
        ctx.merge_plugin_data({
            judge_session_id = judge_id,
            origin_session_id = ctx.session_id,
        })
    else
        -- Subsequent turns: reset the existing session.
        ctx.emit("reset_session", {
            session_id = judge_id,
        })
    end

    -- Inject judge system prompt with the origin session ID.
    ctx.emit("push_chat_entry", {
        session_id = judge_id,
        kind = { system = SYSTEM_PROMPT:format(ctx.session_id) },
    })

    -- Ask the judge to evaluate the latest response.
    ctx.emit("enqueue_user_message", {
        session_id = judge_id,
        text = "Evaluate the last assistant response in the origin session.",
    })

    -- Return immediately. The child LLM will run, call a judgment tool,
    -- and the tool handler will push results back to the origin session.
end

--- Sync hook: returns the judge session ID for sidebar preview.
--- Reads from plugin_data (written by the async on_turn_end hook).
function M.on_session_preview(ctx)
    local pd = ctx.plugin_data or {}
    if pd.judge_session_id then
        return { session_id = pd.judge_session_id }
    end
    return nil
end

return M
