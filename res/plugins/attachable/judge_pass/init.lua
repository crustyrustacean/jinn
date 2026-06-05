-- description: Judgement pass — acknowledges and disables itself
-- Fires on turn end when judgement passes. Disables itself after firing
-- so it only runs once per attachment.

local M = {}

function M.on_turn_end(ctx)
    ctx.emit("push_chat_entry", {
        session_id = ctx.session_id,
        kind = { system = "judgement passed" },
    })
    ctx.emit("disable_plugin", {
        session_id = ctx.session_id,
        plugin_name = ctx.plugin_name,
    })
end

return M
