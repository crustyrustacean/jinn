-- description: Judge plugin — evaluates assistant responses after each turn using a child LLM session
--
-- Spawns an automated child session with `session_query` access. The child LLM
-- reviews the origin session's last assistant response and calls either
-- `judgment_passed()` or `judgment_failed(message)` to route results back.
--
-- A FRESH child session is created every turn (persist=false, no history
-- reuse). Tool handlers derive the origin from `ctx.parent_session_id`
-- (the child→origin parent edge) and key their verdict on `ctx.session_id`
-- (the child's own id, unique per verdict).
--
-- Multi-instance (panel/aggregation):
--   Multiple judge instances may be attached to one origin. Each runs its own
--   child in parallel. They coordinate via a shared global-data bag, keyed by
--   origin. The LAST judge to complete aggregates all verdicts and emits ONE
--   merged result to the origin, then disables itself. All-must-finish: if one
--   judge's child errors or hangs, the merged result never fires (no timeout —
--   accepted trade-off).
--
--   We key verdicts by the CHILD session id (ctx.session_id), not the judge
--   instance id. Each child is unique, so N judges → N children → N verdicts
--   with no collisions, and the tool handler never needs to know which parent
--   instance spawned it.
--
--   Shared keys (namespaced by origin):
--     judge:<origin>:count      — number of participating instances (attach/detach)
--     judge:<origin>:verdicts   — { [child_session_id] = { verdict, message } }
--     judge:<origin>:completed  — count of verdicts posted this turn
--     judge:<origin>:turn       — turn counter; bumped each turn so the first
--                                 fire resets verdicts/completed exactly once
--
-- Concurrency note: this read-modify-write pattern on the global bag is safe
-- because hooks + tool callbacks serialize through the single plugin thread.
-- The child LLM turns run concurrently but never touch these keys.

local M = {}

-- ─── Shared-key helpers (namespaced by origin) ──────────────────────────

local function count_key(origin)
	return "judge:" .. origin .. ":count"
end

local function verdicts_key(origin)
	return "judge:" .. origin .. ":verdicts"
end

local function completed_key(origin)
	return "judge:" .. origin .. ":completed"
end

local function turn_key(origin)
	return "judge:" .. origin .. ":turn"
end

local function instances_key(origin)
	-- Set of participating instance ids: { [instance_id] = true }.
	-- Used by the aggregator to disable/enable ALL instances on pass/fail.
	return "judge:" .. origin .. ":instances"
end

-- ─── Plugin-defined tools ──────────────────────────────────────────────

M.tools = {
	{
		name = "judgment_passed",
		description = "Call when the assistant's response passes evaluation and then STOP.",
		scope = "attached",
		parameters = {},
		handler = function(ctx)
			record_verdict(ctx, "passed", nil)
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
			record_verdict(ctx, "failed", tostring(args.message))
		end,
	},
}

-- ─── Verdict posting + aggregation (called from tool handlers) ──────────
--
-- Posts this child's verdict, increments the completed count, and — when
-- all participants have posted — merges everything and emits ONE result.

function record_verdict(ctx, verdict, message)
	local origin = ctx.parent_session_id
	local me = ctx.session_id -- the child session; unique per verdict
	local count = ctx.get_global_data(count_key(origin)) or 0

	-- Post this child's verdict.
	local verdicts = ctx.get_global_data(verdicts_key(origin)) or {}
	verdicts[me] = { verdict = verdict, message = message }
	ctx.set_global_data(verdicts_key(origin), verdicts)

	-- Increment completed.
	local completed = (ctx.get_global_data(completed_key(origin)) or 0) + 1
	ctx.set_global_data(completed_key(origin), completed)

	-- Only the last to finish aggregates + emits.
	if completed < count then
		return
	end

	-- Majority vote: count pass vs fail. Strict majority pass wins;
	-- a tie or fail-majority means the response failed.
	local pass_count = 0
	local fail_count = 0
	local fail_parts = {}
	for _id, v in pairs(verdicts) do
		if v.verdict == "passed" then
			pass_count = pass_count + 1
		elseif v.verdict == "failed" then
			fail_count = fail_count + 1
			table.insert(fail_parts, v.message or "(no reason given)")
		end
	end
	local passed = pass_count > fail_count

	if passed then
		ctx.emit("push_chat_entry", {
			session_id = origin,
			kind = { transient = "✓ Judgment passed" },
		})
	else
		ctx.emit("enqueue_user_message", {
			session_id = origin,
			text = "✗ Judgment failed: " .. table.concat(fail_parts, "; "),
		})
	end

	-- Aggregation complete. Resolve the lifecycle of EVERY participating
	-- instance, not just this one:
	--   pass → disable all (one-shot judges; the user re-enables manually)
	--   fail/tie → re-enable all (so the next turn re-runs every judge)
	local instances = ctx.get_global_data(instances_key(origin)) or {}
	for instance_id, _ in pairs(instances) do
		if passed then
			ctx.emit("disable_plugin", {
				session_id = origin,
				plugin_name = ctx.plugin_name,
				instance_id = instance_id,
			})
		else
			ctx.emit("enable_plugin", {
				session_id = origin,
				plugin_name = ctx.plugin_name,
				instance_id = instance_id,
			})
		end
	end

	-- Reset per-turn shared state for the next turn.
	ctx.set_global_data(verdicts_key(origin), {})
	ctx.set_global_data(completed_key(origin), 0)
end

-- ─── Hooks ────────────────────���─────────────────────────────────────────

--- @param ctx OnAttachCtx
function M.on_attach(ctx)
	local origin = ctx.session_id
	-- Increment the count.
	local ckey = count_key(origin)
	local count = (ctx.get_global_data(ckey) or 0) + 1
	ctx.set_global_data(ckey, count)
	-- Register this instance in the participants set.
	local ikey = instances_key(origin)
	local instances = ctx.get_global_data(ikey) or {}
	instances[ctx.instance_id] = true
	ctx.set_global_data(ikey, instances)
end

--- @param ctx OnDetachCtx
function M.on_detach(ctx)
	local origin = ctx.session_id
	-- Decrement the count.
	local ckey = count_key(origin)
	local count = (ctx.get_global_data(ckey) or 0) - 1
	if count <= 0 then
		-- Last instance leaving: clear all shared keys for this origin.
		ctx.set_global_data(ckey, nil)
		ctx.set_global_data(verdicts_key(origin), nil)
		ctx.set_global_data(completed_key(origin), nil)
		ctx.set_global_data(turn_key(origin), nil)
		ctx.set_global_data(instances_key(origin), nil)
	else
		ctx.set_global_data(ckey, count)
		-- Remove this instance from the participants set.
		local ikey = instances_key(origin)
		local instances = ctx.get_global_data(ikey) or {}
		instances[ctx.instance_id] = nil
		ctx.set_global_data(ikey, instances)
	end
end

--- @param ctx OnTurnEndCtx
function M.on_turn_end(ctx)
	-- Push a transient status indicator to the origin session.
	ctx.emit("push_chat_entry", {
		session_id = ctx.session_id,
		kind = { transient = "⚖ Judge evaluating..." },
	})

	local origin = ctx.session_id

	-- Per-turn reset: bump the turn counter and reset verdicts/completed.
	-- Because hooks serialize, the first fire each turn resets cleanly.
	local prev_turn = ctx.get_global_data(turn_key(origin)) or 0
	ctx.set_global_data(turn_key(origin), prev_turn + 1)
	ctx.set_global_data(verdicts_key(origin), {})
	ctx.set_global_data(completed_key(origin), 0)

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

	-- Tell the domain domain layer about the managed session so the sidebar can
	-- navigate to it (and reflect its busy state).
	ctx.emit("set_managed_session", {
		session_id = ctx.session_id,
		plugin_name = ctx.plugin_name,
		managed_session_id = judge_id,
		instance_id = ctx.instance_id,
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
			origin
		),
	})

	-- Return immediately. The child LLM will run, call a judgment tool,
	-- and the tool handler will post the verdict and (if last) emit the result.
end

return M
