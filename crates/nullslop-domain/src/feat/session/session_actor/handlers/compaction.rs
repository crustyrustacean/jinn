//! Context compaction handlers — manage session context compaction lifecycle.
//!
//! Handles beginning and ending context compaction: transitioning session phase,
//! inserting compaction summary entries, marking gathered entries as ignored,
//! and draining queued messages that arrived during compaction.

use crate::common::actor::ActorContext;
use crate::protocol::{ChatEntry, ChatEntryId, ChatEntryKind};

use super::super::SessionPersistenceActor;
use crate::feat::compaction_actor::protocol::command::{BeginCompaction, EndCompaction};
use crate::feat::session::chat_session::SessionPhase;

impl SessionPersistenceActor {
    /// BeginCompaction: set phase to Compacting, push "Starting..." system entry,
    /// mark gathered entries as ignored, and persist.
    pub(in crate::feat::session::session_actor) async fn handle_begin_compaction(
        &self,
        payload: &BeginCompaction,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.begin_compacting(payload.gathered_indices.clone());
            session.push_entry(ChatEntry::system("Starting context compaction..."));
            if !payload.gathered_indices.is_empty() {
                session.mark_entries_ignored(&payload.gathered_indices);
            }
        }

        self.save_active_session(&payload.session_id).await;
    }

    /// EndCompaction: insert compaction entry or error entry, set phase to Idle,
    /// drain any queued messages, persist, and start a new turn if needed.
    ///
    /// Ignores the payload if the session is not currently in Compacting phase
    /// (e.g. compaction was cancelled while the LLM call was in flight).
    pub(in crate::feat::session::session_actor) async fn handle_end_compaction(
        &self,
        payload: &EndCompaction,
        ctx: &ActorContext,
    ) {
        let should_process_queue: bool;
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);

            // Guard: ignore stale EndCompaction if phase is no longer Compacting.
            if !matches!(session.phase(), SessionPhase::Compacting) {
                tracing::warn!(
                    session_id = ?payload.session_id,
                    current_phase = ?session.phase(),
                    "EndCompaction received but session is not compacting — ignoring"
                );
                return;
            }

            if let Some(result) = &payload.result {
                let compaction_entry = ChatEntry {
                    id: ChatEntryId::new(),
                    timestamp: jiff::Timestamp::now(),
                    kind: ChatEntryKind::Compaction {
                        summary: result.summary.clone(),
                        tokens_before: result.tokens_before,
                        entries_compacted: result.entries_compacted,
                        model_used: result.model_used.clone(),
                    },
                    pin_position: None,
                    ignored: false,
                };
                session.insert_entry_at(result.boundary_index, compaction_entry);
                session.push_entry(ChatEntry::system(format!(
                    "Context was compacted. {} messages were summarized.",
                    result.entries_compacted
                )));
            } else {
                let error_msg = payload.error.as_deref().unwrap_or("Unknown error");
                session.push_entry(ChatEntry::error(format!("Compaction failed: {error_msg}")));
            }
            session.finish_compacting();

            // Check if queue has items to process.
            should_process_queue = session.queue_len() > 0;
        }

        self.save_active_session(&payload.session_id).await;

        // If messages were queued during compaction, process the queue.
        if should_process_queue {
            self.process_queue(&payload.session_id, ctx).await;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::super::super::helpers::{test_actor, test_context};
    use crate::feat::session::chat_session::SessionPhase;
    use crate::protocol::{ChatEntry, ChatEntryKind, Command};

    #[tokio::test]
    async fn begin_compaction_sets_compacting_phase_and_pushes_system_entry() {
        // Given a session actor with a session.
        let actor = test_actor();
        let (_sink, _ctx) = test_context();
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        // When handling BeginCompaction.
        actor
            .handle_begin_compaction(
                &crate::feat::compaction_actor::protocol::command::BeginCompaction {
                    session_id: session_id.clone(),
                    gathered_indices: vec![0, 1],
                },
            )
            .await;

        // Then the session is in Compacting phase.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(matches!(session.phase(), SessionPhase::Compacting));
        assert!(!matches!(session.phase(), SessionPhase::Idle));
        drop(state);
    }

    #[tokio::test]
    async fn end_compaction_on_success_inserts_entry_and_resets_phase() {
        // Given a session actor with a session in Compacting phase.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("old message"));
            session.begin_compacting(vec![]);
            state.session.active_session_id().clone()
        };

        // When handling EndCompaction with a successful result.
        actor
            .handle_end_compaction(
                &crate::feat::compaction_actor::protocol::command::EndCompaction {
                    session_id: session_id.clone(),
                    result: Some(
                        crate::feat::compaction_actor::protocol::command::CompactionResult {
                            summary: "summarized".to_owned(),
                            entries_compacted: 1,
                            tokens_before: 100,
                            model_used: "test/model".to_owned(),
                            boundary_index: 1,
                        },
                    ),
                    error: None,
                },
                &ctx,
            )
            .await;

        // Then the session is idle and has the compaction entry.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(matches!(session.phase(), SessionPhase::Idle));
        // The history should have: user entry, compaction entry, system entry.
        assert!(session.history().iter().any(ChatEntry::is_compaction));
        assert!(session.history().iter().any(
            |e| matches!(&e.kind, ChatEntryKind::System(t) if t.contains("Context was compacted"))
        ));
    }

    #[tokio::test]
    async fn end_compaction_on_failure_pushes_error_entry_and_resets_phase() {
        // Given a session actor with a session in Compacting phase.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            state.active_session_mut().begin_compacting(vec![]);
            state.session.active_session_id().clone()
        };

        // When handling EndCompaction with an error.
        actor
            .handle_end_compaction(
                &crate::feat::compaction_actor::protocol::command::EndCompaction {
                    session_id: session_id.clone(),
                    result: None,
                    error: Some("LLM call failed".to_owned()),
                },
                &ctx,
            )
            .await;

        // Then the session is idle and has an error entry.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(matches!(session.phase(), SessionPhase::Idle));
        let last = session.history().last().expect("has an entry");
        assert!(
            matches!(&last.kind, ChatEntryKind::Error(msg) if msg.contains("Compaction failed"))
        );
    }

    #[tokio::test]
    async fn end_compaction_drains_queued_messages_on_success() {
        // Given a session in Compacting phase with a queued user message.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(ChatEntry::user("queued during compaction")));
            session.begin_compacting(vec![]);
            state.session.active_session_id().clone()
        };

        // When handling EndCompaction with a successful result.
        actor
            .handle_end_compaction(
                &crate::feat::compaction_actor::protocol::command::EndCompaction {
                    session_id: session_id.clone(),
                    result: Some(
                        crate::feat::compaction_actor::protocol::command::CompactionResult {
                            summary: "summarized".to_owned(),
                            entries_compacted: 1,
                            tokens_before: 100,
                            model_used: "test/model".to_owned(),
                            boundary_index: 0,
                        },
                    ),
                    error: None,
                },
                &ctx,
            )
            .await;

        // Then AssemblePrompt was emitted for the queued message.
        let commands = sink.commands();
        let has_assemble = commands
            .iter()
            .any(|c| matches!(c, Command::AssemblePrompt(_)));
        assert!(
            has_assemble,
            "expected AssemblePrompt command for queued message"
        );
    }

    #[tokio::test]
    async fn end_compaction_drains_queued_messages_on_failure() {
        // Given a session in Compacting phase with a queued user message.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(ChatEntry::user("queued during compaction")));
            session.begin_compacting(vec![]);
            state.session.active_session_id().clone()
        };

        // When handling EndCompaction with a failure result.
        actor
            .handle_end_compaction(
                &crate::feat::compaction_actor::protocol::command::EndCompaction {
                    session_id: session_id.clone(),
                    result: None,
                    error: Some("LLM call failed".to_owned()),
                },
                &ctx,
            )
            .await;

        // Then AssemblePrompt was emitted for the queued message.
        let commands = sink.commands();
        let has_assemble = commands
            .iter()
            .any(|c| matches!(c, Command::AssemblePrompt(_)));
        assert!(
            has_assemble,
            "expected AssemblePrompt command for queued message after failure"
        );
    }

    #[tokio::test]
    async fn end_compaction_with_empty_queue_does_not_emit_assemble_prompt() {
        // Given a session in Compacting phase with no queued messages.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            state.active_session_mut().begin_compacting(vec![]);
            state.session.active_session_id().clone()
        };

        // When handling EndCompaction with a successful result.
        actor
            .handle_end_compaction(
                &crate::feat::compaction_actor::protocol::command::EndCompaction {
                    session_id: session_id.clone(),
                    result: Some(
                        crate::feat::compaction_actor::protocol::command::CompactionResult {
                            summary: "summarized".to_owned(),
                            entries_compacted: 1,
                            tokens_before: 100,
                            model_used: "test/model".to_owned(),
                            boundary_index: 0,
                        },
                    ),
                    error: None,
                },
                &ctx,
            )
            .await;

        // Then no AssemblePrompt command was emitted.
        let commands = sink.commands();
        let has_assemble = commands
            .iter()
            .any(|c| matches!(c, Command::AssemblePrompt(_)));
        assert!(
            !has_assemble,
            "expected no AssemblePrompt when queue is empty"
        );
    }

    #[tokio::test]
    async fn end_compaction_drain_leaves_session_in_sending_state() {
        // Given a session in Compacting phase with a queued user message.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(ChatEntry::user("queued during compaction")));
            session.begin_compacting(vec![]);
            state.session.active_session_id().clone()
        };

        // When handling EndCompaction with a successful result.
        actor
            .handle_end_compaction(
                &crate::feat::compaction_actor::protocol::command::EndCompaction {
                    session_id: session_id.clone(),
                    result: Some(
                        crate::feat::compaction_actor::protocol::command::CompactionResult {
                            summary: "summarized".to_owned(),
                            entries_compacted: 1,
                            tokens_before: 100,
                            model_used: "test/model".to_owned(),
                            boundary_index: 0,
                        },
                    ),
                    error: None,
                },
                &ctx,
            )
            .await;

        // Then the session is in Sending state (start_turn_from_queued called begin_sending).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(matches!(session.phase(), SessionPhase::Sending));
    }

    #[tokio::test]
    async fn end_compaction_ignored_when_not_compacting() {
        // Given a session in Idle phase (e.g. compaction was cancelled).
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };
        // Session is Idle by default.

        // When handling EndCompaction with a successful result.
        actor
            .handle_end_compaction(
                &crate::feat::compaction_actor::protocol::command::EndCompaction {
                    session_id: session_id.clone(),
                    result: Some(
                        crate::feat::compaction_actor::protocol::command::CompactionResult {
                            summary: "summarized".to_owned(),
                            entries_compacted: 1,
                            tokens_before: 100,
                            model_used: "test/model".to_owned(),
                            boundary_index: 0,
                        },
                    ),
                    error: None,
                },
                &ctx,
            )
            .await;

        // Then no compaction entry was inserted and the session is still Idle.
        let state = actor.state.read();
        let session = state
            .session
            .sessions()
            .get(&session_id)
            .expect("session exists");
        assert_eq!(session.phase(), SessionPhase::Idle);
        assert!(!session.history().iter().any(ChatEntry::is_compaction));
    }
}
