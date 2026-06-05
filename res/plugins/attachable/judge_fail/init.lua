-- description: Judgement fail — prompts user to retry
-- Fires on turn end if judgement failed.

local M = {}

---@param ctx OnTurnEndCtx
function M.on_turn_end(ctx)
    ctx.emit("enqueue_user_message", {
        session_id = ctx.session_id,
        text = "judgement failed, try again",
    })
end

return M
