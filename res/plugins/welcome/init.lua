-- Welcome plugin for jinn.
-- Shows a startup greeting when the app starts and tips on new sessions.

local welcome_shown = false

ps.sub("app::started", function(ctx)
    if welcome_shown then return end
    welcome_shown = true
    ns.emit("push_chat_entry", {
        session_id = ctx.session_id or "",
        message = "Welcome to jinn! Press ? for keybindings.",
    })
end)

ps.sub("session::created", function(ctx)
    ns.emit("push_chat_entry_transient", {
        session_id = ctx.session_id or "",
        message = "New session started. Type your message below.",
    })
end)
