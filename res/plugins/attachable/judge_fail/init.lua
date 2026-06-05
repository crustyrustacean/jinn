-- description: Judgement fail — prompts user to retry
-- Fires on turn end if judgement failed.

local M = {}

function M.on_turn_end(ctx)
    ctx.emit("push_chat_entry", {
        session_id = ctx.session_id,
        message = "judgement failed, try again",
    })
end

return M
