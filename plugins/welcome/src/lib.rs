//! Welcome plugin — startup greeting and new-session tips.
//!
//! Shows a system entry when the app starts and a transient entry when a new
//! session is created. No keybinds, no tools, no state — the simplest plugin
//! shape, demonstrating the minimal `Plugin` trait surface.

wit_bindgen::generate!({
    path: "../../wit/jinn.wit",
    world: "plugin",
});

use crate::prelude::*;

struct Welcome;

impl Plugin for Welcome {
    fn get_manifest() -> Manifest {
        Manifest::new().with_description("Welcome greeting and new session tips")
    }

    async fn on_app_started(ctx: SessionCtx) {
        host::push_system_entry(
            &ctx.session_id,
            "Welcome to jinn! Press ? for keybindings.",
        );
    }

    async fn on_session_created(ctx: SessionCtx) {
        host::push_transient_entry(
            &ctx.session_id,
            "New session started. Type your message below.",
        );
    }
}

jinn_guest_pdk::plugin!();
jinn_guest_pdk::export_plugin!(Welcome);
