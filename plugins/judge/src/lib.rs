//! Judge plugin — evaluates assistant responses after each turn using a child LLM session.
//!
//! Spawns an automated child session with `session-query` access. The child LLM
//! reviews the origin session's last assistant response and calls either
//! `judgment_passed` or `judgment_failed(message)` (plugin-defined tools) to
//! route results back.
//!
//! A FRESH child session is created every turn (persist=false, no history reuse).
//! Tool handlers derive the origin from `ctx.parent_session_id` (child→origin
//! parent edge) and key their verdict on `ctx.session_id` (the child's own id,
//! unique per verdict).
//!
//! Multi-instance (panel/aggregation): multiple judge instances may be attached
//! to one origin. Each runs its own child in parallel. They coordinate via a
//! shared global-data bag keyed by origin. The LAST judge to complete aggregates
//! all verdicts and emits ONE merged result, then disables/re-enables all
//! instances. Verdicts are keyed by child session id (unique per verdict).
//!
//! Concurrency note: read-modify-write on the global bag is safe because hooks
//! + tool callbacks serialize through the single plugin thread.

wit_bindgen::generate!({
    path: "../../wit/jinn.wit",
    world: "plugin",
});

use crate::prelude::*;
use std::collections::BTreeMap;

// ── Verdict types (serialized into the shared global-data bag) ──────────────

#[derive(Serialize, Deserialize)]
struct Verdict {
    verdict: String,
    message: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct VerdictMap(BTreeMap<String, Verdict>);

#[derive(Serialize, Deserialize, Default)]
struct Instances(Vec<String>);

// ── Shared-key helpers (namespaced by origin) ───────────────────────────────

fn count_key(origin: &str) -> String { format!("judge:{origin}:count") }
fn verdicts_key(origin: &str) -> String { format!("judge:{origin}:verdicts") }
fn completed_key(origin: &str) -> String { format!("judge:{origin}:completed") }
fn turn_key(origin: &str) -> String { format!("judge:{origin}:turn") }
fn instances_key(origin: &str) -> String { format!("judge:{origin}:instances") }

// ── Plugin ──────────────────────────────────────────────────────────────────

struct Judge;

impl Plugin for Judge {
    fn get_manifest() -> Manifest {
        Manifest::new()
            .with_description(
                "Judge plugin — evaluates assistant responses after each turn using a child LLM session",
            )
            .with_tool(
                Tool::new(
                    "judgment_passed",
                    "Call when the assistant's response passes evaluation and then STOP.",
                )
                .attached(),
            )
            .with_tool(
                Tool::new(
                    "judgment_failed",
                    "Call when the assistant's response fails evaluation and then STOP.",
                )
                .attached()
                .with_param(
                    ToolParam::new("message", "string")
                        .described_as("Why the response failed"),
                ),
            )
    }

    // ── Lifecycle: maintain shared count + participants set ──────────────

    async fn on_attach(ctx: AttachCtx) {
        let origin = &ctx.session_id;

        let count = bag::get_global_data::<u32>(&count_key(origin)).unwrap_or(0) + 1;
        bag::set_global_data(&count_key(origin), &count);

        let mut instances = bag::get_global_data::<Instances>(&instances_key(origin)).unwrap_or_default();
        if !instances.0.contains(&ctx.instance_id) {
            instances.0.push(ctx.instance_id.clone());
        }
        bag::set_global_data(&instances_key(origin), &instances);
    }

    async fn on_detach(ctx: AttachCtx) {
        let origin = &ctx.session_id;
        let key = count_key(origin);
        let count = bag::get_global_data::<u32>(&key).unwrap_or(0).saturating_sub(1);

        if count == 0 {
            // Last instance leaving: clear all shared keys for this origin.
            bag::set_global_data::<()>(&count_key(origin), &());
            bag::set_global_data::<()>(&verdicts_key(origin), &());
            bag::set_global_data::<()>(&completed_key(origin), &());
            bag::set_global_data::<()>(&turn_key(origin), &());
            bag::set_global_data::<()>(&instances_key(origin), &());
        } else {
            bag::set_global_data(&key, &count);
            let mut instances = bag::get_global_data::<Instances>(&instances_key(origin)).unwrap_or_default();
            instances.0.retain(|id| id != &ctx.instance_id);
            bag::set_global_data(&instances_key(origin), &instances);
        }
    }

    // ── Turn end: spawn the child judge session ──────────────────────────

    async fn on_turn_end(ctx: TurnEndCtx) {
        let origin = ctx.session_id.clone();

        // Transient status indicator.
        host::push_transient_entry(&origin, "⚖ Judge evaluating...");

        // Per-turn reset: bump turn counter and clear verdicts/completed.
        let prev_turn = bag::get_global_data::<u32>(&turn_key(&origin)).unwrap_or(0);
        bag::set_global_data(&turn_key(&origin), &(prev_turn + 1));
        bag::set_global_data(&verdicts_key(&origin), &VerdictMap::default());
        bag::set_global_data::<u32>(&completed_key(&origin), &0);

        // Every turn: create a fresh transient child session. No reuse, no reset.
        let result = host::create_session(CreateSessionReq {
            parent_session_id: origin.clone(),
            automated: true,
            persist: false,
            inherit_tools: false,
            tools: vec![
                "judgment_passed".to_owned(),
                "judgment_failed".to_owned(),
                "session_query".to_owned(),
            ],
        })
        .await;

        let judge_id = match result {
            CreateSessionOutcome::Ok(resp) => resp.session_id,
            CreateSessionOutcome::Cancelled => return,
            CreateSessionOutcome::Other(msg) => {
                host::push_error_entry(&origin, &format!("judge: failed to create session: {msg}"));
                return;
            }
        };

        // Tell the domain layer about the managed session (sidebar navigation).
        host::emit(Command::SetManagedSession(SetManagedSessionCmd {
            session_id: origin.clone(),
            plugin_name: ctx.plugin_name.clone(),
            managed_session_id: judge_id.clone(),
            instance_id: ctx.instance_id.clone(),
        }));

        // Ask the judge to evaluate the latest response.
        let prompt = format!(
            "You are a response quality judge. The origin session UUID is: {origin}\n\
             Use session_query to inspect the origin session's conversation history.\n\
             When calling session_query, pass this exact UUID as the session_id parameter.\n\n\
             After reviewing the last assistant response, call exactly one of:\n\
               - judgment_passed() if the response is satisfactory\n\
               - judgment_failed(message) if the response has problems, explaining what went wrong\n\n\
             Be thorough. Check for accuracy, completeness, and relevance. After issuing a judgment tool call, STOP."
        );
        host::enqueue_user_message(&judge_id, &prompt);
    }

    // ── Plugin-defined tools (single dispatch export) ───────────────────

    async fn run_tool(name: String, args: String, ctx: ToolCtx) -> String {
        match name.as_str() {
            "judgment_passed" => record_verdict(ctx, "passed".to_owned(), None),
            "judgment_failed" => {
                let message = parse_message(&args);
                record_verdict(ctx, "failed".to_owned(), message);
            }
            _ => {}
        }
        String::new()
    }
}


// ── Verdict posting + aggregation ───────────────────────────────────────────

fn record_verdict(ctx: ToolCtx, verdict: String, message: Option<String>) {
    // parent_session_id is the origin; session_id is the child (unique per verdict).
    let origin = match &ctx.parent_session_id {
        Some(o) => o.clone(),
        None => return,
    };
    let me = ctx.session_id.clone();
    let count = bag::get_global_data::<u32>(&count_key(&origin)).unwrap_or(0);

    // Post this child's verdict.
    let mut verdicts = bag::get_global_data::<VerdictMap>(&verdicts_key(&origin)).unwrap_or_default();
    verdicts.0.insert(me, Verdict { verdict: verdict.clone(), message: message.clone() });
    bag::set_global_data(&verdicts_key(&origin), &verdicts);

    // Increment completed.
    let completed = bag::get_global_data::<u32>(&completed_key(&origin)).unwrap_or(0) + 1;
    bag::set_global_data::<u32>(&completed_key(&origin), &completed);

    // Only the last to finish aggregates + emits.
    if completed < count {
        return;
    }

    aggregate_and_emit(&origin, &verdicts, &ctx);
}

fn aggregate_and_emit(origin: &str, verdicts: &VerdictMap, ctx: &ToolCtx) {
    // Majority vote: strict pass majority wins; tie or fail-majority fails.
    let mut pass_count = 0u32;
    let mut fail_count = 0u32;
    let mut fail_parts: Vec<String> = Vec::new();
    for v in verdicts.0.values() {
        match v.verdict.as_str() {
            "passed" => pass_count += 1,
            "failed" => {
                fail_count += 1;
                fail_parts.push(v.message.clone().unwrap_or_else(|| "(no reason given)".to_owned()));
            }
            _ => {}
        }
    }
    let passed = pass_count > fail_count;

    if passed {
        host::push_transient_entry(origin, "✓ Judgment passed");
    } else {
        let text = format!("✗ Judgment failed: {}", fail_parts.join("; "));
        host::enqueue_user_message(origin, &text);
    }

    // Resolve lifecycle of EVERY participating instance:
    //   pass → disable all (one-shot; user re-enables manually)
    //   fail/tie → re-enable all (next turn re-runs every judge)
    let instances = bag::get_global_data::<Instances>(&instances_key(origin)).unwrap_or_default();
    for instance_id in &instances.0 {
        let cmd = if passed {
            Command::DisablePlugin(DisablePluginCmd {
                session_id: origin.to_owned(),
                plugin_name: ctx.plugin_name.clone(),
                instance_id: instance_id.clone(),
            })
        } else {
            Command::EnablePlugin(EnablePluginCmd {
                session_id: origin.to_owned(),
                plugin_name: ctx.plugin_name.clone(),
                instance_id: instance_id.clone(),
            })
        };
        host::emit(cmd);
    }

    // Reset per-turn shared state for the next turn.
    bag::set_global_data::<VerdictMap>(&verdicts_key(origin), &VerdictMap::default());
    bag::set_global_data::<u32>(&completed_key(origin), &0);
}

/// Parse `{"message": "..."}` from the LLM-supplied tool args JSON.
fn parse_message(args: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct FailedArgs { message: Option<String> }
    serde_json::from_str::<FailedArgs>(args).ok().and_then(|a| a.message)
}

jinn_guest_pdk::plugin!();
jinn_guest_pdk::export_plugin!(Judge);
