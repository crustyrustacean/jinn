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

/// Pending verdict children keyed by origin: maps child session id → (instance, turn).
/// A child present here but absent from `VerdictMap` at aggregation time means it
/// ended without issuing a verdict → forced fail.
#[derive(Serialize, Deserialize, Default)]
struct Pending(BTreeMap<String, PendingChild>);

#[derive(Serialize, Deserialize)]
struct PendingChild {
    instance_id: String,
    turn: u32,
}

// ── Shared-key helpers (namespaced by origin) ───────────────────────────────

fn count_key(origin: &str) -> String { format!("judge:{origin}:count") }
fn verdicts_key(origin: &str) -> String { format!("judge:{origin}:verdicts") }
fn completed_key(origin: &str) -> String { format!("judge:{origin}:completed") }
fn turn_key(origin: &str) -> String { format!("judge:{origin}:turn") }
fn instances_key(origin: &str) -> String { format!("judge:{origin}:instances") }
fn pending_key(origin: &str) -> String { format!("judge:{origin}:pending") }
fn judged_key(origin: &str) -> String { format!("judge:{origin}:judged") }

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

        // Re-attaching (manually, after a prior one-shot verdict disabled
        // the instance) resets the judged guard so the next turn evaluates anew.
        bag::set_global_data::<bool>(&judged_key(origin), &false);
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
            bag::set_global_data::<()>(&pending_key(origin), &());
            bag::set_global_data::<()>(&judged_key(origin), &());
        } else {
            bag::set_global_data(&key, &count);
            let mut instances = bag::get_global_data::<Instances>(&instances_key(origin)).unwrap_or_default();
            instances.0.retain(|id| id != &ctx.instance_id);
            bag::set_global_data(&instances_key(origin), &instances);
        }
    }

    // ── Turn end: spawn the child judge session ──────────────────────────

    async fn on_turn_end(ctx: TurnEndCtx) {
        // Gate to origin sessions only. Child/automated sessions (e.g. the
        // judge's own evaluation child) never re-spawn a judge. Without this
        // guard a child that inherits the judge attachment would recurse, and
        // the fail message enqueued by aggregation can otherwise race the
        // DisablePlugin emit and re-trigger evaluation.
        if ctx.parent_session_id.is_some() {
            return;
        }

        let origin = ctx.session_id.clone();

        // Guard against re-judging within the same turn. When a fail verdict
        // enqueues a message it triggers a new origin turn; `DisablePlugin`
        // is racing to disable this instance. If on_turn_end fires before the
        // disable lands, this guard prevents a redundant re-evaluation of the
        // just-emitted verdict message.
        if bag::get_global_data::<bool>(&judged_key(&origin)).unwrap_or(false) {
            return;
        }

        // Host-provided authoritative turn number (count of assistant entries).
        // With N judge instances attached, on_turn_end fires N times per
        // genuine turn — once per instance. Only the FIRST fire per turn
        // performs the per-turn reset + force-fail; subsequent fires (same
        // turn) skip straight to spawning their own child. This prevents the
        // reset from clobbering verdicts/completed that sibling instances'
        // children are concurrently populating.
        let last_claimed = bag::get_global_data::<u32>(&turn_key(&origin)).unwrap_or(0);
        let first_fire_this_turn = ctx.turn != last_claimed;
        if first_fire_this_turn {
            bag::set_global_data(&turn_key(&origin), &ctx.turn);

            // Force-fail any children still pending from a prior turn that
            // never issued a verdict (child ended/timed out without calling
            // a verdict tool). No-verdict = fail; never silently pass.
            force_fail_stale_children(&origin);

            // Genuine new turn: reset shared state.
            bag::set_global_data(&verdicts_key(&origin), &VerdictMap::default());
            bag::set_global_data::<u32>(&completed_key(&origin), &0);

            // Transient status indicator.
            host::push_transient_entry(&origin, "⚖ Judge evaluating...");
        }

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

        // Track this child as pending so a no-verdict exit can be force-failed
        // on the next origin turn.
        {
            let mut pending = bag::get_global_data::<Pending>(&pending_key(&origin)).unwrap_or_default();
            pending.0.insert(
                judge_id.clone(),
                PendingChild { instance_id: ctx.instance_id.clone(), turn: ctx.turn },
            );
            bag::set_global_data(&pending_key(&origin), &pending);
        }

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
    verdicts.0.insert(me.clone(), Verdict { verdict: verdict.clone(), message: message.clone() });
    bag::set_global_data(&verdicts_key(&origin), &verdicts);

    // This child issued a verdict: it's no longer pending.
    {
        let mut pending = bag::get_global_data::<Pending>(&pending_key(&origin)).unwrap_or_default();
        pending.0.remove(&me);
        bag::set_global_data(&pending_key(&origin), &pending);
    }

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

    // Resolve lifecycle of EVERY participating instance.
    // Both pass AND fail are one-shot: disable all instances so the
    // verdict (especially the enqueued fail message) does not re-trigger
    // a new origin turn that would re-judge the verdict itself. The user
    // re-attaches manually to run another evaluation.
    let instances = bag::get_global_data::<Instances>(&instances_key(origin)).unwrap_or_default();
    for instance_id in &instances.0 {
        host::emit(Command::DisablePlugin(DisablePluginCmd {
            session_id: origin.to_owned(),
            plugin_name: ctx.plugin_name.clone(),
            instance_id: instance_id.clone(),
        }));
    }

    // Reset per-turn shared state for the next turn.
    bag::set_global_data::<VerdictMap>(&verdicts_key(origin), &VerdictMap::default());
    bag::set_global_data::<u32>(&completed_key(origin), &0);
    bag::set_global_data::<Pending>(&pending_key(origin), &Pending::default());
    // Mark this origin as judged so a re-judge (fail message triggering a new
    // turn before DisablePlugin lands) is suppressed. Cleared on manual re-attach.
    bag::set_global_data::<bool>(&judged_key(origin), &true);
}

/// Force-fail any children from a prior turn that never issued a verdict.
///
/// Called at the start of each origin `on_turn_end`. A child present in
/// `pending` but absent from `verdicts` ended/timed out without calling a
/// verdict tool. Each such child is recorded as an explicit fail so
/// aggregation reflects it, and `completed` is advanced. This guarantees a
/// no-verdict child can never silently stall or pass.
fn force_fail_stale_children(origin: &str) {
    let mut pending = bag::get_global_data::<Pending>(&pending_key(origin)).unwrap_or_default();
    if pending.0.is_empty() {
        return;
    }
    let count = bag::get_global_data::<u32>(&count_key(origin)).unwrap_or(0);
    let mut verdicts = bag::get_global_data::<VerdictMap>(&verdicts_key(origin)).unwrap_or_default();
    let mut added = 0u32;
    // Children from any turn older than the current pending turn are stale.
    // The current turn's children haven't had a chance to run yet.
    let max_turn = pending.0.values().map(|c| c.turn).max().unwrap_or(0);
    let stale: Vec<String> = pending.0.iter()
        .filter(|(_, c)| c.turn < max_turn)
        .map(|(id, _)| id.clone())
        .collect();
    for child_id in &stale {
        verdicts.0.insert(child_id.clone(), Verdict {
            verdict: "failed".to_owned(),
            message: Some("judge session ended without a verdict".to_owned()),
        });
        pending.0.remove(child_id);
        added += 1;
    }
    if added > 0 {
        bag::set_global_data(&verdicts_key(origin), &verdicts);
        let completed = bag::get_global_data::<u32>(&completed_key(origin)).unwrap_or(0) + added;
        bag::set_global_data::<u32>(&completed_key(origin), &completed);
        // If the force-fail pushed completed to the threshold, aggregate now.
        if completed >= count && count > 0 {
            let ctx = ToolCtx {
                session_id: String::new(),
                parent_session_id: Some(origin.to_owned()),
                instance_id: String::new(),
                plugin_name: "judge".to_owned(),
            };
            aggregate_and_emit(origin, &verdicts, &ctx);
        }
    }
    bag::set_global_data(&pending_key(origin), &pending);
}

/// Parse `{"message": "..."}` from the LLM-supplied tool args JSON.
fn parse_message(args: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct FailedArgs { message: Option<String> }
    serde_json::from_str::<FailedArgs>(args).ok().and_then(|a| a.message)
}

jinn_guest_pdk::plugin!();
jinn_guest_pdk::export_plugin!(Judge);
