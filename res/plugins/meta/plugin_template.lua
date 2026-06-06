-- Copy this file to start a new plugin. Delete hooks you don't need.
-- The `---@param` annotations wire your editor up to the types in
-- `meta/plugin_ctx.lua` for autocomplete and diagnostics.

local M = {}

---@param ctx OnAppStartedCtx
function M.on_app_started(ctx) end

---@param ctx OnSessionCreatedCtx
function M.on_session_created(ctx) end

---@param ctx OnUserSubmitCtx
function M.on_user_submit(ctx) end

---@param ctx OnTurnEndCtx
function M.on_turn_end(ctx) end

return M
