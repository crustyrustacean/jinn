//! Fixture plugin for jinn-wasm-host integration tests.
//!
//! Exports a broad mix of hooks so tests can exercise:
//! - async host imports (`request-llm-oneshot`, `cancel-task`),
//! - optional hook semantics (absent exports skipped),
//! - the plugin-data bag (postcard round-trip, dual-store visibility),
//! - sync render hooks (badge directive, keybind trigger),
//! - plugin-defined async trigger fired by runtime export lookup.

wit_bindgen::generate!({
    path: "../../../../wit/jinn.wit",
    world: "plugin",
});

use crate::host::LlmOutcome;
use crate::jinn::plugin::types::BadgeDirective as WitBadgeDirective;
use crate::prelude::*;

#[derive(Default, Serialize, Deserialize)]
struct State {
    /// Count of `on-turn-end` calls observed — survives across hook calls via
    /// the plugin-data bag. The dual-store test writes here (async store) and
    /// reads here (sync store) to confirm the bag is shared across threads.
    turn_ends: u32,
    /// Last prompt processed by the trigger.
    last_enriched: Option<String>,
}

fn read_state() -> State {
    bag::get_plugin_data::<State>().unwrap_or_default()
}

fn write_state(s: &State) {
    bag::set_plugin_data(s);
}

struct Fixture;

impl Plugin for Fixture {
    fn get_manifest() -> Manifest {
        Manifest::new()
            .with_description("fixture test plugin")
            .with_keybind(
                Keybind::new("Input", "<M-e>", "on_enrich").described_as("enrich prompt"),
            )
    }

    // ── Lifecycle hook exercised by the async store ───────────────────
    async fn on_turn_end(_ctx: TurnEndCtx) {
        let mut s = read_state();
        s.turn_ends = s.turn_ends.saturating_add(1);
        write_state(&s);
    }

    // ── Sync render hook (badge) — rendered in input mode ─────────────
    fn on_chat_input_badges_render(ctx: BadgeCtx) -> Option<WitBadgeDirective> {
        if ctx.mode == "input" {
            return Some(BadgeDirective::input_badge([
                Segment::text("[").muted(),
                Segment::text("fixture").streaming(),
                Segment::text("]").muted(),
            ]));
        }
        None
    }

    // ── Sync keybind trigger — only respond to our own keybind ────────
    fn on_keybind_trigger(ctx: KeybindTriggerCtx) -> Option<KeybindResult> {
        if ctx.keybound_plugin != "test-plugin" {
            return None;
        }
        Some(KeybindResult::Run)
    }

    // ── Plugin-defined async trigger, fired by runtime export lookup ──
    async fn run_trigger(action: String, ctx: TriggerCtx) {
        if action != "on_enrich" {
            return;
        }
        let mut s = read_state();
        s.last_enriched = Some(ctx.text.clone());
        write_state(&s);

        match host::request_llm_oneshot(LlmOneshotReq {
            session_id: ctx.session_id.clone(),
            system: "rewrite".to_owned(),
            prompt: ctx.text.clone(),
            persist: false,
            disable_tool_loop: true,
            timeout_ms: Some(30_000),
            task: Some("fixture-enrich".to_owned()),
        })
        .await
        {
            LlmOutcome::Ok(resp) => {
                let mut s = read_state();
                s.last_enriched = Some(resp.text);
                write_state(&s);
            }
            LlmOutcome::Cancelled => {}
            LlmOutcome::Other(_) => {}
        }
    }
}

jinn_guest_pdk::plugin!();
jinn_guest_pdk::export_plugin!(Fixture);
