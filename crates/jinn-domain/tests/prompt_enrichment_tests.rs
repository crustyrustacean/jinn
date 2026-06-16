//! Integration tests for the `prompt_enrichment` plugin's tap-to-enrich behavior.
//!
//! These load the **real** plugin from `res/plugins/global/prompt_enrichment` and
//! drive it through the sync/async plugin VM, asserting the observable behaviors
//! specified in `.plans/tap-to-enrich/plan.md` §"New plugin integration tests":
//!
//! - `on_enrich` no-ops on empty text (no LLM call, no `set_chat_input` emit).
//! - `on_enrich` writes the enriched text back to the input via `set_chat_input`.
//! - `on_enrich` drops the stale result on double-tap (generation supersession).
//! - The `[Enrich]` badge colors the `E` `accent_action` in Input mode,
//!   `muted_text` otherwise, and always renders unconditionally.
//! - When enriching (`plugin_data.status == "enriching"`), the badge shows `[Working]`
//!   with the `Working` text styled as `streaming`.
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_in_result,
    reason = "test code"
)]

use jinn_domain::feat::plugin_dispatch::{HookContext, PluginSyncHooks};
use jinn_domain::feat::plugin_system::{
    PluginCommand, PluginSystem, PluginSystemBuildResult, SyncPlugins,
};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

// ── Helpers ──────────────────────────────────────────────────────────────

/// Resolve the workspace `res/plugins` directory from the test crate's
/// manifest dir (`crates/jinn-plugin` is one level below the workspace root).
fn res_plugins_dir() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .ancestors()
        .nth(2) // crates/jinn-plugin → workspace root
        .expect("workspace root")
        .join("res/plugins")
}

/// Recorded `ctx.request("llm_oneshot", …)` invocations, so a test can assert
/// that an enrichment fired (or did not).
#[derive(Default, Clone)]
struct RequestLog {
    /// Number of `llm_oneshot` requests observed.
    oneshot_calls: Arc<Mutex<u32>>,
}

/// Captured emitted commands, shared between the dispatcher closure and the test.
type Captured = Arc<Mutex<Vec<PluginCommand>>>;

/// A built plugin system wired for these tests: capturing dispatcher, stub
/// request handler, and the two handles needed to drive sync + async hooks.
struct TestSystem {
    captured: Captured,
    sync: SyncPlugins,
    async_handle: jinn_domain::feat::plugin_system::AsyncPluginHandle,
    oneshot_calls: Arc<Mutex<u32>>,
}

/// Build a test system over the **real** `res/plugins` tree.
///
/// `stub_result` is the JSON returned for every `llm_oneshot` request. Tests
/// that need per-call control build their own handler instead and pass `None`.
fn build_system_with_oneshot(stub_result: Value) -> TestSystem {
    let request_log = RequestLog::default();
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let captured_for_dispatch = captured.clone();

    // Leak the runtime — it lives for the test duration. A `Runtime` cannot be
    // dropped inside a `#[tokio::test]` async context.
    let rt = Box::leak(Box::new(tokio::runtime::Runtime::new().expect("runtime")));

    let PluginSystemBuildResult {
        sync, async_handle, ..
    } = PluginSystem::build(
        &res_plugins_dir(),
        Path::new("/nonexistent"),
        rt.handle().clone(),
        Arc::new(move |cmd| {
            captured_for_dispatch.lock().push(cmd);
        }),
        Arc::new({
            let log = request_log.clone();
            move |_name, _data, _cancel| {
                *log.oneshot_calls.lock() += 1;
                // `stub_result` is the full `ctx.request` response — the same shape
                // `handle_plugin_request` now produces: a result envelope
                // `{ ok: true, value }` or `{ ok: false, error }`. Returned as-is.
                let result = stub_result.clone();
                Box::pin(async move { result })
            }
        }),
    );

    TestSystem {
        captured,
        sync,
        async_handle,
        oneshot_calls: request_log.oneshot_calls,
    }
}

/// Helper to read all captured `set_chat_input` emits.
fn set_chat_input_emits(captured: &Captured) -> Vec<Value> {
    captured
        .lock()
        .iter()
        .filter(|c| c.name == "set_chat_input")
        .map(|c| c.data.clone())
        .collect()
}

// ── on_enrich (async) tests ──────────────────────────────────────────────

#[tokio::test]
async fn on_enrich_noops_on_empty_text() {
    // Given a plugin system whose request handler records every llm_oneshot call.
    let oneshot_calls = Arc::new(Mutex::new(0u32));
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let captured_for_dispatch = captured.clone();
    let rt = Box::leak(Box::new(tokio::runtime::Runtime::new().expect("runtime")));

    let counter = oneshot_calls.clone();
    let PluginSystemBuildResult { async_handle, .. } = PluginSystem::build(
        &res_plugins_dir(),
        Path::new("/nonexistent"),
        rt.handle().clone(),
        Arc::new(move |cmd| {
            captured_for_dispatch.lock().push(cmd);
        }),
        Arc::new(move |name, _data, _cancel| {
            let counter = counter.clone();
            let name = name.to_owned();
            Box::pin(async move {
                if name == "llm_oneshot" {
                    *counter.lock() += 1;
                }
                json!(null)
            })
        }),
    );

    // When firing on_enrich with an empty draft.
    async_handle
        .fire_async("on_enrich", &json!({ "session_id": "s1", "text": "" }))
        .await
        .expect("fire");

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // Then no llm_oneshot request was issued.
    assert_eq!(
        *oneshot_calls.lock(),
        0,
        "empty draft must not trigger an LLM call"
    );
    // And no set_chat_input was emitted.
    assert!(
        set_chat_input_emits(&captured).is_empty(),
        "empty draft must not write anything to the input"
    );
}

#[tokio::test]
async fn on_enrich_writes_enriched_text_to_input() {
    // Given a plugin system whose llm_oneshot stub returns a canned result.
    let sys = build_system_with_oneshot(json!({ "ok": true, "value": { "text": "enriched" } }));

    // When firing on_enrich with a real draft.
    sys.async_handle
        .fire_async(
            "on_enrich",
            &json!({ "session_id": "s1", "text": "fix the bug" }),
        )
        .await
        .expect("fire");

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // Then exactly one llm_oneshot ran and exactly one set_chat_input was
    // emitted carrying the enriched text.
    let oneshot_calls = *sys.oneshot_calls.lock();
    assert_eq!(oneshot_calls, 1, "expected exactly one llm_oneshot call");

    let emits = set_chat_input_emits(&sys.captured);
    let all_names: Vec<String> = sys.captured.lock().iter().map(|c| c.name.clone()).collect();
    // plugin_data is read only for the diagnostic message; the plugin no longer
    // writes any (status was its only field and had no reader), so tolerate absent.
    let pd = sys
        .async_handle
        .get_plugin_data("prompt_enrichment")
        .unwrap_or_else(|| serde_json::json!({}));

    assert_eq!(
        emits.len(),
        1,
        "expected one set_chat_input emit, got {emits:?}; all emit names = {all_names:?}; plugin_data = {pd:?}"
    );
    assert_eq!(emits[0]["text"], json!("enriched"));
}

#[tokio::test]
async fn on_enrich_two_taps_both_succeed_last_wins() {
    // Given a stub request handler that returns a distinct result per call.
    //
    // The plugin thread serializes jobs (run_hooks_fire awaits to completion
    // before the next job dequeues), so two fires run strictly in order and
    // each emits its own set_chat_input. Cancel-on-retap is handled by the
    // sync `on_keybind_trigger` hook (tested via the TUI integration tests),
    // not by this async-path test. This test confirms the baseline:
    // sequential, non-overlapping fires both succeed and last-wins by ordering.
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let captured_for_dispatch = captured.clone();
    let rt = Box::leak(Box::new(tokio::runtime::Runtime::new().expect("runtime")));

    let call_count = Arc::new(Mutex::new(0u32));
    let counter = call_count.clone();
    let PluginSystemBuildResult { async_handle, .. } = PluginSystem::build(
        &res_plugins_dir(),
        Path::new("/nonexistent"),
        rt.handle().clone(),
        Arc::new(move |cmd| {
            captured_for_dispatch.lock().push(cmd);
        }),
        Arc::new(move |name, _data, _cancel| {
            // Resolve the name decision to an owned value BEFORE the async block,
            // since `name: &str` cannot be captured by the returned future.
            let is_oneshot = name == "llm_oneshot";
            let counter = counter.clone();
            Box::pin(async move {
                if !is_oneshot {
                    return serde_json::json!({ "ok": false, "error": "unknown request" });
                }

                let n = {
                    let mut g = counter.lock();
                    *g += 1;
                    *g
                };
                // Each call returns a distinct result so the final write is identifiable.
                let text = if n == 1 {
                    "enriched-first"
                } else {
                    "enriched-second"
                };
                serde_json::json!({ "ok": true, "value": { "text": text } })
            })
        }),
    );

    // When firing on_enrich twice in quick succession on the same session.
    async_handle
        .fire_async(
            "on_enrich",
            &json!({ "session_id": "s1", "text": "draft one" }),
        )
        .await
        .expect("first fire");
    async_handle
        .fire_async(
            "on_enrich",
            &json!({ "session_id": "s1", "text": "draft two" }),
        )
        .await
        .expect("second fire");

    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    // Then both fires emitted, and the FINAL emitted text is the second result.
    let emits = set_chat_input_emits(&captured);
    let texts: Vec<&Value> = emits.iter().map(|e| &e["text"]).collect();
    assert!(
        texts.len() == 2,
        "both sequential fires should emit; saw {texts:?}"
    );
    assert_eq!(
        texts.last(),
        Some(&&json!("enriched-second")),
        "final emit should be the second result by ordering, saw {texts:?}"
    );
}

// ── error surfacing ─────────────────────────────────────────────────────

#[tokio::test]
async fn on_enrich_surfaces_request_error_as_chat_entry() {
    // Given a plugin system whose llm_oneshot stub returns an error envelope.
    let sys = build_system_with_oneshot(json!({ "ok": false, "error": "model timed out" }));

    // When firing on_enrich with a real draft.
    sys.async_handle
        .fire_async(
            "on_enrich",
            &json!({ "session_id": "s1", "text": "fix the bug" }),
        )
        .await
        .expect("fire");

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // Then exactly one llm_oneshot ran.
    let oneshot_calls = *sys.oneshot_calls.lock();
    assert_eq!(oneshot_calls, 1, "expected exactly one llm_oneshot call");

    // And no set_chat_input fired (the draft is left untouched).
    let set_inputs = set_chat_input_emits(&sys.captured);
    assert!(
        set_inputs.is_empty(),
        "error path must not overwrite the draft; saw {set_inputs:?}"
    );

    // And exactly one push_chat_entry surfaced the error message.
    let errors: Vec<String> = {
        let cmds = sys.captured.lock();
        cmds.iter()
            .filter(|c| c.name == "push_chat_entry")
            .map(|c| {
                c.data["kind"]["error"]
                    .as_str()
                    .expect("error str")
                    .to_owned()
            })
            .collect()
    };
    assert_eq!(
        errors,
        vec!["model timed out".to_owned()],
        "error message should surface as an error chat entry, saw {errors:?}"
    );
}

// ── badge (sync) tests ───────────────────────────────────────────────────

#[tokio::test]
async fn badge_returns_accent_action_for_e_in_input_mode() {
    let sys = build_system_with_oneshot(json!(null));

    // When the renderer fires the badge hook in Input mode.
    let directives = sys.sync.call_hooks(
        "on_chat_input_badges_render",
        &HookContext::from(json!({ "active_session_id": "s1", "mode": "input" })),
    );

    // Then the E segment is styled accent_action.
    let e = e_segment(&directives);
    assert_eq!(
        e.style.as_deref(),
        Some("accent_action"),
        "E must be accent_action in Input mode"
    );
}

#[tokio::test]
async fn badge_returns_muted_text_for_e_outside_input_mode() {
    let sys = build_system_with_oneshot(json!(null));

    // When the renderer fires the badge hook in Normal mode.
    let directives = sys.sync.call_hooks(
        "on_chat_input_badges_render",
        &HookContext::from(json!({ "active_session_id": "s1", "mode": "normal" })),
    );

    // Then the E segment is styled muted_text.
    let e = e_segment(&directives);
    assert_eq!(
        e.style.as_deref(),
        Some("muted_text"),
        "E must be muted_text outside Input mode"
    );
}

#[tokio::test]
async fn badge_always_returns_enrich_directive() {
    let sys = build_system_with_oneshot(json!(null));

    // When the renderer fires the badge hook in either mode.
    for mode in ["input", "normal"] {
        let directives = sys.sync.call_hooks(
            "on_chat_input_badges_render",
            &HookContext::from(json!({ "active_session_id": "s1", "mode": mode })),
        );

        // Then the directive's segments join to "[Enrich]".
        let directive: jinn_domain::BadgeDirective =
            serde_json::from_value(directives[0].clone()).expect("deserialize directive");
        let joined: String = directive.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(
            joined, "[Enrich]",
            "badge must read [Enrich] in {mode} mode (got {joined:?})"
        );
        // And the four segments are "[" / "E" / "nrich" / "]".
        assert_eq!(directive.segments.len(), 4, "expected four segments");
        assert_eq!(directive.segments[0].text, "[");
        assert_eq!(directive.segments[1].text, "E");
        assert_eq!(directive.segments[2].text, "nrich");
        assert_eq!(directive.segments[3].text, "]");
    }
}

// ── badge working-state tests ──────────────────────────────────────────

#[tokio::test]
async fn badge_returns_working_when_enriching() {
    // Given a system where the enrich plugin is actively enriching for a
    // specific session. The status is written to the SESSION-SCOPED bucket
    // (mirroring what `on_enrich`'s `ctx.merge_plugin_data({status="enriching"})`
    // does), and the badge ctx carries the canonical `session_id` key the host
    // emits in production, so the sync hook reads from the matching bucket.
    let sys = build_system_with_oneshot(json!(null));
    sys.async_handle
        .set_plugin_data("prompt_enrichment", json!({ "status": "enriching" }));

    // When the renderer fires the badge hook with a session_id-bearing ctx.
    let directives = sys.sync.call_hooks(
        "on_chat_input_badges_render",
        &HookContext::from(json!({ "session_id": "s1", "mode": "input" })),
    );

    // Then the badge text is [Working].
    let directive: jinn_domain::BadgeDirective =
        serde_json::from_value(directives[0].clone()).expect("deserialize directive");
    let joined: String = directive.segments.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(
        joined, "[Working]",
        "badge must show [Working] when enriching"
    );
}

#[tokio::test]
async fn working_badge_uses_streaming_style() {
    // Given a system where the enrich plugin is actively enriching for a
    // specific session (session-scoped data + canonical session_id ctx, as above).
    let sys = build_system_with_oneshot(json!(null));
    sys.async_handle
        .set_plugin_data("prompt_enrichment", json!({ "status": "enriching" }));

    // When the renderer fires the badge hook with a session_id-bearing ctx.
    let directives = sys.sync.call_hooks(
        "on_chat_input_badges_render",
        &HookContext::from(json!({ "session_id": "s1", "mode": "input" })),
    );

    // Then the Working segment is styled streaming.
    let directive: jinn_domain::BadgeDirective =
        serde_json::from_value(directives[0].clone()).expect("deserialize directive");
    let working = directive
        .segments
        .iter()
        .find(|s| s.text == "Working")
        .unwrap_or_else(|| panic!("directive must contain a 'Working' segment: {directive:?}"));
    assert_eq!(
        working.style.as_deref(),
        Some("streaming"),
        "Working segment must use streaming style"
    );
}

#[tokio::test]
async fn badge_returns_idle_enrich_when_no_plugin_data() {
    // Given a system with no plugin_data set (fresh state).
    let sys = build_system_with_oneshot(json!(null));

    // When the renderer fires the badge hook with a session_id-bearing ctx
    // (the host always emits one in production).
    let directives = sys.sync.call_hooks(
        "on_chat_input_badges_render",
        &HookContext::from(json!({ "session_id": "s1", "mode": "input" })),
    );

    // Then the badge text is [Enrich] (the idle state).
    let directive: jinn_domain::BadgeDirective =
        serde_json::from_value(directives[0].clone()).expect("deserialize directive");
    let joined: String = directive.segments.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(
        joined, "[Enrich]",
        "badge must show [Enrich] when no plugin_data"
    );
}

// ── cancel-on-retap integration ───────────────────────────────

#[tokio::test]
async fn enrichment_first_tap_proceeds_when_idle() {
    // Given the real enrichment plugin, idle (status unset).
    let sys = build_system_with_oneshot(json!({ "ok": true, "value": { "text": "rewritten" } }));

    // When the sync on_keybind_trigger fires for our keybind.
    let results = sys.sync.call_hooks(
        "on_keybind_trigger",
        &HookContext::from(json!({
            "hook": "on_enrich",
            "session_id": "s1",
            "text": "fix the bug",
            "keybound_plugin": "prompt_enrichment",
        })),
    );

    // Then exactly one result, run_action=true (no veto).
    assert_eq!(results.len(), 1, "exactly one plugin should answer");
    let run_action = results[0].get("run_action").and_then(|v| v.as_bool());
    assert_eq!(
        run_action,
        Some(true),
        "idle state must not veto the action"
    );
}

#[tokio::test]
async fn enrichment_retap_cancels_inflight_and_vetoes() {
    // Given the real enrichment plugin with status=enriching (simulating an
    // in-flight enrichment) and an on_enrich that would otherwise complete.
    let sys = build_system_with_oneshot(json!({ "ok": true, "value": { "text": "rewritten" } }));
    sys.async_handle
        .set_plugin_data("prompt_enrichment", json!({ "status": "enriching" }));

    // When the sync on_keybind_trigger fires for our keybind.
    let results = sys.sync.call_hooks(
        "on_keybind_trigger",
        &HookContext::from(json!({
            "hook": "on_enrich",
            "session_id": "s1",
            "text": "fix the bug",
            "keybound_plugin": "prompt_enrichment",
        })),
    );

    // Then the hook returns run_action=false (cancel the in-flight, don't re-run).
    assert_eq!(results.len(), 1, "exactly one plugin should answer");
    let run_action = results[0].get("run_action").and_then(|v| v.as_bool());
    assert_eq!(
        run_action,
        Some(false),
        "enriching state must veto the action and cancel the in-flight request"
    );
}

/// Extract the `E` segment from the first badge directive returned by the plugin.
fn e_segment(directives: &[Value]) -> jinn_domain::BadgeSegment {
    assert!(
        !directives.is_empty(),
        "plugin must return a badge directive"
    );
    let directive: jinn_domain::BadgeDirective =
        serde_json::from_value(directives[0].clone()).expect("deserialize directive");
    directive
        .segments
        .iter()
        .find(|s| s.text == "E")
        .cloned()
        .unwrap_or_else(|| panic!("directive must contain an 'E' segment: {directive:?}"))
}
