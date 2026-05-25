//! Context compaction handlers — manage session context compaction lifecycle.
//!
//! Handles beginning and ending context compaction: transitioning session phase,
//! inserting compaction summary entries, marking gathered entries as ignored,
//! and draining queued messages that arrived during compaction.

use crate::common::actor::ActorContext;
use crate::protocol::{ChatEntry, ChatEntryId, ChatEntryKind, ContextOverride};

use super::super::SessionPersistenceActor;
use crate::feat::compaction_actor::protocol::command::{BeginCompaction, EndCompaction};
use crate::feat::session::chat_session::SessionPhase;
use crate::feat::session::protocol::soft_cancel_turn::SoftCancelTurn;

impl SessionPersistenceActor {
    /// BeginCompaction: set phase to Compacting, push "Starting..." system entry,
    /// mark gathered entries as ignored, and persist.
    pub(in crate::feat::session::session_actor) async fn handle_begin_compaction(
        &self,
        payload: &BeginCompaction,
        ctx: &ActorContext,
    ) {
        let (old_phase, new_phase) = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            let old_phase = session.phase();
            session.begin_compacting(payload.gathered_indices.clone());
            session.push_entry(ChatEntry::system("Starting context compaction..."));
            if !payload.gathered_indices.is_empty() {
                session.mark_entries_ignored(&payload.gathered_indices);
            }
            (old_phase, session.phase())
        };

        super::super::helpers::emit_phase_changed(ctx, &payload.session_id, old_phase, new_phase);

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
        let (old_phase, new_phase);
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            old_phase = session.phase();

            // Guard: ignore stale EndCompaction if phase is no longer Compacting.
            if !matches!(session.phase(), SessionPhase::Compacting) {
                tracing::warn!(
                    session_id = ?payload.session_id,
                    current_phase = ?session.phase(),
                    "EndCompaction received but session is not compacting — ignoring"
                );
                return;
            }

            if payload.skipped {
                // Compaction was skipped — all tokens fit within the reserve.
                if let Some(msg) = &payload.error {
                    session.push_entry(ChatEntry::system(msg.clone()));
                }
                session.finish_compacting();
            } else if let Some(result) = &payload.result {
                let compaction_entry = ChatEntry {
                    id: ChatEntryId::new(),
                    timestamp: jiff::Timestamp::now(),
                    kind: ChatEntryKind::Compaction {
                        summary: result.summary.clone(),
                        tokens_before: result.tokens_before,
                        tokens_after: result.tokens_after,
                        entries_compacted: result.entries_compacted,
                        model_used: result.model_used.clone(),
                    },
                    pin_position: None,
                    context_override: ContextOverride::Default,
                };
                session.insert_entry_at(result.boundary_index, compaction_entry);
                session.push_entry(ChatEntry::system(format!(
                    "Context was compacted. {} messages were summarized.",
                    result.entries_compacted
                )));

                if payload.auto {
                    // Auto-compaction succeeded — push continuation message and
                    // transition to Sending so the QueueActor dispatches the prompt.
                    session.push_entry(ChatEntry::user("A compaction has just occurred. Continue"));
                    session.finish_compacting_into_sending();
                } else {
                    // Manual compaction — return to Idle.
                    session.finish_compacting();
                }
            } else {
                let error_msg = payload.error.as_deref().unwrap_or("Unknown error");
                session.push_entry(ChatEntry::error(format!("Compaction failed: {error_msg}")));
                // Both auto and manual compaction failure → Idle (safe fallback).
                session.finish_compacting();
            }

            new_phase = session.phase();
        }

        super::super::helpers::emit_phase_changed(ctx, &payload.session_id, old_phase, new_phase);

        self.save_active_session(&payload.session_id).await;
    }

    /// SoftCancelTurn: request graceful turn termination at the next pause point.
    ///
    /// Sets a flag on the session that is checked at `on_tool_batch_completed`
    /// and `on_stream_completed`. When the flag is set, the turn ends (→ Idle)
    /// instead of continuing, allowing auto-compaction to trigger mid-turn.
    pub(in crate::feat::session::session_actor) fn handle_soft_cancel_turn(
        &self,
        payload: &SoftCancelTurn,
    ) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&payload.session_id);
        session.request_soft_cancel();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::super::super::helpers::{test_actor, test_context};
    use crate::feat::session::chat_session::SessionPhase;
    use crate::feat::session::protocol::soft_cancel_turn::SoftCancelTurn;
    use crate::protocol::{ChatEntry, ChatEntryKind, Command};

    #[tokio::test]
    async fn begin_compaction_sets_compacting_phase_and_pushes_system_entry() {
        // Given a session actor with a session.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
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
                &ctx,
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
                            tokens_after: 50,
                            model_used: "test/model".to_owned(),
                            boundary_index: 1,
                        },
                    ),
                    error: None,
                    auto: false,
                    skipped: false,
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
                    auto: false,
                    skipped: false,
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
    async fn end_compaction_retains_queued_messages_on_success() {
        // Given a session in Compacting phase with a queued user message.
        // The queue is no longer drained by the session actor — the QueueActor handles it.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(
                ChatEntry::user("queued during compaction"),
            ));
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
                            tokens_after: 50,
                            model_used: "test/model".to_owned(),
                            boundary_index: 0,
                        },
                    ),
                    error: None,
                    auto: false,
                    skipped: false,
                },
                &ctx,
            )
            .await;

        // Then the queue still has the item (QueueActor will pop it).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(
            session.queue_len(),
            1,
            "expected queue to retain item for QueueActor"
        );
    }

    #[tokio::test]
    async fn end_compaction_retains_queued_messages_on_failure() {
        // Given a session in Compacting phase with a queued user message.
        // The queue is no longer drained by the session actor — the QueueActor handles it.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(
                ChatEntry::user("queued during compaction"),
            ));
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
                    auto: false,
                    skipped: false,
                },
                &ctx,
            )
            .await;

        // Then the queue still has the item (QueueActor will pop it).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(
            session.queue_len(),
            1,
            "expected queue to retain item for QueueActor"
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
                            tokens_after: 50,
                            model_used: "test/model".to_owned(),
                            boundary_index: 0,
                        },
                    ),
                    error: None,
                    auto: false,
                    skipped: false,
                },
                &ctx,
            )
            .await;

        // Then no SendToLlmProvider command was emitted.
        let commands = sink.commands();
        let has_send = commands
            .iter()
            .any(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(
            !has_send,
            "expected no SendToLlmProvider when queue is empty"
        );
    }

    #[tokio::test]
    async fn end_compaction_with_queue_leaves_session_in_idle_state() {
        // Given a session in Compacting phase with a queued user message.
        // The session returns to Idle — the QueueActor will pop and dispatch.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(
                ChatEntry::user("queued during compaction"),
            ));
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
                            tokens_after: 50,
                            model_used: "test/model".to_owned(),
                            boundary_index: 0,
                        },
                    ),
                    error: None,
                    auto: false,
                    skipped: false,
                },
                &ctx,
            )
            .await;

        // Then the session is in Idle state (QueueActor will dispatch).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(matches!(session.phase(), SessionPhase::Idle));
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
                            tokens_after: 50,
                            model_used: "test/model".to_owned(),
                            boundary_index: 0,
                        },
                    ),
                    error: None,
                    auto: false,
                    skipped: false,
                },
                &ctx,
            )
            .await;

        // Then no compaction entry was inserted and the session is still Idle.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(session.phase(), SessionPhase::Idle);
        assert!(!session.history().iter().any(ChatEntry::is_compaction));
    }

    #[tokio::test]
    async fn end_compaction_auto_success_transitions_to_sending_with_continuation() {
        // Given a session in Compacting phase (simulating auto-compaction).
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("old message"));
            session.begin_compacting(vec![]);
            state.session.active_session_id().clone()
        };

        // When handling EndCompaction with auto=true and a successful result.
        actor
            .handle_end_compaction(
                &crate::feat::compaction_actor::protocol::command::EndCompaction {
                    session_id: session_id.clone(),
                    result: Some(
                        crate::feat::compaction_actor::protocol::command::CompactionResult {
                            summary: "summarized".to_owned(),
                            entries_compacted: 1,
                            tokens_before: 100,
                            tokens_after: 50,
                            model_used: "test/model".to_owned(),
                            boundary_index: 1,
                        },
                    ),
                    error: None,
                    auto: true,
                    skipped: false,
                },
                &ctx,
            )
            .await;

        // Then the session is in Sending phase (not Idle).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            matches!(session.phase(), SessionPhase::Sending),
            "expected Sending after auto-compaction success, got {:?}",
            session.phase()
        );

        // And the continuation user entry was pushed.
        let last_entry = session.history().last().expect("has continuation entry");
        assert!(
            matches!(&last_entry.kind, ChatEntryKind::User { display, .. } if display == "A compaction has just occurred. Continue"),
            "expected continuation user entry, got {:?}",
            last_entry.kind
        );

        // And SessionPhaseChanged was emitted (Compacting → Sending).
        let events = sink.events();
        let has_phase_change = events.iter().any(|e| {
            matches!(e, crate::protocol::Event::SessionPhaseChanged(p) if p.session_id == session_id)
        });
        assert!(has_phase_change, "expected SessionPhaseChanged event");
    }

    #[tokio::test]
    async fn end_compaction_auto_failure_falls_back_to_idle() {
        // Given a session in Compacting phase (simulating auto-compaction).
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            state.active_session_mut().begin_compacting(vec![]);
            state.session.active_session_id().clone()
        };

        // When handling EndCompaction with auto=true but a failure.
        actor
            .handle_end_compaction(
                &crate::feat::compaction_actor::protocol::command::EndCompaction {
                    session_id: session_id.clone(),
                    result: None,
                    error: Some("LLM call failed".to_owned()),
                    auto: true,
                    skipped: false,
                },
                &ctx,
            )
            .await;

        // Then the session falls back to Idle (safe fallback).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            matches!(session.phase(), SessionPhase::Idle),
            "expected Idle after auto-compaction failure, got {:?}",
            session.phase()
        );

        // And an error entry is present (no continuation entry).
        let last = session.history().last().expect("has an entry");
        assert!(
            matches!(&last.kind, ChatEntryKind::Error(msg) if msg.contains("Compaction failed"))
        );
    }

    #[tokio::test]
    async fn end_compaction_skipped_shows_system_message_and_returns_to_idle() {
        // Given a session actor with a session in Compacting phase.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            state.active_session_mut().begin_compacting(vec![]);
            state.session.active_session_id().clone()
        };

        // When handling EndCompaction with skipped=true.
        actor
            .handle_end_compaction(
                &crate::feat::compaction_actor::protocol::command::EndCompaction {
                    session_id: session_id.clone(),
                    result: None,
                    error: Some(
                        "Skipped compaction: 500 tokens within the 20000 token reserve.".to_owned(),
                    ),
                    auto: false,
                    skipped: true,
                },
                &ctx,
            )
            .await;

        // Then the session is back to Idle.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            matches!(session.phase(), SessionPhase::Idle),
            "expected Idle after skipped compaction, got {:?}",
            session.phase()
        );

        // And a system message (not error) was pushed.
        let last = session.history().last().expect("has an entry");
        assert!(
            matches!(&last.kind, ChatEntryKind::System(msg) if msg.contains("Skipped compaction")),
            "expected system message with skip explanation, got {:?}",
            last.kind
        );
    }

    #[tokio::test]
    async fn soft_cancel_turn_sets_flag_on_session() {
        // Given a session actor with a session.
        let actor = test_actor();
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        // When handling SoftCancelTurn.
        actor.handle_soft_cancel_turn(&SoftCancelTurn {
            session_id: session_id.clone(),
        });

        // Then the soft cancel flag is set.
        let mut state = actor.state.write();
        let session = state.session.get_mut(&session_id).expect("session exists");
        assert!(
            session.take_soft_cancel(),
            "expected soft cancel flag to be set"
        );
    }
}
