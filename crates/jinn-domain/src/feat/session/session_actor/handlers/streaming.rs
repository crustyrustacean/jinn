//! Streaming lifecycle handlers - manage token streaming and stream completion.
//!
//! Handles appending individual tokens to the assistant entry (including
//! reasoning/thinking tokens), and finalizing the stream with token accounting
//! and queue draining on `StreamCompleted`.

use crate::common::actor_deps::BusPublish;
use crate::feat::context::protocol::event::ContextOverrideChanged;
use crate::feat::context::strategy::token_estimator::TokenCounter;
use crate::feat::provider::protocol::event::{StreamCompleted, StreamCompletedReason, StreamToken};
use crate::protocol::ChatEntry;

use super::super::SessionPersistenceActor;
use crate::feat::session::phase_machine::PhaseKind;

impl SessionPersistenceActor {
    /// Appends a streaming token to the session's assistant entry,
    /// or to the thinking entry if the token is flagged as reasoning.
    #[expect(
        clippy::else_if_without_else,
        reason = "no-op on fallthrough is intentional"
    )]
    pub(in crate::feat::session::session_actor) fn on_stream_token(&self, event: &StreamToken) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        match session.phase() {
            PhaseKind::Streaming => {}
            PhaseKind::Sending => {
                // Defensive: stream token arrived without phase transition.
                session.begin_streaming();
            }
            PhaseKind::Idle => {
                tracing::warn!(
                    phase = ?session.phase(),
                    "StreamToken received in unexpected phase"
                );
            }
        }
        if event.is_thinking {
            if session.streaming_thinking_entry_index().is_none() {
                session.begin_thinking(event.dispatched_at);
            }
            if let Err(e) = session.append_thinking_token(&event.token) {
                tracing::error!(err = ?e, "failed to append thinking token");
            }
        } else if let Err(e) = session.append_stream_token(&event.token, event.dispatched_at) {
            tracing::error!(err = ?e, "failed to append stream token");
        }
    }

    /// Marks the session's stream as finished, records output tokens, and
    /// drains any queued messages into a new turn.
    ///
    /// For `Finished` reason, drains the message queue. If messages were queued,
    /// pushes each as a separate user entry and triggers re-assembly.
    ///
    /// For `ToolUse` reason, transitions to sending state instead of fully idle,
    /// so the streaming indicator remains visible while the followup response
    /// is awaited. The queue is NOT drained - the turn hasn't ended.
    #[expect(clippy::too_many_lines, reason = "1 line over limit")]
    #[expect(
        clippy::else_if_without_else,
        reason = "no-op on fallthrough is intentional"
    )]
    pub(in crate::feat::session::session_actor) async fn on_stream_completed(
        &self,
        event: &StreamCompleted,
    ) {
        let should_save = event.reason == StreamCompletedReason::Finished
            || event.reason == StreamCompletedReason::Error
            || event.reason == StreamCompletedReason::Canceled;

        // Count output tokens: always count locally, then take max with provider-reported.
        // This handles providers that undercount (e.g., excluding tool call arguments).
        let local_count_handle: Option<tokio::task::JoinHandle<u32>> = if event.reason
            != StreamCompletedReason::Canceled
            && event.reason != StreamCompletedReason::Error
        {
            event.assistant_content.as_ref().map(|content| {
                let content = content.clone();
                let tool_calls = event.tool_calls.clone();
                let thinking = event.thinking_content.clone().unwrap_or_default();
                let counter = self.counter;
                tokio::task::spawn_blocking(move || {
                    let mut tokens = counter.count(&content) as u32;
                    tokens += counter.count(&thinking) as u32;
                    if let Some(tool_calls) = tool_calls {
                        for tc in &tool_calls {
                            tokens += counter.count(&tc.arguments) as u32;
                            tokens += counter.count(&tc.name) as u32;
                        }
                    }
                    tokens
                })
            })
        } else {
            None
        };

        let provider_tokens: Option<u32> = event.provider_completion_tokens.map(|t| t as u32);

        // Await local counting outside the lock, then take max with provider.
        let output_tokens: Option<u32> = match local_count_handle {
            Some(handle) => {
                let local = handle.await.unwrap_or_else(|e| {
                    tracing::warn!(
                        err = ?e,
                        "spawn_blocking panicked during output token counting"
                    );
                    0
                });
                Some(local.max(provider_tokens.unwrap_or(0)))
            }
            None => provider_tokens,
        };

        let mut all_changed: Vec<crate::protocol::ChatEntryId> = Vec::new();
        let (old_phase, new_phase);
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&event.session_id);
            old_phase = session.phase();
            if event.reason == StreamCompletedReason::Canceled {
                session.push_entry(ChatEntry::error("Cancelled"));
            } else if event.reason == StreamCompletedReason::Error {
                // Error entry is pushed by the LLM actor via PushChatEntry before
                // emitting StreamCompleted(Error). Nothing to push here.
            } else if let Some(output_tokens) = output_tokens {
                // Finalize the last record if one exists (i.e., prompt assembled first).
                // If no record exists (e.g., session restored mid-stream), skip silently.
                if !session.token_ledger().is_empty()
                    && let Err(e) = session.finalize_last_token_record(
                        output_tokens,
                        event.cost,
                        event.model_used.clone(),
                    )
                {
                    tracing::error!(err = ?e, "failed to finalize token record");
                }
            }
            let preserve_assistant = event.reason == StreamCompletedReason::Finished
                || event.reason == StreamCompletedReason::ToolUse;
            session.finish_streaming(preserve_assistant, event.dispatched_at);

            // Hard cancel: force-exclude dangling tool calls left by the interrupted stream.
            if event.reason == StreamCompletedReason::Canceled {
                all_changed.extend(session.force_exclude_dangling_tool_calls());
            }

            // Apply pending history mutations for non-ToolUse completions.
            // ToolUse defers to on_tool_batch_completed.
            if event.reason != StreamCompletedReason::ToolUse {
                let (count, changed) = session.drain_and_apply_pending_mutations();
                all_changed.extend(changed);
                if count > 0 {
                    tracing::debug!(
                        session_id = %event.session_id,
                        count,
                        reason = ?event.reason,
                        "applied pending history mutations at stream completion"
                    );
                }
            }

            // Tool use means the conversation continues - always transition to sending
            // so the tool loop runs.
            if event.reason == StreamCompletedReason::ToolUse {
                session.begin_sending();
            }

            // When returning to Idle with no retry, drain queued messages
            // back to the input buffer so the user can review and retry.
            if matches!(
                event.reason,
                StreamCompletedReason::Error | StreamCompletedReason::Canceled
            ) {
                let drained = session.drain_queue();
                let display_texts: Vec<&str> = drained
                    .iter()
                    .filter_map(|item| match item {
                        crate::feat::session::queue_item::QueueItem::UserMessage(entry) => {
                            match &entry.kind {
                                crate::protocol::ChatEntryKind::User { display, .. } => {
                                    Some(display.as_str())
                                }
                                _ => None,
                            }
                        }
                        crate::feat::session::queue_item::QueueItem::ToolContinuation => None,
                    })
                    .collect();
                let drained_text = display_texts.join("\n");
                if !drained_text.is_empty() {
                    session.chat_input_mut().replace_all(drained_text);
                }
            }

            new_phase = session.phase();
        }

        // Emit ContextOverrideChanged events for any entry whose override actually changed
        // (from dangling-tool-call sweep or pending worker mutations). Outside the write lock.
        for entry_id in all_changed {
            self.publish(ContextOverrideChanged {
                session_id: event.session_id.clone(),
                entry_id,
            })
            .await;
        }

        super::super::helpers::emit_phase_changed(
            self.bus(),
            &event.session_id,
            old_phase,
            new_phase,
        )
        .await;
        super::super::helpers::emit_history_appended(self.bus(), &event.session_id).await;

        // Persist session after stream finishes.
        if should_save {
            self.save_active_session(&event.session_id).await;
        }
    }
}

//FIXME: plugin migration
#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::super::super::helpers::{test_actor, test_actor_recording, test_actor_with_store_recording};
    use crate::feat::provider::protocol::event::{
        StreamCompleted, StreamCompletedReason, StreamToken,
    };
    use crate::feat::session::phase_machine::PhaseKind;
    use crate::feat::session::token_stats::TokenRecord;
    use crate::protocol::{ChangeSource, ChatEntry, ChatEntryKind};

    #[tokio::test]
    async fn on_stream_completed_error_stops_streaming() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Error,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(!matches!(session.phase(), PhaseKind::Streaming));
    }

    #[tokio::test]
    async fn on_stream_completed_error_reason_drains_queue_to_input_buffer() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            session.enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(
                Box::new(ChatEntry::user("queued message")),
            ));
            state.session.active_session_id().clone()
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Error,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(session.queue_len(), 0);
        assert_eq!(session.chat_input().text(), "queued message");
    }

    #[tokio::test]
    async fn on_stream_completed_error_with_multiple_queued_messages_joins_with_newline() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            session.enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(
                Box::new(ChatEntry::user("first message")),
            ));
            session.enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(
                Box::new(ChatEntry::user("second message")),
            ));
            state.session.active_session_id().clone()
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Error,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(session.queue_len(), 0);
        assert_eq!(session.chat_input().text(), "first message\nsecond message");
    }

    #[tokio::test]
    async fn on_stream_completed_canceled_reason_drains_queue_to_input_buffer() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            session.enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(
                Box::new(ChatEntry::user("queued message")),
            ));
            state.session.active_session_id().clone()
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Canceled,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(session.queue_len(), 0);
        assert_eq!(session.chat_input().text(), "queued message");
    }

    #[tokio::test]
    async fn on_stream_completed_finished_emits_history_appended() {
        let (actor, audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("response".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        assert!(
            audit.contains_name("HistoryAppended"),
            "expected HistoryAppended event after stream completed"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_error_emits_history_appended() {
        let (actor, audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Error,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        assert!(
            audit.contains_name("HistoryAppended"),
            "expected HistoryAppended event after stream error"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_canceled_emits_history_appended() {
        let (actor, audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Canceled,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        assert!(
            audit.contains_name("HistoryAppended"),
            "expected HistoryAppended event after stream canceled"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_canceled_force_excludes_dangling_tool_calls() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("run it"));
            session.push_entry(ChatEntry::assistant(""));
            session.push_entry(ChatEntry::tool_call("tc-1", "bash", r#"{"command":"ls"}"#));
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Canceled,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let history = session.history();
        assert_eq!(
            history[0].context_override(),
            crate::protocol::ContextOverride::Default
        );
        assert_eq!(
            history[1].context_override(),
            crate::protocol::ContextOverride::ForcedExclude
        );
        assert_eq!(
            history[2].context_override(),
            crate::protocol::ContextOverride::ForcedExclude
        );
        assert_eq!(
            history[3].context_override(),
            crate::protocol::ContextOverride::Default
        );
    }

    #[tokio::test]
    async fn on_stream_token_appends_text_to_assistant_entry() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        actor.on_stream_token(&StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "Hello".to_owned(),
            is_thinking: false,
            dispatched_at: jiff::Timestamp::now(),
        });
        actor.on_stream_token(&StreamToken {
            session_id: session_id.clone(),
            index: 1,
            token: " world".to_owned(),
            is_thinking: false,
            dispatched_at: jiff::Timestamp::now(),
        });

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let assistant_text = session
            .history()
            .iter()
            .find_map(|e| match &e.kind {
                ChatEntryKind::Assistant(t) => Some(t.clone()),
                _ => None,
            })
            .expect("should have an assistant entry");
        assert_eq!(assistant_text, "Hello world");
    }

    #[tokio::test]
    async fn on_stream_token_keeps_phase_as_streaming() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        {
            let mut state = actor.state.write();
            state.active_session_mut().begin_streaming();
        }
        actor.on_stream_token(&StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "hi".to_owned(),
            is_thinking: false,
            dispatched_at: jiff::Timestamp::now(),
        });

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            matches!(session.phase(), PhaseKind::Streaming),
            "expected Streaming phase, got {:?}",
            session.phase()
        );
    }

    #[tokio::test]
    async fn on_stream_token_corrects_sending_phase_to_streaming() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("go"));
            session.begin_sending();
            state.session.active_session_id().clone()
        };

        actor.on_stream_token(&StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "response".to_owned(),
            is_thinking: false,
            dispatched_at: jiff::Timestamp::now(),
        });

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            matches!(session.phase(), PhaseKind::Streaming),
            "expected Streaming phase after correction from Sending, got {:?}",
            session.phase()
        );
    }

    #[tokio::test]
    async fn on_stream_completed_finished_persists_session() {
        let (actor, store, _audit) = test_actor_with_store_recording(vec![]).await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.mark_interacted();
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("response".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        // Then the session was persisted (should_save = true for Finished).
        assert!(
            store.last_saved_session(&session_id).is_some(),
            "expected session to be saved after Finished"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_error_persists_session() {
        // Given an interacted session in streaming state.
        let (actor, store, _audit) = test_actor_with_store_recording(vec![]).await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.mark_interacted();
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted with Error reason.
        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Error,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        // Then the session was persisted (should_save = true for Error).
        assert!(
            store.last_saved_session(&session_id).is_some(),
            "expected session to be saved after Error"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_canceled_persists_session() {
        // Given an interacted session in streaming state.
        let (actor, store, _audit) = test_actor_with_store_recording(vec![]).await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.mark_interacted();
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted with Canceled reason.
        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Canceled,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        // Then the session was persisted.
        assert!(
            store.last_saved_session(&session_id).is_some(),
            "expected session to be saved after Canceled"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_does_not_count_tokens_on_error() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_token_record(TokenRecord {
                model_used: None,
                timestamp: jiff::Timestamp::now(),
                tokens_sent: 100,
                tokens_received: 0,
                cost: None,
            });
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Error,
            assistant_content: Some("some error content".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let ledger = session.token_ledger();
        assert_eq!(ledger.len(), 1);
        assert_eq!(
            ledger[0].tokens_received, 0,
            "expected 0 tokens_received on Error"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_tool_use_preserves_assistant_entry() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("do something"));
            session.begin_streaming();
            state.session.active_session_id().clone()
        };
        actor.on_stream_token(&StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "I will help".to_owned(),
            is_thinking: false,
            dispatched_at: jiff::Timestamp::now(),
        });

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::ToolUse,
            assistant_content: Some("response".to_owned()),
            tool_calls: Some(vec![crate::feat::tools_actor::tool_types::ToolCall {
                id: "tc-1".to_owned(),
                name: "bash".to_owned(),
                arguments: "{}".to_owned(),
            }]),
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let has_assistant = session
            .history()
            .iter()
            .any(|e| matches!(&e.kind, ChatEntryKind::Assistant(t) if t == "I will help"));
        assert!(
            has_assistant,
            "expected assistant entry 'I will help' to be preserved after ToolUse"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_tool_use_counts_tool_call_arguments() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_token_record(TokenRecord {
                model_used: None,
                timestamp: jiff::Timestamp::now(),
                tokens_sent: 100,
                tokens_received: 0,
                cost: None,
            });
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::ToolUse,
            assistant_content: Some("checking".to_owned()),
            tool_calls: Some(vec![crate::feat::tools_actor::tool_types::ToolCall {
                id: "tc-1".to_owned(),
                name: "bash".to_owned(),
                arguments: r#"{"command":"ls -la /very/long/path"}"#.to_owned(),
            }]),
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let ledger = session.token_ledger();
        assert_eq!(ledger.len(), 1);
        assert!(
            ledger[0].tokens_received > 2,
            "expected tokens_received > 2 (text only), got {}",
            ledger[0].tokens_received
        );
    }

    #[tokio::test]
    async fn on_stream_completed_finished_preserves_assistant_entry() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.begin_streaming();
            state.session.active_session_id().clone()
        };
        actor.on_stream_token(&StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "world".to_owned(),
            is_thinking: false,
            dispatched_at: jiff::Timestamp::now(),
        });

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("world".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let has_world = session
            .history()
            .iter()
            .any(|e| matches!(&e.kind, ChatEntryKind::Assistant(t) if t.contains("world")));
        assert!(
            has_world,
            "expected assistant entry with 'world' to be preserved after Finished"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_canceled_with_complete_tool_loop_does_not_exclude() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("run it"));
            session.push_entry(ChatEntry::assistant(""));
            session.push_entry(ChatEntry::tool_call("tc-1", "bash", r#"{"command":"ls"}"#));
            session.push_entry(ChatEntry::tool_result(
                "tc-1",
                "bash",
                "file.txt",
                crate::feat::session::tool_result_status::ToolResultStatus::Success,
            ));
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Canceled,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        for entry in session.history() {
            assert_eq!(
                entry.context_override(),
                crate::protocol::ContextOverride::Default,
                "expected Default for entry {:?}",
                entry.kind
            );
        }
    }

    #[tokio::test]
    async fn on_stream_completed_finished_without_auto_compaction_goes_to_idle() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("response".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            matches!(session.phase(), PhaseKind::Idle),
            "expected Idle after Finished without auto-compaction, got {:?}",
            session.phase()
        );
    }

    #[tokio::test]
    async fn on_stream_completed_finished_applies_pending_mutations() {
        let (actor, audit) = test_actor_recording().await;
        let (entry_id, session_id) = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            let entry = ChatEntry::assistant("response");
            let entry_id = entry.id.clone();
            session.push_entry(entry);
            session.begin_streaming();
            session.queue_mutations(vec![
                crate::feat::session::history_mutation::HistoryMutation::SetContextOverride {
                    entry_id: entry_id.clone(),
                    value: crate::protocol::ContextOverride::ForcedExclude,
                    source: ChangeSource::Internal {
                        label: "test".into(),
                    },
                },
            ]);
            (entry_id, state.session.active_session_id().clone())
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("response".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let assistant = session
            .history()
            .iter()
            .find(|e| e.id == entry_id)
            .expect("entry");
        assert_eq!(
            assistant.context_override(),
            crate::protocol::ContextOverride::ForcedExclude
        );
        assert!(audit.contains_name("HistoryAppended"));
    }
    #[tokio::test]
    async fn on_stream_completed_error_applies_pending_mutations() {
        let (actor, _audit) = test_actor_recording().await;
        let (entry_id, session_id) = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            let entry = ChatEntry::assistant("partial");
            let entry_id = entry.id.clone();
            session.push_entry(entry);
            session.begin_streaming();
            session.queue_mutations(vec![
                crate::feat::session::history_mutation::HistoryMutation::SetContextOverride {
                    entry_id: entry_id.clone(),
                    value: crate::protocol::ContextOverride::ForcedExclude,
                    source: ChangeSource::Internal {
                        label: "test".into(),
                    },
                },
            ]);
            (entry_id, state.session.active_session_id().clone())
        };

        // When handling StreamCompleted with Error reason.
        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Error,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        // Then the mutation was applied.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let assistant = session
            .history()
            .iter()
            .find(|e| e.id == entry_id)
            .expect("assistant entry exists");
        assert_eq!(
            assistant.context_override(),
            crate::protocol::ContextOverride::ForcedExclude,
            "expected mutation to be applied at stream error"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_canceled_applies_pending_mutations() {
        // Given a session in streaming state with pending mutations.
        let actor = test_actor().await;
        let (entry_id, session_id) = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            let entry = ChatEntry::assistant("partial");
            let entry_id = entry.id.clone();
            session.push_entry(entry);
            session.begin_streaming();
            session.queue_mutations(vec![
                crate::feat::session::history_mutation::HistoryMutation::SetContextOverride {
                    entry_id: entry_id.clone(),
                    value: crate::protocol::ContextOverride::ForcedExclude,
                    source: ChangeSource::Internal {
                        label: "test".into(),
                    },
                },
            ]);
            (entry_id, state.session.active_session_id().clone())
        };

        // When handling StreamCompleted with Canceled reason.
        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Canceled,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let assistant = session
            .history()
            .iter()
            .find(|e| e.id == entry_id)
            .expect("entry");
        assert_eq!(
            assistant.context_override(),
            crate::protocol::ContextOverride::ForcedExclude
        );
    }

    #[tokio::test]
    async fn on_stream_completed_tool_use_does_not_apply_mutations() {
        let (actor, _audit) = test_actor_recording().await;
        let (entry_id, session_id) = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            let entry = ChatEntry::assistant("checking");
            let entry_id = entry.id.clone();
            session.push_entry(entry);
            session.begin_streaming();
            session.queue_mutations(vec![
                crate::feat::session::history_mutation::HistoryMutation::SetContextOverride {
                    entry_id: entry_id.clone(),
                    value: crate::protocol::ContextOverride::ForcedExclude,
                    source: ChangeSource::Internal {
                        label: "test".into(),
                    },
                },
            ]);
            (entry_id, state.session.active_session_id().clone())
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::ToolUse,
            assistant_content: Some("response".to_owned()),
            tool_calls: Some(vec![crate::feat::tools_actor::tool_types::ToolCall {
                id: "tc-1".to_owned(),
                name: "bash".to_owned(),
                arguments: "{}".to_owned(),
            }]),
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let assistant = session
            .history()
            .iter()
            .find(|e| e.id == entry_id)
            .expect("entry");
        assert_eq!(
            assistant.context_override(),
            crate::protocol::ContextOverride::Default,
            "expected mutation to NOT be applied for ToolUse reason"
        );
    }

    // --- Hybrid token counting tests ---

    #[tokio::test]
    async fn on_stream_completed_provider_tokens_used_directly() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_token_record(TokenRecord {
                timestamp: jiff::Timestamp::now(),
                tokens_sent: 100,
                tokens_received: 0,
                cost: None,
                model_used: None,
            });
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("short".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: Some(5000),
            thinking_content: Some("very long thinking content here".to_owned()),
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(session.token_ledger()[0].tokens_received, 5000);
    }

    #[tokio::test]
    async fn on_stream_completed_local_fallback_includes_thinking() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_token_record(TokenRecord {
                timestamp: jiff::Timestamp::now(),
                tokens_sent: 100,
                tokens_received: 0,
                cost: None,
                model_used: None,
            });
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("short".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: Some("a substantial amount of reasoning text".to_owned()),
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            session.token_ledger()[0].tokens_received > 2,
            "expected tokens_received > 2, got {}",
            session.token_ledger()[0].tokens_received
        );
    }

    #[tokio::test]
    async fn on_stream_completed_local_fallback_without_thinking_backward_compat() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_token_record(TokenRecord {
                model_used: None,
                timestamp: jiff::Timestamp::now(),
                tokens_sent: 100,
                tokens_received: 0,
                cost: None,
            });
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("response text".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            session.token_ledger()[0].tokens_received > 0,
            "expected nonzero tokens_received for 'response text'"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_provider_tokens_preferred_over_local() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_token_record(TokenRecord {
                model_used: None,
                timestamp: jiff::Timestamp::now(),
                tokens_sent: 100,
                tokens_received: 0,
                cost: None,
            });
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("short".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: Some(9999),
            thinking_content: Some(
                "extremely long thinking content that would produce many tokens".to_owned(),
            ),
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(session.token_ledger()[0].tokens_received, 9999);
    }

    #[tokio::test]
    async fn on_stream_completed_takes_max_when_provider_undercounts() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_token_record(TokenRecord {
                model_used: None,
                timestamp: jiff::Timestamp::now(),
                tokens_sent: 100,
                tokens_received: 0,
                cost: None,
            });
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted with no provider tokens and no thinking.
        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("response text".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        // Then tokens_received counts only the text (backward compat).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let ledger = session.token_ledger();
        assert_eq!(ledger.len(), 1);
        assert!(
            ledger[0].tokens_received > 0,
            "expected nonzero tokens_received for 'response text', got {}",
            ledger[0].tokens_received
        );
    }


    #[tokio::test]
    async fn on_stream_completed_takes_max_when_provider_overcounts() {
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_token_record(TokenRecord {
                model_used: None,
                timestamp: jiff::Timestamp::now(),
                tokens_sent: 100,
                tokens_received: 0,
                cost: None,
            });
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("ok".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: Some(50000),
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(session.token_ledger()[0].tokens_received, 50000);
    }

    #[tokio::test]
    async fn on_stream_completed_uses_local_count_when_no_provider_report() {
        use crate::feat::context::strategy::token_estimator::TokenCounter;
        let (actor, _audit) = test_actor_recording().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_token_record(TokenRecord {
                model_used: None,
                timestamp: jiff::Timestamp::now(),
                tokens_sent: 100,
                tokens_received: 0,
                cost: None,
            });
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        let content = "hello world this is a test";
        let event = StreamCompleted {
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some(content.to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
        };
        actor.on_stream_completed(&event).await;

        let counter =
            crate::feat::context::strategy::token_estimator::TiktokenCounter::o200k_base();
        let expected = counter.count(content) as u32;

        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(session.token_ledger()[0].tokens_received, expected);
    }

    // --- EntryTiming integration tests ---

    #[tokio::test]
    async fn dispatched_at_flows_from_stream_token_to_entry_timing() {
        // Given a session actor with a session in streaming state.
        let actor = test_actor().await;
        let dispatched = jiff::Timestamp::now();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When handling a StreamToken with a specific dispatched_at.
        actor.on_stream_token(&StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "Hello".to_owned(),
            is_thinking: false,
            dispatched_at: dispatched,
        });

        // Then the assistant entry's timing has that dispatched_at.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let assistant = session
            .history()
            .iter()
            .find(|e| matches!(e.kind, crate::protocol::ChatEntryKind::Assistant(_)))
            .expect("assistant entry");
        match &assistant.timing {
            crate::protocol::EntryTiming::Streamed {
                dispatched_at,
                first_token_at,
                finished_at,
            } => {
                assert_eq!(dispatched_at, &dispatched);
                assert!(first_token_at.is_some());
                assert!(finished_at.is_none());
            }
            other => panic!("expected Streamed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn thinking_entry_gets_dispatched_at_from_stream_token() {
        // Given a session actor with a session in streaming state.
        let actor = test_actor().await;
        let dispatched = jiff::Timestamp::now();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When handling a thinking StreamToken with a specific dispatched_at.
        actor.on_stream_token(&StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "reasoning".to_owned(),
            is_thinking: true,
            dispatched_at: dispatched,
        });

        // Then the thinking entry's timing has that dispatched_at.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let thinking = session
            .history()
            .iter()
            .find(|e| matches!(e.kind, crate::protocol::ChatEntryKind::Thinking(_)))
            .expect("thinking entry");
        match &thinking.timing {
            crate::protocol::EntryTiming::Streamed {
                dispatched_at,
                first_token_at,
                finished_at,
            } => {
                assert_eq!(dispatched_at, &dispatched);
                assert!(first_token_at.is_some());
                assert!(finished_at.is_none());
            }
            other => panic!("expected Streamed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stream_completed_sets_finished_at_on_assistant_entry() {
        // Given a session actor with a session in streaming state and a token.
        let actor = test_actor().await;
        let dispatched = jiff::Timestamp::now();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            state.session.active_session_id().clone()
        };
        actor.on_stream_token(&StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "Hello".to_owned(),
            is_thinking: false,
            dispatched_at: dispatched,
        });

        // When handling StreamCompleted with Finished reason.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("Hello".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: Some(10),
            thinking_content: None,
            dispatched_at: dispatched,
            model_used: None,
        };
        actor.on_stream_completed(&event).await;

        // Then the assistant entry has finished_at set.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let assistant = session
            .history()
            .iter()
            .find(|e| matches!(e.kind, crate::protocol::ChatEntryKind::Assistant(_)))
            .expect("assistant entry");
        match &assistant.timing {
            crate::protocol::EntryTiming::Streamed { finished_at, .. } => {
                assert!(
                    finished_at.is_some(),
                    "finished_at should be set after completion"
                );
            }
            other => panic!("expected Streamed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancelled_stream_records_finished_at() {
        // Given a session actor with a session in streaming state and a token.
        let actor = test_actor().await;
        let dispatched = jiff::Timestamp::now();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            state.session.active_session_id().clone()
        };
        actor.on_stream_token(&StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "Partial".to_owned(),
            is_thinking: false,
            dispatched_at: dispatched,
        });

        // When handling StreamCompleted with Canceled reason.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Canceled,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: dispatched,
            model_used: None,
        };
        actor.on_stream_completed(&event).await;

        // Then the assistant entry has finished_at set (cancellation is a finish event).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let assistant = session
            .history()
            .iter()
            .find(|e| matches!(e.kind, crate::protocol::ChatEntryKind::Assistant(_)))
            .expect("assistant entry");
        match &assistant.timing {
            crate::protocol::EntryTiming::Streamed { finished_at, .. } => {
                assert!(
                    finished_at.is_some(),
                    "finished_at should be set even on cancellation"
                );
            }
            other => panic!("expected Streamed, got {other:?}"),
        }
    }
}
