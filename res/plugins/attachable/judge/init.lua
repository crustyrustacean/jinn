-- description: Judge plugin — evaluates assistant responses after each turn using a child LLM session
--
-- Spawns an automated child session with `session_query` access. The child LLM
-- reviews the origin session's last assistant response and calls either
-- `judgment_passed()` or `judgment_failed(message)` to route results back.
--
-- A FRESH child session is created every turn (persist=false, no history
-- reuse). Tool handlers derive the origin from `ctx.parent_session_id`
-- (the child→origin parent edge).

local M = {}


-- ─── Plugin-defined tools ──────────────────────────────────────────

M.tools = {
	{
		name = "judgment_passed",
		description = "Call when the assistant's response passes evaluation and then STOP.",
		scope = "attached",
		parameters = {},
		handler = function(ctx)
			local origin = ctx.parent_session_id
			ctx.emit("push_chat_entry", {
				session_id = origin,
				kind = { transient = "✓ Judgment passed" },
			})
			ctx.emit("disable_plugin", {
				session_id = origin,
				plugin_name = ctx.plugin_name,
			})
		end,
	},
	{
		name = "judgment_failed",
		description = "Call when the assistant's response fails evaluation and then STOP.",
		scope = "attached",
		parameters = {
			{ name = "message", type = "string", description = "Why the response failed" },
		},
		handler = function(ctx, args)
			local origin = ctx.parent_session_id
			ctx.emit("enqueue_user_message", {
				session_id = origin,
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

	-- Every turn: create a fresh transient child session. No reuse, no reset —
	-- the judge starts from a clean slate so it cannot reference prior judgments.
	local result = ctx.request("create_session", {
		parent_session_id = ctx.session_id,
		automated = true,
		persist = false,
		inherit_tools = false,
		tools = { "judgment_passed", "judgment_failed", "session_query" },
	})
	if not result.ok then
		ctx.emit("push_chat_entry", {
			session_id = ctx.session_id,
			kind = { error = "judge: failed to create session: " .. tostring(result.error) },
		})
		return
	end
	local judge_id = result.value.session_id

	-- Tell the domain layer about the managed session so the sidebar can
	-- navigate to it (and reflect its busy state).
	ctx.emit("set_managed_session", {
		session_id = ctx.session_id,
		plugin_name = ctx.plugin_name,
		managed_session_id = judge_id,
	})

	-- Ask the judge to evaluate the latest response.
	ctx.emit("enqueue_user_message", {
		session_id = judge_id,
		text = string.format(
			[[You are a response quality judge. The origin session UUID is: %s

Use session_query to inspect the origin session's conversation history.
When calling session_query, pass this exact UUID as the session_id parameter.

After reviewing the last assistant response, call exactly one of:
  - judgment_passed() if the response is satisfactory
  - judgment_failed(message) if the response has problems, explaining what went wrong

Be thorough. Check for accuracy, completeness, and relevance. After issuing a judgment tool call, STOP.]],
			ctx.session_id
		),
	})

	-- Return immediately. The child LLM will run, call a judgment tool,
	-- and the tool handler will push results back to the origin session.
end

return M
