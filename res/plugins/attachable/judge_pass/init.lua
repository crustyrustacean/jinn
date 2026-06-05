-- description: Judgement pass — acknowledges and disables workflow
-- Fires on turn end when judgement passes.

local M = {}

function M.on_turn_end(ctx)
    ctx.emit("push_chat_entry", {
        session_id = ctx.session_id,
        message = "judgement passed",
    })
end

return M
