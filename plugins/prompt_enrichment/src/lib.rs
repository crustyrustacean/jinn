//! Prompt enrichment — rewrite the draft via an LLM one-shot on `<M-e>`.
//!
//! Declares `<M-e>` (action `on_enrich`) to run an LLM one-shot rewrite of the
//! current draft, then writes the enriched text back into the input box. A
//! badge ("[Enrich]") is always drawn as a hotkey legend; while enriching it
//! shows "[Working]" instead. A second tap of `<M-e>` cancels an in-flight
//! enrichment.

wit_bindgen::generate!({
    path: "../../wit/jinn.wit",
    world: "plugin",
});

use crate::host::LlmOutcome;
use crate::jinn::plugin::types::BadgeDirective as WitBadgeDirective;
use crate::prelude::*;

const ENRICH_PROMPT: &str = "\
Rewrite the following user input into a clearer, more detailed prompt.
Preserve the user's intent and add specificity where helpful.
Return ONLY the rewritten prompt, with no preamble or explanation.";

#[derive(Default, Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
enum Status {
    #[default]
    Idle,
    Enriching,
}

struct PromptEnrichment;

impl Plugin for PromptEnrichment {
    fn get_manifest() -> Manifest {
        Manifest::new()
            .with_description("Prompt enrichment — rewrite the draft via an LLM one-shot on tap")
            .with_keybind(
                Keybind::new("Input", "<M-e>", "on_enrich")
                    .described_as("enrich prompt"),
            )
    }

    // Async trigger keyed by the keybind's `action` ("on_enrich").
    async fn run_trigger(action: String, ctx: TriggerCtx) {
        if action != "on_enrich" {
            return;
        }
        if ctx.text.is_empty() {
            return;
        }

        host::set_chat_input_enabled(&ctx.session_id, false);
        set_status(Status::Enriching);

        // Restore idle + re-enable input no matter how this resolves.
        let outcome = host::request_llm_oneshot(LlmOneshotReq {
            session_id: ctx.session_id.clone(),
            system: ENRICH_PROMPT.to_owned(),
            prompt: ctx.text.clone(),
            persist: false,
            disable_tool_loop: true,
            timeout_ms: Some(30_000),
            task: Some(format!("enrich:{}", ctx.session_id)),
        })
        .await;

        match outcome {
            LlmOutcome::Ok(resp) if !resp.text.is_empty() => {
                host::set_chat_input(&ctx.session_id, &resp.text);
            }
            // A cancel (retap) is intentional — suppress the error entry.
            LlmOutcome::Cancelled => {}
            // Other failures surface so the user knows enrichment failed.
            LlmOutcome::Ok(_) => {}
            LlmOutcome::Other(msg) => {
                host::push_error_entry(&ctx.session_id, &msg);
            }
        }

        host::set_chat_input_enabled(&ctx.session_id, true);
        set_status(Status::Idle);
    }

    // Sync veto: self-select on our keybind, cancel in-flight on retap.
    fn on_keybind_trigger(ctx: KeybindTriggerCtx) -> Option<KeybindResult> {
        if ctx.keybound_plugin != "prompt_enrichment" {
            return None;
        }
        if status() == Status::Enriching {
            host::cancel_task(&format!("enrich:{}", ctx.session_id));
            return Some(KeybindResult::Skip);
        }
        Some(KeybindResult::Run)
    }

    // Sync render: the badge.
    fn on_chat_input_badges_render(ctx: BadgeCtx) -> Option<WitBadgeDirective> {
        if status() == Status::Enriching {
            return Some(BadgeDirective::input_badge([
                Segment::text("[").muted(),
                Segment::text("Working").streaming(),
                Segment::text("]").muted(),
            ]));
        }

        let e_style = if ctx.mode == "input" {
            Style::AccentAction
        } else {
            Style::MutedText
        };
        Some(BadgeDirective::input_badge([
            Segment::text("[").muted(),
            Segment::styled("E", e_style),
            Segment::text("nrich").muted(),
            Segment::text("]").muted(),
        ]))
    }
}

// ── Host-owned bag helpers ───────────────────────────────────────────

#[derive(Default, Serialize, Deserialize)]
struct EnrichState {
    status: Status,
}

fn status() -> Status {
    bag::get_plugin_data::<EnrichState>()
        .map(|s| s.status)
        .unwrap_or_default()
}

fn set_status(status: Status) {
    bag::set_plugin_data(&EnrichState { status });
}

jinn_guest_pdk::plugin!();
jinn_guest_pdk::export_plugin!(PromptEnrichment);
