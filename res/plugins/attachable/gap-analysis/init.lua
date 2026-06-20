-- description: Gap-analysis plugin — runs #gap-analysis automatically once when the task list completes
--
-- Watches the session's task list via `on_task_list_updated` and folds
-- `is_complete` into `plugin_data`. When the list is complete and the session
-- goes idle (`on_turn_end`), enqueues `#gap-analysis` into the origin once,
-- then disarms itself via the `fired` guard. Auto-rearms when a new plan
-- arrives (any `on_task_list_updated` with `is_complete == false`).
--
-- Single-site expansion (ChatSessionState::push_entry) resolves the
-- `#gap-analysis` token to the prompt body stored in
-- `res/prompts/gap-analysis.md` — the plugin never touches the prompt text.
--
-- Guard: `plugin_data.fired` (one-shot per plan), NOT `disable_plugin`. This
-- lets the plugin re-fire on a subsequent plan without the user re-attaching.

local M = {}

-- ─── Hooks ─────────────────────────────────────────────────────────────

--- @param ctx OnTaskListUpdatedCtx
function M.on_task_list_updated(ctx)
	-- Fold the latest completion state. When the list is (re)opened — e.g. a
	-- new plan arrives via set_list — auto-rearm so the next completion fires
	-- again. plugin_data defaults to {} on a fresh instance.
	local data = ctx.plugin_data or {}
	data.is_complete = ctx.is_complete
	if not ctx.is_complete then
		data.fired = false
	end
	ctx.set_plugin_data(data)
end

--- @param ctx OnTurnEndCtx
function M.on_turn_end(ctx)
	local data = ctx.plugin_data or {}
	-- Only fire at the Idle transition, only once per completed plan.
	if not data.is_complete or data.fired then
		return
	end

	-- Push a transient status so the user sees the analysis is queued.
	ctx.emit("push_chat_entry", {
		session_id = ctx.session_id,
		kind = { transient = "🔍 Gap analysis queued" },
	})

	-- Enqueue the token; push_entry expands it against the prompt store.
	ctx.emit("enqueue_user_message", {
		session_id = ctx.session_id,
		text = "#gap-analysis",
	})

	data.fired = true
	ctx.set_plugin_data(data)
end

return M
