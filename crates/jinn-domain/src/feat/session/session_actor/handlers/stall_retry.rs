//! Stall-retry handler — re-dispatches a turn whose LLM stream went silent.
//!
//! See [`SessionPersistenceActor::on_retry_stalled_session`]. The
//! `stall-watchdog` plugin detects silence on an in-flight provider stream
//! and pushes a mirrored `RestartStalledStream`, which the plugin
//! coordinator translates into
//! [`RetryStalledSession`](crate::feat::session::protocol::retry_stalled_session::RetryStalledSession).
//! A hung stream is treated like a hard provider error: partial streaming
//! entries are discarded and the turn is re-dispatched.

use crate::common::actor_deps::BusPublish;
use crate::feat::session::phase_machine::PhaseKind;
use crate::feat::session::protocol::retry_stalled_session::RetryStalledSession;
use crate::protocol::ChatEntry;

use super::super::SessionPersistenceActor;

impl SessionPersistenceActor {
    /// Re-dispatch a stalled turn: discard partial streaming entries, push a
    /// system marker, and re-send the existing history.
    ///
    /// The guard is *in-flight-stream*, not elapsed time: the handler acts
    /// only when the phase is `Sending`/`Streaming` **and**
    /// `stream_dispatched_at` is set — i.e. an LLM request is genuinely in
    /// flight. That timestamp is set at dispatch and cleared when the
    /// generation's `StreamCompleted` is consumed, so:
    ///
    /// - a stream that self-resolved between the plugin's trip and this
    ///   handler running has a `None` timestamp → no-op (the self-resolved
    ///   race is closed by construction, not by timestamp comparison);
    /// - a session waiting on a tool batch has a `None` timestamp (the
    ///   generation completed with `ToolUse` before tools dispatch) → no-op:
    ///   a restart during tool execution is structurally impossible, even if
    ///   a guest misfires.
    pub(in crate::feat::session::session_actor) async fn on_retry_stalled_session(
        &self,
        payload: &RetryStalledSession,
    ) {
        // Push the retry marker and discard partial streaming entries — but
        // only while a stream is genuinely in flight for this session.
        let marker = ChatEntry::system(format!(
            "\u{21bb} LLM stream stalled, retrying (attempt {} of {})\u{2026}",
            payload.attempt, payload.max_restarts
        ));
        let acted = self.state.with_session(&self.cap, |view| {
            let session = view.session.map().get_or_create(&payload.session_id);
            if matches!(session.phase(), PhaseKind::Sending | PhaseKind::Streaming)
                && session.core.ephemeral.stream_dispatched_at.is_some()
            {
                let removed = session.reset_streaming_entries_for_retry();
                // Partial tool calls left by a starved/errored stream
                // must be excluded from the retried request, otherwise
                // the next provider call carries structurally invalid
                // (truncated-arguments) entries. Mirrors the `Canceled`
                // path.
                let excluded = session.force_exclude_dangling_tool_calls();
                tracing::warn!(
                    session_id = %payload.session_id,
                    removed_entries = removed,
                    excluded_dangling = excluded.len(),
                    "retrying stalled turn"
                );
                session.push_entry(marker.clone());
                // The re-dispatch below emits a fresh `SendToLlmProvider`,
                // which the plugin host forwards as `stream_start` — the
                // watchdog re-arms for the new generation automatically.
                true
            } else {
                false
            }
        });

        if !acted {
            return;
        }

        // Phase is already Streaming (we didn't change it above); emit a
        // no-op-safe phase-changed event for consistency with other dispatch
        // paths, then re-send the assembled history.
        super::super::helpers::emit_history_appended(self.bus(), &payload.session_id).await;

        // Re-dispatch the existing history (no new user entry). This mirrors
        // `EnqueueResumeTurn`: assemble the prompt, resolve the model, transition
        // to Streaming, and emit `SendToLlmProvider`.
        self.resolve_model_and_dispatch(&payload.session_id).await;
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
    use super::super::super::helpers::test_actor_recording;
    use crate::common::services::BusAudit;
    use crate::feat::provider::protocol::command::SendToLlmProvider;

    use crate::feat::session::protocol::retry_stalled_session::RetryStalledSession;
    use crate::feat::session::session_actor::SessionPersistenceActor;
    use crate::protocol::ChatEntryKind;

    /// A session in `Streaming` with a partial assistant entry, a dangling
    /// partial tool call slot free, and an in-flight stream generation
    /// registered — the exact shape a stalled stream presents.
    async fn stall_setup() -> (SessionPersistenceActor, BusAudit, RetryStalledSession) {
        let (actor, audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write_test_no_cap();
            let session = state.active_session_mut();
            session.begin_streaming();
            // A partial assistant entry created via the streaming path so it
            // registers a streaming index and is discarded on retry.
            session
                .append_stream_token("partial", jiff::Timestamp::now())
                .expect("append first token");
            // Register the in-flight stream generation — the guard's source
            // of truth.
            session.core.ephemeral.stream_dispatched_at = Some(jiff::Timestamp::now());
            state.session.active_session_id().clone()
        };
        (
            actor,
            audit,
            RetryStalledSession {
                session_id,
                attempt: 2,
                max_restarts: 3,
            },
        )
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handler_discards_partial_entries_and_redispatches() {
        // Given a stalled Streaming session holding a partial assistant entry
        // and an in-flight stream generation.
        let (actor, audit, payload) = stall_setup().await;
        let session_id = payload.session_id.clone();

        // When the retry handler runs.
        actor.on_retry_stalled_session(&payload).await;

        // Then the partial assistant entry is gone.
        {
            let state = actor.state.read();
            let session = state.session.get(&session_id).expect("session exists");
            let has_partial = session
                .core
                .history
                .iter()
                .any(|e| matches!(e.kind, ChatEntryKind::Assistant(ref t) if t == "partial"));
            assert!(!has_partial, "partial assistant entry must be discarded");
            // And a retry system marker was pushed, naming the attempt and
            // budget reported by the watchdog.
            assert!(
                session.core.history.iter().any(|e| matches!(
                    e.kind,
                    ChatEntryKind::System(ref t)
                        if t.contains("stalled")
                            && t.contains("attempt 2 of 3")
                )),
                "a retry system marker naming the attempt must be pushed"
            );
        }
        // And SendToLlmProvider was emitted to re-dispatch the turn.
        let sent = audit.of_type::<SendToLlmProvider>();
        assert!(
            sent.iter().any(|s| s.session_id == session_id),
            "SendToLlmProvider must be re-emitted to re-dispatch the turn"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handler_noops_when_no_stream_is_in_flight() {
        // Given a session whose stream generation was consumed between the
        // plugin's trip and this handler running (the self-resolved shape:
        // `StreamCompleted` cleared `stream_dispatched_at`).
        let (actor, _audit, payload) = stall_setup().await;
        let session_id = payload.session_id.clone();
        {
            let mut state = actor.state.write_test_no_cap();
            let session = state.active_session_mut();
            session.core.ephemeral.stream_dispatched_at = None;
        }

        // When the retry handler runs.
        actor.on_retry_stalled_session(&payload).await;

        // Then nothing was discarded and no marker was pushed: the partial
        // assistant entry is still present.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            session.core.history.iter().any(|e| matches!(
                e.kind, ChatEntryKind::Assistant(ref t) if t == "partial"
            )),
            "a self-resolved stream must not be discarded"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handler_noops_when_session_is_idle() {
        // Given an Idle session with a stale guard value (defensive: both a
        // finished turn and a tool-batch wait present `None`, but the guard
        // must not rely on phase alone either).
        let (actor, _audit, payload) = stall_setup().await;
        let session_id = payload.session_id.clone();
        {
            use crate::feat::session::phase_machine::PhaseTransitions;
            let mut state = actor.state.write_test_no_cap();
            let session = state.active_session_mut();
            let _ = session
                .core
                .ephemeral
                .machine
                .on_stream_completed_finished();
        }

        // When the retry handler runs.
        actor.on_retry_stalled_session(&payload).await;

        // Then nothing was discarded: history is untouched.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            session.core.history.iter().any(|e| matches!(
                e.kind, ChatEntryKind::Assistant(ref t) if t == "partial"
            )),
            "an idle session must not be restarted"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn handler_excludes_dangling_partial_tool_call() {
        // Given a stalled Streaming session holding a partial (dangling)
        // tool call with no matching ToolResult.
        let (actor, _audit, payload) = stall_setup().await;
        let session_id = payload.session_id.clone();
        {
            let mut state = actor.state.write_test_no_cap();
            let session = state.active_session_mut();
            let now = jiff::Timestamp::now();
            session.begin_tool_call(0, "tc-partial", "bash", now);
            session
                .append_tool_call_delta(0, "{\"command\":\"cd /mnt")
                .expect("append partial delta");
        }

        // When the retry handler runs.
        actor.on_retry_stalled_session(&payload).await;

        // Then the partial tool call entry is no longer included in the
        // request context — it is marked ForcedExclude.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let has_active_partial = session.core.history.iter().any(|e| {
            matches!(&e.kind, ChatEntryKind::ToolCall { id, .. } if id == "tc-partial")
                && !matches!(
                    e.context_override(),
                    crate::protocol::ContextOverride::ForcedExclude
                )
        });
        assert!(
            !has_active_partial,
            "dangling partial tool call must be excluded from the retried request"
        );
    }
}
