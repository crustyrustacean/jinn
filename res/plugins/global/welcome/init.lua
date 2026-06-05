-- description: Welcome greeting and new session tips
-- Welcome plugin for jinn.
-- Shows a startup greeting when the app starts and tips on new sessions.

local M = {}

function M.on_app_started(ctx)
    ctx.emit("push_chat_entry", {
        session_id = ctx.session_id,
        message = "Welcome to jinn! Press ? for keybindings.",
    })
end

function M.on_session_created(ctx)
    ctx.emit("push_chat_entry_transient", {
        session_id = ctx.session_id,
        message = "New session started. Type your message below.",
    })
end

return M
