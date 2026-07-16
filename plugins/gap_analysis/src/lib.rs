//! Gap-analysis — runs `#gap-analysis` automatically once when the task list completes.
//!
//! Watches the session's task list via `on-task-list-updated` and folds
//! `is-complete` into the host-owned bag. When the list is complete and the
//! session goes idle (`on-turn-end`), enqueues `#gap-analysis` into the origin,
//! then disables itself via `disable-plugin`. Stays disabled until re-attached.
//!
//! Single-site expansion (`ChatSessionState::push_entry`) resolves the
//! `#gap-analysis` token to the prompt body in `res/prompts/gap-analysis.md` —
//! the plugin never touches the prompt text.

wit_bindgen::generate!({
    path: "../../wit/jinn.wit",
    world: "plugin",
});

use crate::prelude::*;

#[derive(Default, Serialize, Deserialize)]
struct GapState {
    is_complete: bool,
}

struct GapAnalysis;

impl Plugin for GapAnalysis {
    fn get_manifest() -> Manifest {
        Manifest::new()
            .with_description(
                "Gap-analysis plugin — runs #gap-analysis automatically once when the task list completes",
            )
    }

    // Fold the latest completion state for on-turn-end to read.
    async fn on_task_list_updated(ctx: TaskListCtx) {
        let mut state = bag::get_plugin_data::<GapState>().unwrap_or_default();
        state.is_complete = ctx.is_complete;
        bag::set_plugin_data(&state);
    }

    async fn on_turn_end(ctx: TurnEndCtx) {
        let is_complete = bag::get_plugin_data::<GapState>()
            .map(|s| s.is_complete)
            .unwrap_or(false);

        // Only fire at the Idle transition when the task list is complete.
        if !is_complete {
            return;
        }

        // Transient status so the user sees the analysis is queued.
        host::push_transient_entry(&ctx.session_id, "🔍 Gap analysis queued");

        // Enqueue the token; push_entry expands it against the prompt store.
        host::enqueue_user_message(&ctx.session_id, "#gap-analysis");

        // Disable this instance so it never fires again until re-attached.
        host::emit(Command::DisablePlugin(DisablePluginCmd {
            session_id: ctx.session_id.clone(),
            plugin_name: ctx.plugin_name.clone(),
            instance_id: ctx.instance_id.clone(),
        }));
    }
}

jinn_guest_pdk::plugin!();
jinn_guest_pdk::export_plugin!(GapAnalysis);
