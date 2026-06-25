//! Stall-retry handler — re-dispatches a turn whose history has gone silent.
//!
//! See [`SessionPersistenceActor::on_retry_stalled_session`]. The stall watchdog
//! publishes [`RetryStalledSession`](crate::feat::session::protocol::retry_stalled_session::RetryStalledSession)
//! when a session in `Sending`/`Streaming` has had no chat-history activity for
//! longer than `history_stall_timeout_secs`. A hung session is treated like a
//! hard provider error: partial streaming entries are discarded and the turn is
//! re-dispatched.

use crate::common::actor_deps::BusPublish;
use crate::feat::session::phase_machine::PhaseKind;
use crate::feat::session::protocol::retry_stalled_session::RetryStalledSession;
use crate::protocol::ChatEntry;

use super::super::SessionPersistenceActor;

impl SessionPersistenceActor {
    /// Re-dispatch a stalled turn: discard partial streaming entries, push a
    /// system marker, and re-send the existing history.
    ///
    /// Guards against a self-resolved stream: between the watchdog publishing
    /// `RetryStalledSession` and this handler running, a token may have landed
    /// and advanced `last_history_activity_at`. We re-check the phase (which
    /// only a still-active turn keeps in `Sending`/`Streaming`) and, under the
    /// write lock, that the activity timestamp is still stale before we discard
    /// anything. A late-arriving token that bumped the timestamp causes a no-op.
    pub(in crate::feat::session::session_actor) async fn on_retry_stalled_session(
        &self,
        payload: &RetryStalledSession,
    ) {
        let timeout_secs = self
            .services
            .user_preferences_storage
            .read()
            .history_stall_timeout_secs;
        let now = jiff::Timestamp::now();

        // Push the retry marker and discard partial streaming entries — but only
        // if the session is genuinely still stalled. A token that landed between
        // the watchdog's publish and now makes this a no-op.
        let marker = ChatEntry::system("\u{21bb} LLM stream stalled, retrying\u{2026}");
        let acted = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            if matches!(session.phase(), PhaseKind::Sending | PhaseKind::Streaming) {
                let elapsed_secs = now
                    .since(session.core.last_history_activity_at)
                    .map_or(i64::MAX, |s| s.get_seconds().max(0));
                if elapsed_secs >= timeout_secs as i64 {
                    let removed = session.reset_streaming_entries_for_retry();
                    tracing::warn!(
                        session_id = %payload.session_id,
                        removed_entries = removed,
                        "retrying stalled turn"
                    );
                    session.push_entry(marker.clone());
                    // `begin_streaming` re-seeds last_history_activity_at so the
                    // watchdog's next tick starts a fresh window for this retry.
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };

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
    use std::time::Duration;

    async fn stall_setup() -> (SessionPersistenceActor, BusAudit, RetryStalledSession) {
        let (actor, audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            // A partial assistant entry created via the streaming path so it
            // registers a streaming index and is discarded on retry.
            session
                .append_stream_token("partial", jiff::Timestamp::now())
                .expect("append first token");
            // Age the activity timestamp past the stall window.
            {
                let ts = jiff::Timestamp::now();
                session.core.last_history_activity_at =
                    ts.checked_sub(Duration::from_mins(2)).unwrap();
            }
            state.session.active_session_id().clone()
        };
        // Force a tight stall window so the re-check trips.
        {
            let mut prefs = actor.services.user_preferences_storage.read();
            prefs.history_stall_timeout_secs = 1;
            actor
                .services
                .user_preferences_storage
                .save(&prefs)
                .expect("save prefs");
        }
        (actor, audit, RetryStalledSession { session_id })
    }

    #[tokio::test]
    async fn handler_discards_partial_entries_and_redispatches() {
        // Given a stalled Streaming session holding a partial assistant entry.
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
            // And a retry system marker was pushed.
            assert!(
                session.core.history.iter().any(|e| matches!(
                    e.kind,
                    ChatEntryKind::System(ref t) if t.contains("stalled")
                )),
                "a retry system marker must be pushed"
            );
        }
        // And SendToLlmProvider was emitted to re-dispatch the turn.
        let sent = audit.of_type::<SendToLlmProvider>();
        assert!(
            sent.iter().any(|s| s.session_id == session_id),
            "SendToLlmProvider must be re-emitted to re-dispatch the turn"
        );
    }

    #[tokio::test]
    async fn handler_noops_when_timestamp_advanced_since_publish() {
        // Given a session that self-resolved between publish and handle: its
        // activity timestamp is now recent.
        let (actor, _audit, payload) = stall_setup().await;
        let session_id = payload.session_id.clone();
        {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.core.last_history_activity_at = jiff::Timestamp::now();
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
}
