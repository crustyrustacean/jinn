-- description: Gap-analysis plugin — runs #gap-analysis automatically once when the task list completes
--
-- Watches the session's task list via `on_task_list_updated` and folds
-- `is_complete` into `plugin_data`. When the list is complete and the session
-- goes idle (`on_turn_end`), enqueues `#gap-analysis` into the origin, then
-- disables itself via `disable_plugin`. Stays disabled until the user
-- re-attaches it.
--
-- Single-site expansion (ChatSessionState::push_entry) resolves the
-- `#gap-analysis` token to the prompt body stored in
-- `res/prompts/gap-analysis.md` — the plugin never touches the prompt text.
local M = {}

-- ─── Hooks ─────────────────────────────────────────────────────────────

--- @param ctx OnTaskListUpdatedCtx
function M.on_task_list_updated(ctx)
	-- Fold the latest completion state for on_turn_end to read.
	-- plugin_data defaults to {} on a fresh instance.
	local data = ctx.plugin_data or {}
	data.is_complete = ctx.is_complete
	ctx.set_plugin_data(data)
end

--- @param ctx OnTurnEndCtx
function M.on_turn_end(ctx)
	local data = ctx.plugin_data or {}
	-- Only fire at the Idle transition when the task list is complete.
	if not data.is_complete then
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

	-- Disable this instance so it never fires again. The plugin stays
	-- disabled until the user re-attaches it.
	ctx.emit("disable_plugin", {
		session_id = ctx.session_id,
		plugin_name = ctx.plugin_name,
		instance_id = ctx.instance_id,
	})
end

return M
