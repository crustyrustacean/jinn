-- Welcome plugin for nullslop.
-- Shows a startup greeting when the app starts and tips on new sessions.

local welcome_shown = false

ps.sub("app::started", function(payload)
    if welcome_shown then return end
    welcome_shown = true
    ns.emit("welcome::show", {
        message = "Welcome to nullslop! Press ? for keybindings.",
    })
end)

ps.sub("session::created", function(payload)
    ns.emit("welcome::session_tip", {
        message = "New session started. Type your message below.",
    })
end)
