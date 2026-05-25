-- Welcome plugin for nullslop.
-- Shows a startup greeting when the app starts.

local welcome_shown = false

ps.sub("app::started", function(payload)
    if welcome_shown then return end
    welcome_shown = true
    ns.emit("welcome::show", {
        message = "Welcome to nullslop! Press ? for keybindings.",
    })
end)
