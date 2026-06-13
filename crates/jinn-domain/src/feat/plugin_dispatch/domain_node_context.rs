//! Domain context for Lua plugin LLM access.
//!
//! Provides `send_llm_request_cloned` so Lua scripts can call `ctx.llm()`
//! through the existing session infrastructure. Also provides `send_command`
//! for the controller to emit domain commands.

use std::collections::HashMap;
use std::sync::Arc;

use crate::feat::provider::protocol::command::CancelStream;
use error_stack::Report;
use parking_lot::Mutex;
use tokio::sync::oneshot;
use wherror::Error;

use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::chat_input::protocol::command::EnqueueUserMessage;
use crate::feat::context::assemble::AssemblyOverrides;
use crate::feat::session::chat_entry::ChatEntry;
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session::chat_session::SessionCoreEphemeral;
use crate::feat::session::model_selection::ModelSelection;
use crate::protocol::SessionId;

/// Error for domain context operations.
#[derive(Debug, Error)]
#[error(debug)]
pub struct DomainContextError;

type PendingResult = Arc<Mutex<HashMap<SessionId, oneshot::Sender<Result<String, String>>>>>;

/// Domain context for Lua plugin LLM access.
///
/// Provides:
/// - `send_command` - emit domain commands through the actor channel
/// - `send_llm_request_cloned` - clone a session and send an LLM request
#[derive(Clone, Debug)]
pub struct DomainNodeContext {
    /// Shared services for accessing the actor bus.
    services: Services,
    /// Shared application state.
    state: State,
    /// Maps session IDs to pending oneshot senders.
    ///
    /// The sender carries a `Result`: `Ok(text)` for a successful one-shot
    /// (assistant text) or `Err(message)` when the one-shot session ended in an
    /// error entry (e.g. provider connection failure).
    pending: PendingResult,
}

impl DomainNodeContext {
    /// Create a new domain context.
    pub fn new(services: Services, state: State) -> Self {
        Self {
            services,
            state,
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }


    /// Create a child session with the given parent, automation, and persistence flags.
    ///
    /// Returns the new session's ID.
    pub fn create_child_session(
        &self,
        parent_session_id: SessionId,
        automated: bool,
        persist: bool,
    ) -> SessionId {
        let mut session = ChatSessionState::default();
        session.core.parent_session = Some(parent_session_id.clone());
        session.core.is_automated = automated;
        session.core.persist = persist;

        // Inherit the parent session's model so the child can send to the LLM provider.
        if let Some(model) = self
            .state
            .read()
            .session
            .get(&parent_session_id)
            .map(|s| s.model().to_owned())
        {
            session.set_model(model);
        }

        let session_id = session.session_id().clone();
        self.state.write().session.insert(session);

        // Inherit attached-scoped plugin tools from the parent session.
        // When a plugin (like the judge) creates a child session, the child
        // needs the plugin's attached tools (e.g. judgment_passed/judgment_failed).
        self.register_inherited_tools(&parent_session_id, &session_id);

        session_id
    }

    /// Look up attached-scoped tools registered for the parent session and
    /// re-register them for the child session.
    fn register_inherited_tools(&self, parent_id: &SessionId, child_id: &SessionId) {
        let parent_tools = self
            .state
            .read()
            .context
            .session_tool_definitions
            .get(parent_id)
            .cloned();

        let Some(tools) = parent_tools else { return; };
        if tools.is_empty() { return; }

        // Write directly to state so tools are immediately visible
        // (works in tests without actor bus).
        self.state
            .write()
            .context
            .session_tool_definitions
            .entry(child_id.clone())
            .or_default()
            .extend(tools.into_iter());
    }

    /// Returns `true` if there is a pending oneshot for the given session ID.
    pub fn has_pending(&self, session_id: &SessionId) -> bool {
        self.pending.lock().contains_key(session_id)
    }

    /// Resolves a pending oneshot with the given outcome.
    ///
    /// `Ok(text)` fulfills a successful one-shot; `Err(message)` reports that the
    /// one-shot session ended in an error entry.
    pub fn resolve_completed(&self, session_id: &SessionId, outcome: Result<String, String>) {
        if let Some(tx) = self.pending.lock().remove(session_id) {
            let _ = tx.send(outcome);
        }
    }

    /// Inserts a pending oneshot sender for the given session ID.
    #[cfg(test)]
    pub fn insert_pending(
        &self,
        session_id: SessionId,
        tx: oneshot::Sender<Result<String, String>>,
    ) {
        self.pending.lock().insert(session_id, tx);
    }

    /// Send an LLM request using a cloned session and wait for the full response.
    ///
    /// Clones an existing session, giving the clone a new ID, `is_automated = true`,
    /// and `parent_session = Some(source)`. The clone inherits full history, profile,
    /// and tools from the source.
    ///
    /// # Errors
    ///
    /// Returns an error if the source session is not found or the oneshot is cancelled.
    pub async fn send_llm_request_cloned(
        &self,
        source_session_id: &SessionId,
        user_prompt: String,
        system_prompt: Option<String>,
        provider_id: Option<String>,
    ) -> Result<String, Report<DomainContextError>> {
        // 1. Read source session, clone it entirely
        let mut session = {
            let guard = self.state.read();
            guard
                .session
                .get(source_session_id)
                .cloned()
                .ok_or_else(|| Report::new(DomainContextError).attach("source session not found"))?
        };

        // 2. Build overrides
        let overrides = AssemblyOverrides {
            system_prompt,
            tool_definitions: Some(vec![]),
            skip_skills: true,
            skip_context_files: true,
        };

        // 3. Generate new session ID (clone must NOT share ID with source)
        session.core.session_id = SessionId::new();

        // 4. Mark as plugin session, reset ephemeral
        session.core.is_automated = true;
        session.core.ephemeral = SessionCoreEphemeral::default();
        session.core.assembly_overrides = Some(overrides);
        session.core.parent_session = Some(source_session_id.clone());

        // 5. Resolve model
        let model = provider_id.map_or_else(
            || session.core.profile.model.clone(),
            ModelSelection::from_single,
        );
        session.set_model(model);

        let session_id = session.session_id().clone();

        // 6. Insert into app state. Do NOT set_active — the cloned one-shot
        //    must stay invisible; the user's active chat view is unchanged.
        {
            let mut state = self.state.write();
            state.session.insert(session);
        }

        // 7. Create oneshot, enqueue, await
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(session_id.clone(), tx);

        let entry = ChatEntry::user(&user_prompt);
        self.services
            .bus
            .publish(EnqueueUserMessage {
                session_id: session_id.clone(),
                entry,
            })
            .await;

        match rx.await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(message)) => Err(Report::new(DomainContextError).attach(message)),
            Err(_) => {
                Err(Report::new(DomainContextError).attach("cloned plugin LLM request cancelled"))
            }
        }
    }

    /// Send a history-less one-shot LLM request, inheriting only the source session's
    /// provider+model. Unlike [`send_llm_request_cloned`], this builds a FRESH minimal
    /// session (default `SessionCore`, empty history) — no inherited chat history, profile
    /// skills, or tools. Used by plugin enrichment (`ctx.request("llm_oneshot", ...)`).
    ///
    /// The completion is resolved by the plugin-dispatch actor when this plugin session
    /// transitions to `Idle` (see `SessionPhaseChanged` handler).
    ///
    /// # Errors
    ///
    /// Returns an error if the source session is not found or the oneshot is cancelled.
    pub async fn send_llm_request_oneshot(
        &self,
        source_session_id: &SessionId,
        user_prompt: String,
        system_prompt: Option<String>,
        persist: bool,
        disable_tool_loop: bool,
        timeout_ms: u64,
    ) -> Result<String, Report<DomainContextError>> {
        // 1. Read source session ONLY for provider+model (no history clone).
        let provider_model = {
            let guard = self.state.read();
            let s = guard.session.get(source_session_id).ok_or_else(|| {
                Report::new(DomainContextError).attach("source session not found")
            })?;
            s.core.profile.model.clone()
        };

        // 2. Build a FRESH minimal session (default SessionCore, empty history).
        let mut session = ChatSessionState::default();

        // 3. History-less overrides: custom system prompt, no skills/context files.
        //    Tools branch on `disable_tool_loop`: an empty vec forces no tools and
        //    pairs with `set_tool_loop_disabled` (step 4a); `None` lets assembly
        //    inherit the full global catalog, same as a normal session.
        let tool_definitions = disable_tool_loop.then(Vec::new);
        let overrides = AssemblyOverrides {
            system_prompt,
            tool_definitions,
            skip_skills: true,
            skip_context_files: true,
        };

        // 4. New session ID; mark automated; inherit provider+model; record parent;
        //    honor the caller's persistence intent.
        session.core.session_id = SessionId::new();
        session.core.is_automated = true;
        session.core.persist = persist;
        session.core.ephemeral = SessionCoreEphemeral::default();
        session.core.assembly_overrides = Some(overrides);
        session.core.parent_session = Some(source_session_id.clone());
        session.set_model(provider_model);

        // 4a. When the caller disables the tool loop, set the machine flag so a
        //     model that hallucinates a tool-call-shaped turn transitions to
        //     `Idle` after the (empty) batch instead of looping `Sending ↔ Streaming`.
        if disable_tool_loop {
            session.set_tool_loop_disabled();
        }

        let session_id = session.session_id().clone();

        // 5. Insert into app state. Do NOT set_active — the one-shot must
        //    stay invisible; the user's active chat view is unchanged.
        {
            let mut state = self.state.write();
            state.session.insert(session);
        }

        // 6. Create oneshot, enqueue, await.
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(session_id.clone(), tx);

        let entry = ChatEntry::user(&user_prompt);
        self.services
            .bus
            .publish(EnqueueUserMessage {
                session_id: session_id.clone(),
                entry,
            })
            .await;

        // 7. Await with a bounded timeout. On expiry: hard-cancel the underlying
        //    session (no zombie stream burning provider tokens) and drop the pending
        //    entry so a later Idle transition can't resolve a dead receiver.
        match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), rx).await {
            Ok(Ok(Ok(response))) => Ok(response),
            Ok(Ok(Err(message))) => Err(Report::new(DomainContextError).attach(message)),
            Ok(Err(_)) => {
                Err(Report::new(DomainContextError).attach("one-shot LLM request cancelled"))
            }
            Err(_) => {
                self.pending.lock().remove(&session_id);
                self.services.bus.publish(CancelStream {
                    session_id: session_id.clone(),
                }).await;
                Err(Report::new(DomainContextError).attach(format!(
                    "one-shot LLM request timed out after {timeout_ms}ms"
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::services::bus_service::{BusAudit, BusService};
    use crate::common::services::test_services::TestServices;
    use crate::common::state::State;
    use crate::feat::session::chat_entry::ChatEntry;

    fn make_ctx() -> DomainNodeContext {
        let services = TestServices::builder().build();
        let state = State::new(AppState::default());
        DomainNodeContext::new(services, state)
    }

    fn make_ctx_with_audit() -> (DomainNodeContext, BusAudit) {
        let (bus, audit) = BusService::new_recording();
        let services = TestServices::builder().with_bus(bus).build();
        let state = State::new(AppState::default());
        (DomainNodeContext::new(services, state), audit)
    }

    #[rstest::rstest]
    fn has_pending_returns_false_when_empty() {
        let ctx = make_ctx();
        assert!(!ctx.has_pending(&SessionId::new()));
    }

    #[rstest::rstest]
    fn has_pending_returns_true_after_insert() {
        let ctx = make_ctx();
        let session_id = SessionId::new();
        let (tx, rx) = oneshot::channel();
        ctx.pending.lock().insert(session_id.clone(), tx);
        assert!(ctx.has_pending(&session_id));
        drop(rx);
    }

    #[rstest::rstest]
    fn resolve_completed_sends_response() {
        let ctx = make_ctx();
        let session_id = SessionId::new();
        let (tx, mut rx) = oneshot::channel();
        ctx.pending.lock().insert(session_id.clone(), tx);
        ctx.resolve_completed(&session_id, Ok("hello world".to_owned()));
        let result = rx.try_recv().expect("should have a value");
        assert_eq!(result, Ok("hello world".to_owned()));
    }

    #[rstest::rstest]
    fn resolve_completed_removes_pending() {
        let ctx = make_ctx();
        let session_id = SessionId::new();
        let (tx, _rx) = oneshot::channel();
        ctx.pending.lock().insert(session_id.clone(), tx);
        ctx.resolve_completed(&session_id, Ok("response".to_owned()));
        assert!(!ctx.has_pending(&session_id));
    }

    #[rstest::rstest]
    fn resolve_completed_ignores_unknown_session() {
        let ctx = make_ctx();
        ctx.resolve_completed(&SessionId::new(), Ok("response".to_owned()));
    }

    // ── send_llm_request_oneshot ────────────────────────────────��─────────
    //
    // Verifies the history-less one-shot path:
    //   - reads ONLY provider+model from the source session (no history clone)
    //   - builds a fresh session with a NEW id, is_automated=true
    //   - inherits the source's model on the new session
    //   - publishes exactly one EnqueueUserMessage for the new session
    //   - awaits a oneshot; resolve_completed fulfills it with the text

    fn seed_source_session(ctx: &DomainNodeContext, model: &str) -> SessionId {
        use crate::feat::session::profile::SessionProfile;
        let mut session = ChatSessionState::new_with_profile(SessionProfile {
            model: ModelSelection::Single(model.to_owned()),
            ..SessionProfile::default()
        });
        let id = SessionId::new();
        session.core.session_id = id.clone();
        // Give the source session some history; the one-shot must NOT inherit it.
        session.push_entry(ChatEntry::user("old user message from history"));
        session.push_entry(ChatEntry::assistant("old assistant message from history"));
        ctx.state.write().session.insert(session);
        id
    }

    #[tokio::test]
    async fn oneshot_inherits_provider_model_and_emits_enqueue() {
        use std::future::Future as _;

        let (ctx, audit) = make_ctx_with_audit();
        let source_id = seed_source_session(&ctx, "ollama/llama3");

        // Build the one-shot future but don't await it: poll once to drive it
        // up to its first await point (it publishes the EnqueueUserMessage
        // synchronously via bus.publish, then parks on the oneshot).
        let fut = ctx.send_llm_request_oneshot(
            &source_id,
            "rewrite me".to_owned(),
            Some("be concise".to_owned()),
            false,
            true,
            30_000,
        );
        futures::pin_mut!(fut);
        let waker = futures::task::noop_waker();
        let mut poll_cx = std::task::Context::from_waker(&waker);

        assert!(
            matches!(fut.as_mut().poll(&mut poll_cx), std::task::Poll::Pending,),
            "future must park on the oneshot after publishing the message"
        );

        // Exactly one EnqueueUserMessage should be published for the
        // NEW (plugin) session — not the source session.
        let msgs: Vec<EnqueueUserMessage> = audit.of_type::<EnqueueUserMessage>();
        assert_eq!(msgs.len(), 1, "expected exactly one EnqueueUserMessage");
        let new_session_id = msgs[0].session_id.clone();
        assert_ne!(
            new_session_id, source_id,
            "one-shot must create a fresh session, not reuse the source"
        );

        // The new plugin session inherits the source's provider+model, has no
        // history, is marked is_automated, and records the source as parent.
        let guard = ctx.state.read();
        let new = guard
            .session
            .get(&new_session_id)
            .expect("new session inserted");
        assert_eq!(
            new.core.profile.model,
            ModelSelection::Single("ollama/llama3".to_owned())
        );
        assert!(new.core.is_automated);
        assert_eq!(new.core.parent_session.as_ref(), Some(&source_id));
        assert_eq!(
            new.core
                .assembly_overrides
                .as_ref()
                .map(|o| &o.system_prompt),
            Some(&Some("be concise".to_owned())),
            "system prompt override must be carried through",
        );
        assert_eq!(
            new.core
                .assembly_overrides
                .as_ref()
                .map(|o| o.tool_definitions.as_deref()),
            Some(Some(&[][..])),
            "tool definitions override must be empty (None would inherit the full tool catalog)",
        );
        drop(guard);

        // The pending oneshot exists for the new session.
        assert!(ctx.has_pending(&new_session_id));

        // Resolve the oneshot — polling again should yield the text.
        ctx.resolve_completed(&new_session_id, Ok("rewritten!".to_owned()));
        match fut.as_mut().poll(&mut poll_cx) {
            std::task::Poll::Ready(Ok(text)) => assert_eq!(text, "rewritten!"),
            other => panic!("expected Ready(Ok), got {other:?}"),
        }
    }

    #[rstest::rstest]
    fn oneshot_fails_when_source_session_missing() {
        use std::future::Future as _;

        // No #[tokio::test] needed: the error returns before the first .await.
        let (ctx, _audit) = make_ctx_with_audit();
        let missing_id = SessionId::new();
        let fut =
            ctx.send_llm_request_oneshot(&missing_id, "x".to_owned(), None, false, true, 30_000);
        // Pin and poll once to drive to the error before any await point.
        futures::pin_mut!(fut);
        let waker = futures::task::noop_waker();
        let mut cx = std::task::Context::from_waker(&waker);

        match fut.poll(&mut cx) {
            std::task::Poll::Ready(Err(e)) => {
                let s = format!("{e:?}");
                assert!(s.contains("source session not found"), "got: {s}");
            }
            other => panic!("expected immediate error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn oneshot_resolves_err_as_request_failure() {
        // When a one-shot session errors (e.g. LLM connection refused), the session
        // actor pushes a ChatEntry::error and transitions to Idle. The dispatch actor
        // reads that error entry and resolves the pending oneshot as Err, which
        // surfaces to the plugin via the request envelope's `error` field.
        use std::future::Future as _;

        let (ctx, audit) = make_ctx_with_audit();
        let source_id = seed_source_session(&ctx, "ollama/llama3");

        let fut = ctx.send_llm_request_oneshot(
            &source_id,
            "rewrite me".to_owned(),
            None,
            false,
            true,
            30_000,
        );
        futures::pin_mut!(fut);
        let waker = futures::task::noop_waker();
        let mut poll_cx = std::task::Context::from_waker(&waker);

        // Drive to the await point (pending on the oneshot channel).
        assert!(matches!(
            fut.as_mut().poll(&mut poll_cx),
            std::task::Poll::Pending,
        ));

        // Discover the new session id from the published EnqueueUserMessage.
        let msgs: Vec<EnqueueUserMessage> = audit.of_type::<EnqueueUserMessage>();
        let new_session_id = msgs[0].session_id.clone();

        // Resolve as an error (simulating the dispatch actor reading an Error entry).
        ctx.resolve_completed(&new_session_id, Err("LLM stream error".to_owned()));
        match fut.as_mut().poll(&mut poll_cx) {
            std::task::Poll::Ready(Err(e)) => {
                let s = format!("{e:?}");
                assert!(s.contains("LLM stream error"), "got: {s}");
            }
            other => panic!("expected Ready(Err), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn oneshot_does_not_change_active_session() {
        // Given a source session set as the active view.
        let (ctx, _audit) = make_ctx_with_audit();
        let source_id = seed_source_session(&ctx, "ollama/llama3");
        ctx.state.write().session.set_active(source_id.clone());

        // When the one-shot runs (parking on its response channel).
        let fut = ctx.send_llm_request_oneshot(
            &source_id,
            "rewrite me".to_owned(),
            None,
            false,
            true,
            30_000,
        );
        futures::pin_mut!(fut);
        let waker = futures::task::noop_waker();
        let mut poll_cx = std::task::Context::from_waker(&waker);
        assert!(matches!(
            fut.as_mut().poll(&mut poll_cx),
            std::task::Poll::Pending,
        ));

        // Then the active session is still the source — the one-shot must
        // never steal the visible chat view.
        assert_eq!(
            ctx.state.read().session.active_session_id(),
            &source_id,
            "one-shot must not call set_active; the user's chat view is unchanged",
        );
    }

    #[tokio::test]
    async fn oneshot_request_default_persist_is_false() {
        // Given a source session and a one-shot call with persist=false (default).
        let (ctx, _audit) = make_ctx_with_audit();
        let source_id = seed_source_session(&ctx, "ollama/llama3");

        let fut = ctx.send_llm_request_oneshot(
            &source_id,
            "rewrite me".to_owned(),
            None,
            false,
            true,
            30_000,
        );
        futures::pin_mut!(fut);
        let waker = futures::task::noop_waker();
        let mut poll_cx = std::task::Context::from_waker(&waker);
        let _ = fut.as_mut().poll(&mut poll_cx);

        // Find the new session by scanning for the automated one with this parent.
        let guard = ctx.state.read();
        let new = guard
            .session
            .sessions()
            .values()
            .find(|s| s.core.is_automated && s.core.parent_session.as_ref() == Some(&source_id))
            .expect("one-shot session created");
        assert!(
            !new.core.persist,
            "persist=false request must produce a non-persistent session"
        );
    }

    #[tokio::test]
    async fn oneshot_request_persist_true_round_trips() {
        // Given a source session and a one-shot call with persist=true.
        let (ctx, _audit) = make_ctx_with_audit();
        let source_id = seed_source_session(&ctx, "ollama/llama3");

        let fut = ctx.send_llm_request_oneshot(
            &source_id,
            "rewrite me".to_owned(),
            None,
            true,
            true,
            30_000,
        );
        futures::pin_mut!(fut);
        let waker = futures::task::noop_waker();
        let mut poll_cx = std::task::Context::from_waker(&waker);
        let _ = fut.as_mut().poll(&mut poll_cx);

        let guard = ctx.state.read();
        let new = guard
            .session
            .sessions()
            .values()
            .find(|s| s.core.is_automated && s.core.parent_session.as_ref() == Some(&source_id))
            .expect("one-shot session created");
        assert!(
            new.core.persist,
            "persist=true request must produce a persistent session"
        );
    }

    // ── disable_tool_loop knob ─────────────────────────────────────────

    #[tokio::test]
    async fn disable_tool_loop_true_sets_flag_and_empty_tools() {
        // Given a source session.
        let (ctx, _audit) = make_ctx_with_audit();
        let source_id = seed_source_session(&ctx, "ollama/llama3");

        // When the one-shot runs with disable_tool_loop=true.
        let fut = ctx.send_llm_request_oneshot(
            &source_id,
            "rewrite me".to_owned(),
            None,
            false,
            true,
            30_000,
        );
        futures::pin_mut!(fut);
        let waker = futures::task::noop_waker();
        let mut poll_cx = std::task::Context::from_waker(&waker);
        let _ = fut.as_mut().poll(&mut poll_cx);

        // Then the new session has tool_loop_disabled set and an empty tool override.
        let guard = ctx.state.read();
        let new = guard
            .session
            .sessions()
            .values()
            .find(|s| s.core.is_automated && s.core.parent_session.as_ref() == Some(&source_id))
            .expect("one-shot session created");
        assert!(
            new.is_tool_loop_disabled(),
            "disable_tool_loop=true must set the machine flag"
        );
        assert_eq!(
            new.core
                .assembly_overrides
                .as_ref()
                .map(|o| o.tool_definitions.as_deref()),
            Some(Some(&[][..])),
            "disable_tool_loop=true must declare an empty tool set, not None"
        );
    }

    #[tokio::test]
    async fn disable_tool_loop_false_inherits_full_catalog() {
        // Given a source session.
        let (ctx, _audit) = make_ctx_with_audit();
        let source_id = seed_source_session(&ctx, "ollama/llama3");

        // When the one-shot runs with disable_tool_loop=false.
        let fut = ctx.send_llm_request_oneshot(
            &source_id,
            "rewrite me".to_owned(),
            None,
            false,
            false,
            30_000,
        );
        futures::pin_mut!(fut);
        let waker = futures::task::noop_waker();
        let mut poll_cx = std::task::Context::from_waker(&waker);
        let _ = fut.as_mut().poll(&mut poll_cx);

        // Then the new session does NOT set the flag and the tool override is None
        // (assembly will pull the full global catalog).
        let guard = ctx.state.read();
        let new = guard
            .session
            .sessions()
            .values()
            .find(|s| s.core.is_automated && s.core.parent_session.as_ref() == Some(&source_id))
            .expect("one-shot session created");
        assert!(
            !new.is_tool_loop_disabled(),
            "disable_tool_loop=false must leave the machine flag cleared"
        );
        assert_eq!(
            new.core
                .assembly_overrides
                .as_ref()
                .map(|o| &o.tool_definitions),
            Some(&None),
            "disable_tool_loop=false must leave tool override as None (inherit catalog)"
        );
    }

    // ── timeout_ms knob ───────────────────────────────────────────────
    //
    // The timeout uses tokio::time::timeout with the test runtime's auto-advance.
    // We never call resolve_completed, so the receiver never resolves; the
    // timeout must fire, hard-cancel the session, and return an error.

    #[tokio::test]
    async fn oneshot_timeout_cancels_session() {
        // Given a one-shot with a tiny timeout and no resolver.
        let (ctx, audit) = make_ctx_with_audit();
        let source_id = seed_source_session(&ctx, "ollama/llama3");

        let result = ctx
            .send_llm_request_oneshot(&source_id, "rewrite me".to_owned(), None, false, true, 1)
            .await;

        // Then the future returns an error (timeout).
        assert!(result.is_err(), "timeout must surface as an error");
        let msg = format!("{:?}", result.unwrap_err());
        assert!(
            msg.contains("timed out"),
            "error must mention the timeout, got: {msg}"
        );

        // And a CancelStream message was published for the one-shot session.
        let cancel_msgs: Vec<CancelStream> = audit.of_type::<CancelStream>();
        assert!(
            !cancel_msgs.is_empty(),
            "timeout must hard-cancel the underlying session via CancelStream"
        );
    }

    #[tokio::test]
    async fn oneshot_timeout_removes_pending() {
        // Given a one-shot with a tiny timeout and no resolver.
        let (ctx, _audit) = make_ctx_with_audit();
        let source_id = seed_source_session(&ctx, "ollama/llama3");

        // When the one-shot times out.
        let _ = ctx
            .send_llm_request_oneshot(&source_id, "rewrite me".to_owned(), None, false, true, 1)
            .await;

        // Then the pending entry is cleaned up (no leak).
        let still_pending = ctx.state.read().session.sessions().values().any(|s| {
            s.core.is_automated
                && s.core.parent_session.as_ref() == Some(&source_id)
                && ctx.has_pending(&s.core.session_id)
        });
        assert!(
            !still_pending,
            "timeout must remove the pending oneshot entry for the cancelled session"
        );
    }

    #[test]
    fn create_child_session_returns_unique_id() {
        // Given a domain context.
        let ctx = make_ctx();
        let parent_id = SessionId::new();

        // When creating a child session.
        let child_id = ctx.create_child_session(parent_id.clone(), true, true);

        // Then the child ID differs from the parent.
        assert_ne!(child_id, parent_id);
    }

    #[test]
    fn create_child_session_sets_parent_automated_persist_flags() {
        // Given a domain context.
        let ctx = make_ctx();
        let parent_id = SessionId::new();

        // When creating a child session with automated=true, persist=true.
        let child_id = ctx.create_child_session(parent_id.clone(), true, true);

        // Then the child session has the correct flags.
        let state = ctx.state.read();
        let child = state.session.get(&child_id).expect("child session exists");
        assert_eq!(child.core.parent_session.as_ref(), Some(&parent_id));
        assert!(child.core.is_automated);
        assert!(child.core.persist);
    }

    #[test]
    fn create_child_session_inserts_into_state() {
        // Given a domain context.
        let ctx = make_ctx();
        let parent_id = SessionId::new();

        // When creating a child session.
        let child_id = ctx.create_child_session(parent_id, false, false);

        // Then the session map contains the child.
        let state = ctx.state.read();
        assert!(state.session.contains(&child_id));
    }

    #[test]
    fn create_child_session_inherits_parent_model() {
        // Given a domain context and a parent session with a specific model.
        let ctx = make_ctx();
        let parent_id = SessionId::new();
        let mut parent = ChatSessionState::default();
        parent.core.session_id = parent_id.clone();
        parent.set_model(ModelSelection::Single("my-model".to_owned()));
        ctx.state.write().session.insert(parent);

        // When creating a child session.
        let child_id = ctx.create_child_session(parent_id.clone(), true, true);

        // Then the child inherits the parent's model.
        let state = ctx.state.read();
        let child = state.session.get(&child_id).expect("child session exists");
        assert_eq!(
            child.model(),
            &ModelSelection::Single("my-model".to_owned())
        );
    }

    #[test]
    fn create_child_session_uses_default_model_when_parent_not_found() {
        // Given a domain context with no parent session in state.
        let ctx = make_ctx();
        let orphan_parent = SessionId::new();

        // When creating a child session.
        let child_id = ctx.create_child_session(orphan_parent, true, true);

        // Then the child keeps the default model.
        let state = ctx.state.read();
        let child = state.session.get(&child_id).expect("child session exists");
        assert_eq!(child.model(), &ModelSelection::default());
    }

    #[test]
    fn create_child_session_inherits_parent_attached_tools() {
        // Given a parent session with attached-scoped tools.
        use jinn_provider::ToolDefinition;

        let ctx = make_ctx();
        let parent_id = SessionId::new();

        // Simulate the parent having attached-scoped tools registered.
        let tool_def = ToolDefinition {
            name: "judgment_passed".to_owned(),
            description: "test".to_owned(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
            prompt_snippet: None,
            prompt_guidelines: vec![],
            server_tool_type: None,
        };
        ctx.state
            .write()
            .context
            .session_tool_definitions
            .entry(parent_id.clone())
            .or_default()
            .insert("judgment_passed".to_owned(), tool_def);

        // When creating a child session.
        let child_id = ctx.create_child_session(parent_id.clone(), true, true);

        // Then the child session has the parent's attached tools registered.
        let state = ctx.state.read();
        let child_tools = state.context.session_tool_definitions.get(&child_id);
        assert!(
            child_tools.is_some(),
            "child session should have attached tools inherited from parent"
        );
        assert!(
            child_tools.unwrap().contains_key("judgment_passed"),
            "child should have inherited the judgment_passed tool"
        );
    }
}
