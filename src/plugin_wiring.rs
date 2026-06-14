//! Plugin wiring — command dispatcher.
//!
//! Maps plugin command names (strings) to typed domain messages by delegating
//! to [`jinn_domain::common::plugin_bridge::dispatch_verb`]. Each verb's
//! Lua→domain translation lives on the domain message itself via the
//! `TryFromLua` trait in `plugin_bridge`.
//!
//! This module is a thin caller: it builds a [`CmdCtx`], asks the domain to
//! dispatch, and forwards the resulting [`BridgeClosure`] to the [`Bridge`]
//! (the same live channel the rest of the app uses).

use std::sync::Arc;

use jinn_domain::common::bridge::Bridge;
use jinn_domain::common::plugin_bridge::{CmdCtx, dispatch_verb};
use jinn_domain::feat::plugin_system::PluginCommand;

/// Dispatch a plugin command to the appropriate domain action.
///
/// Delegates verb matching and Lua→message translation to [`dispatch_verb`].
/// On success the returned closure is forwarded to the actor system via
/// [`Bridge::send`]. Unknown verbs and translation failures are logged and
/// dropped.
pub fn handle_plugin_command(cmd: PluginCommand, bridge: &Bridge) {
    tracing::debug!(
        plugin = cmd.plugin_name,
        verb = cmd.name,
        "plugin command dispatched"
    );

    let ctx = CmdCtx {
        plugin_name: cmd.plugin_name.clone(),
        verb: cmd.name.clone(),
    };

    match dispatch_verb(&cmd.name, ctx, cmd.data) {
        Some(closure) => {
            let _ = bridge.send(closure);
        }
        None => tracing::warn!(
            plugin = cmd.plugin_name,
            verb = cmd.name,
            "unknown plugin verb"
        ),
    }
}

/// Build a command dispatcher closure for the plugin system.
///
/// The returned closure captures the [`Bridge`] and routes plugin commands
/// through it to the kameo bus.
pub fn build_command_dispatcher(bridge: Bridge) -> Arc<dyn Fn(PluginCommand) + Send + Sync> {
    Arc::new(move |cmd: PluginCommand| {
        handle_plugin_command(cmd, &bridge);
    })
}

// ─── Request handler (for ctx.request from Lua) ────────────────────

/**
 * Handle a request from an async hook's `ctx.request(name, data)` call.
 *
 * Returns a result envelope: `{ ok: true, value }` on success, or
 * `{ ok: false, error }` on any failure (LLM error, malformed payload,
 * unknown request name).
 */
pub async fn handle_plugin_request(
    name: &str,
    data: &serde_json::Value,
    domain_ctx: &jinn_domain::feat::plugin_dispatch::DomainNodeContext,
) -> serde_json::Value {
    match name {
        "llm_oneshot" => {
            // History-less one-shot LLM request: inherits only the source session's
            // provider+model. Request shape:
            //   { session_id, system: Option<String>, prompt: String, persist: Option<bool> }
            // persist defaults to false — one-shots are transient unless the caller
            // explicitly asks to keep them (e.g. a judge run).
            #[derive(serde::Deserialize)]
            struct LlmOneshotPayload {
                session_id: jinn_domain::protocol::SessionId,
                system: Option<String>,
                prompt: String,
                #[serde(default)]
                persist: Option<bool>,
                // Whether the one-shot session is immune to tool-call loops.
                // true  -> empty tool definitions + tool_loop_disabled set
                // false -> inherit the full global tool catalog (default)
                #[serde(default)]
                disable_tool_loop: Option<bool>,
                // Hard timeout for the one-shot in milliseconds.
                // On expiry the underlying session is hard-cancelled (CancelStream)
                // and the await returns an error. Defaults to 30000.
                #[serde(default)]
                timeout_ms: Option<u64>,
            }
            match serde_json::from_value::<LlmOneshotPayload>(data.clone()) {
                Ok(p) => match domain_ctx
                    .send_llm_request_oneshot(
                        &p.session_id,
                        p.prompt,
                        p.system,
                        p.persist.unwrap_or(false),
                        p.disable_tool_loop.unwrap_or(false),
                        p.timeout_ms.unwrap_or(30_000),
                    )
                    .await
                {
                    Ok(text) => request_ok(serde_json::json!({ "text": text })),
                    Err(e) => {
                        tracing::warn!(error = %e, "llm_oneshot request failed");
                        request_err(format_args!("{e:?}"))
                    }
                },
                Err(e) => {
                    tracing::warn!(error = %e, "llm_oneshot malformed payload");
                    request_err(e)
                }
            }
        }
        "llm" => {
            // Full-context LLM (future use): not wired in this phase.
            tracing::warn!(name, "full-context llm request handler not yet wired");
            request_err("full-context llm request handler not yet wired")
        }
        _ => {
            tracing::warn!(name, "unknown plugin request");
            request_err(format_args!("unknown request: {name}"))
        }
    }
}

/// Wrap a success value in the `ctx.request` result envelope.
fn request_ok(value: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "ok": true, "value": value })
}

/// Wrap an error in the `ctx.request` result envelope.
fn request_err(error: impl std::fmt::Display) -> serde_json::Value {
    serde_json::json!({ "ok": false, "error": error.to_string() })
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::*;
    use jinn_domain::common::app_state::AppState;
    use jinn_domain::common::services::test_services::TestServices;
    use jinn_domain::common::state::State;
    use jinn_domain::feat::plugin_dispatch::DomainNodeContext;
    use jinn_domain::feat::session::model_selection::ModelSelection;
    use jinn_domain::feat::session::{ChatSessionState, SessionProfile};
    use jinn_domain::protocol::SessionId;
    use serde_json::json;
    use std::task::Poll;

    /// Build a `DomainNodeContext` over `TestServices` and seed a source
    /// session so `send_llm_request_oneshot` can read the source's provider+model.
    ///
    /// Returns the context, the source session id, and a cloned handle to the
    /// shared `State` (the binary crate cannot reach the private `ctx.state`
    /// field the way the in-module tests do, so we keep a clone — `State` shares
    /// its inner `Arc<RwLock<AppState>>`, so the clone observes every write the
    /// one-shot makes, including the inserted child session).
    fn make_ctx_with_source_session(model: &str) -> (DomainNodeContext, SessionId, State) {
        let services = TestServices::builder().build();
        let state = State::new(AppState::default());
        let source_id = SessionId::new();
        {
            let mut session = ChatSessionState::new_with_profile(SessionProfile {
                model: ModelSelection::Single(model.to_owned()),
                ..SessionProfile::default()
            });
            session.core.session_id = source_id.clone();
            state.write().session.insert(session);
        }
        // Clone BEFORE move into DomainNodeContext — both handles share one Arc.
        let state_handle = state.clone();
        let ctx = DomainNodeContext::new(services, state);
        (ctx, source_id, state_handle)
    }

    #[tokio::test]
    async fn llm_oneshot_success_returns_ok_value_envelope() {
        // Given a DomainNodeContext with a seeded source session.
        let (ctx, source_id, state) = make_ctx_with_source_session("ollama/llama3");
        let payload = json!({
            "session_id": source_id,
            "system": "be concise",
            "prompt": "rewrite me",
            "disable_tool_loop": true,
        });

        // When running the handler as a task (its bus.publish awaits a real
        // actor bus, so it needs the tokio runtime driving the test).
        let ctx_for_task = ctx.clone();
        let task = tokio::spawn(async move {
            handle_plugin_request("llm_oneshot", &payload, &ctx_for_task).await
        });

        // The one-shot creates a NEW child session parented at the source, then
        // parks on a oneshot. Give the bus a moment to publish + insert, then
        // find the child via the shared state handle and resolve it.
        let child_id = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(id) = state
                    .read()
                    .session
                    .iter()
                    .find(|(_, s)| s.core.parent_session.as_ref() == Some(&source_id))
                    .map(|(id, _)| id.clone())
                {
                    return id;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("one-shot must insert a child session parented at the source");
        assert!(
            ctx.has_pending(&child_id),
            "child must have a pending oneshot"
        );
        ctx.resolve_completed(&child_id, Ok("enriched".to_owned()));

        // Then the result envelope carries the resolved text.
        let value = task.await.expect("handler task panicked");
        assert_eq!(
            value,
            json!({ "ok": true, "value": { "text": "enriched" } })
        );
    }

    #[test]
    fn llm_oneshot_missing_source_session_returns_ok_false() {
        // Given a DomainNodeContext with NO seeded source session.
        let services = TestServices::builder().build();
        let ctx = DomainNodeContext::new(services, State::new(AppState::default()));
        let payload = json!({
            "session_id": SessionId::new(),
            "prompt": "rewrite me",
        });

        // When calling handle_plugin_request and polling once.
        // (Source-missing errors synchronously, before the first await.)
        let fut = handle_plugin_request("llm_oneshot", &payload, &ctx);
        let mut fut = std::pin::pin!(fut);
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(&waker);

        // Then it returns Ready with an error envelope.
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(value) => {
                assert_eq!(value["ok"], json!(false), "must be an error envelope");
                let err = value["error"].as_str().expect("error string");
                assert!(
                    err.contains("source session not found"),
                    "error should mention source session not found, got: {err}"
                );
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn llm_oneshot_malformed_payload_returns_ok_false() {
        // Given a DomainNodeContext (contents irrelevant; serde fails first).
        let services = TestServices::builder().build();
        let ctx = DomainNodeContext::new(services, State::new(AppState::default()));

        // When calling handle_plugin_request with a payload missing `prompt`.
        let result =
            handle_plugin_request("llm_oneshot", &json!({ "message": "wrong shape" }), &ctx).await;

        // Then the envelope is an error carrying the serde message.
        assert_eq!(result["ok"], json!(false), "must be an error envelope");
        assert!(
            result["error"].as_str().is_some(),
            "error field must be a string"
        );
    }
}
