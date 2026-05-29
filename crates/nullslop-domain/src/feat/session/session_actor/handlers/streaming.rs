//! Streaming lifecycle handlers — manage token streaming and stream completion.
//!
//! Handles appending individual tokens to the assistant entry (including
//! reasoning/thinking tokens), and finalizing the stream with token accounting
//! and queue draining on `StreamCompleted`.

use crate::common::actor::ActorContext;
use crate::feat::context::strategy::token_estimator::TokenCounter;
use crate::feat::provider::protocol::event::{StreamCompleted, StreamCompletedReason, StreamToken};

use crate::protocol::ChatEntry;

use super::super::SessionPersistenceActor;
use crate::feat::session::chat_session::SessionPhase;

impl SessionPersistenceActor {
    /// Appends a streaming token to the session's assistant entry,
    /// or to the thinking entry if the token is flagged as reasoning.
    pub(in crate::feat::session::session_actor) fn on_stream_token(&self, event: &StreamToken) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        match session.phase() {
            SessionPhase::Streaming => {}
            SessionPhase::Sending => {
                // Defensive: stream token arrived without phase transition.
                session.begin_streaming();
            }
            _ => {
                tracing::warn!(
                    phase = ?session.phase(),
                    "StreamToken received in unexpected phase"
                );
            }
        }
        if event.is_thinking {
            if session.streaming_thinking_entry_index().is_none() {
                session.begin_thinking();
            }
            if let Err(e) = session.append_thinking_token(&event.token) {
                tracing::error!(err = ?e, "failed to append thinking token");
            }
        } else if let Err(e) = session.append_stream_token(&event.token) {
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
    /// is awaited. The queue is NOT drained — the turn hasn't ended.
    #[expect(clippy::too_many_lines, reason = "1 line over limit")]
    pub(in crate::feat::session::session_actor) async fn on_stream_completed(
        &self,
        event: &StreamCompleted,
        ctx: &ActorContext,
    ) {
        let should_save = event.reason == StreamCompletedReason::Finished
            || event.reason == StreamCompletedReason::Error;

        // Count output tokens off the async thread.
        // Prefer provider-reported tokens (includes thinking, matches billing).
        // Fall back to local counting (also includes thinking) when unavailable.
        let output_tokens: Option<tokio::task::JoinHandle<u32>> = if event.reason
            != StreamCompletedReason::Canceled
            && event.reason != StreamCompletedReason::Error
        {
            if let Some(provider_tokens) = event.provider_completion_tokens {
                // Provider-reported path: use directly, no local counting needed.
                Some(tokio::task::spawn_blocking(move || provider_tokens as u32))
            } else {
                // Local fallback path: count text + thinking + tool calls.
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
            }
        } else {
            None
        };

        // Await token counting outside the lock.
        let output_tokens: Option<u32> = match output_tokens {
            Some(handle) => Some(handle.await.unwrap_or_else(|e| {
                tracing::warn!(err = ?e, "spawn_blocking panicked during output token counting");
                0
            })),
            None => None,
        };

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
                    && let Err(e) = session.finalize_last_token_record(output_tokens, event.cost)
                {
                    tracing::error!(err = ?e, "failed to finalize token record");
                }
            }
            let preserve_assistant = event.reason == StreamCompletedReason::Finished
                || event.reason == StreamCompletedReason::ToolUse;
            session.finish_streaming(preserve_assistant);

            // Hard cancel: force-exclude dangling tool calls left by the interrupted stream.
            if event.reason == StreamCompletedReason::Canceled {
                session.force_exclude_dangling_tool_calls();
            }

            // Apply pending history mutations for non-ToolUse completions.
            // ToolUse defers to on_tool_batch_completed.
            if event.reason != StreamCompletedReason::ToolUse
                && !session.is_judge()
            {
                let count = session.drain_and_apply_pending_mutations();
                if count > 0 {
                    tracing::debug!(
                        session_id = %event.session_id,
                        count,
                        reason = ?event.reason,
                        "applied pending history mutations at stream completion"
                    );
                }
            }

            // Tool use means the conversation continues — always transition to sending
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

        super::super::helpers::emit_phase_changed(ctx, &event.session_id, old_phase, new_phase);
        super::super::helpers::emit_history_appended(ctx, &event.session_id);

        // Persist session after stream finishes (not on cancel).
        if should_save {
            self.save_active_session(&event.session_id).await;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::super::super::helpers::{test_actor, test_context};
    use crate::feat::provider::protocol::event::{
        StreamCompleted, StreamCompletedReason, StreamToken,
    };
    use crate::feat::session::chat_session::SessionPhase;
    use crate::feat::session::token_stats::TokenRecord;
    use crate::protocol::{ChatEntry, Event};

    #[tokio::test]
    async fn on_stream_completed_error_reason_finishes_streaming() {
        // Given a session actor with a session in streaming state.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            assert!(matches!(session.phase(), SessionPhase::Streaming));
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted with Error reason.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Error,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the session is no longer streaming.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(!matches!(session.phase(), SessionPhase::Streaming));
    }

    #[tokio::test]
    async fn on_stream_completed_error_reason_drains_queue_to_input_buffer() {
        // Given a session actor with a session in streaming state and a queued message.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            session.enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(
                ChatEntry::user("queued message"),
            ));
            assert_eq!(session.queue_len(), 1);
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted with Error reason.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Error,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the queue is empty and the message text is in the input buffer.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(session.queue_len(), 0);
        assert_eq!(session.chat_input().text(), "queued message");
    }

    #[tokio::test]
    async fn on_stream_completed_error_with_multiple_queued_messages_joins_with_newline() {
        // Given a session actor with a session in streaming state and two queued messages.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            session.enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(
                ChatEntry::user("first message"),
            ));
            session.enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(
                ChatEntry::user("second message"),
            ));
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted with Error reason.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Error,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then both messages are joined with newline in the input buffer.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(session.queue_len(), 0);
        assert_eq!(session.chat_input().text(), "first message\nsecond message");
    }

    #[tokio::test]
    async fn on_stream_completed_canceled_reason_drains_queue_to_input_buffer() {
        // Given a session actor with a session in streaming state and a queued message.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            session.enqueue(crate::feat::session::queue_item::QueueItem::UserMessage(
                ChatEntry::user("queued message"),
            ));
            assert_eq!(session.queue_len(), 1);
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted with Canceled reason.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Canceled,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the queue is empty and the message text is in the input buffer.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(session.queue_len(), 0);
        assert_eq!(session.chat_input().text(), "queued message");
    }

    #[tokio::test]
    async fn on_stream_completed_finished_emits_history_appended() {
        // Given a session actor with a session in streaming state with history.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted with Finished reason.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("response".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then a HistoryAppended event was emitted.
        let events = sink.events();
        let has_history = events.iter().any(
            |e| matches!(e, Event::HistoryAppended(payload) if payload.session_id == session_id),
        );
        assert!(
            has_history,
            "expected HistoryAppended event after stream completed"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_error_emits_history_appended() {
        // Given a session actor with a session in streaming state with history.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted with Error reason.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Error,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then a HistoryAppended event was emitted.
        let events = sink.events();
        let has_history = events.iter().any(
            |e| matches!(e, Event::HistoryAppended(payload) if payload.session_id == session_id),
        );
        assert!(
            has_history,
            "expected HistoryAppended event after stream error"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_canceled_emits_history_appended() {
        // Given a session actor with a session in streaming state with history.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted with Canceled reason.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Canceled,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then a HistoryAppended event was emitted.
        let events = sink.events();
        let has_history = events.iter().any(
            |e| matches!(e, Event::HistoryAppended(payload) if payload.session_id == session_id),
        );
        assert!(
            has_history,
            "expected HistoryAppended event after stream canceled"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_canceled_force_excludes_dangling_tool_calls() {
        // Given a session in streaming state with dangling tool calls in history.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("run it"));
            session.push_entry(ChatEntry::assistant(""));
            session.push_entry(ChatEntry::tool_call("tc-1", "bash", r#"{"command":"ls"}"#));
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted with Canceled reason.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Canceled,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the dangling ToolCall and empty Assistant are ForcedExclude.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let history = session.history();
        // User entry is not excluded.
        assert_eq!(
            history[0].context_override,
            crate::protocol::ContextOverride::Default
        );
        // Empty Assistant and ToolCall are excluded.
        assert_eq!(
            history[1].context_override,
            crate::protocol::ContextOverride::ForcedExclude
        );
        assert_eq!(
            history[2].context_override,
            crate::protocol::ContextOverride::ForcedExclude
        );
        // Error entry ("Cancelled") is not excluded.
        assert_eq!(
            history[3].context_override,
            crate::protocol::ContextOverride::Default
        );
    }

    // --- Mutant killers: on_stream_token ---

    #[tokio::test]
    async fn on_stream_token_appends_text_to_assistant_entry() {
        // Given a session in streaming state.
        let actor = test_actor();
        let (_sink, _ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When receiving two stream tokens.
        actor.on_stream_token(&StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "Hello".to_owned(),
            is_thinking: false,
        });
        actor.on_stream_token(&StreamToken {
            session_id: session_id.clone(),
            index: 1,
            token: " world".to_owned(),
            is_thinking: false,
        });

        // Then the assistant entry contains the concatenated text.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let assistant_text = session
            .history()
            .iter()
            .find_map(|e| match &e.kind {
                crate::protocol::ChatEntryKind::Assistant(t) => Some(t.clone()),
                _ => None,
            })
            .expect("should have an assistant entry");
        assert_eq!(assistant_text, "Hello world");
    }

    #[tokio::test]
    async fn on_stream_token_keeps_phase_as_streaming() {
        // Given a session in streaming state.
        let actor = test_actor();
        let session_id = {
            let state = actor.state.read();
            state.session.active_session_id().clone()
        };

        // When receiving a token while in Streaming phase.
        {
            let mut state = actor.state.write();
            state.active_session_mut().begin_streaming();
        }
        actor.on_stream_token(&StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "hi".to_owned(),
            is_thinking: false,
        });

        // Then the phase is still Streaming (not changed).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            matches!(session.phase(), SessionPhase::Streaming),
            "expected Streaming phase, got {:?}",
            session.phase()
        );
    }

    #[tokio::test]
    async fn on_stream_token_corrects_sending_phase_to_streaming() {
        // Given a session in Sending state (stream token arrived before phase transition).
        let actor = test_actor();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("go"));
            session.begin_sending();
            state.session.active_session_id().clone()
        };

        // When receiving a stream token while in Sending phase.
        actor.on_stream_token(&StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "response".to_owned(),
            is_thinking: false,
        });

        // Then the phase is corrected to Streaming.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            matches!(session.phase(), SessionPhase::Streaming),
            "expected Streaming phase after correction from Sending, got {:?}",
            session.phase()
        );
    }

    // --- Mutant killers: on_stream_completed should_save, token counting, preserve_assistant ---

    #[tokio::test]
    async fn on_stream_completed_finished_persists_session() {
        // Given an interacted session in streaming state.
        let (actor, store) = super::super::super::helpers::test_actor_with_store(vec![]);
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.mark_interacted();
            session.begin_streaming();
            state.session.active_session_id().clone()
        };
        let (_sink, ctx) = test_context();

        // When handling StreamCompleted with Finished reason.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("response".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the session was persisted (should_save = true for Finished).
        assert!(
            store.last_saved_session(&session_id).is_some(),
            "expected session to be saved after Finished"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_error_persists_session() {
        // Given an interacted session in streaming state.
        let (actor, store) = super::super::super::helpers::test_actor_with_store(vec![]);
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.mark_interacted();
            session.begin_streaming();
            state.session.active_session_id().clone()
        };
        let (_sink, ctx) = test_context();

        // When handling StreamCompleted with Error reason.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Error,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the session was persisted (should_save = true for Error).
        assert!(
            store.last_saved_session(&session_id).is_some(),
            "expected session to be saved after Error"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_canceled_does_not_persist_session() {
        // Given an interacted session in streaming state.
        let (actor, store) = super::super::super::helpers::test_actor_with_store(vec![]);
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.mark_interacted();
            session.begin_streaming();
            state.session.active_session_id().clone()
        };
        let (_sink, ctx) = test_context();

        // When handling StreamCompleted with Canceled reason.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Canceled,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the session was NOT persisted (should_save = false for Canceled).
        assert!(
            store.last_saved_session(&session_id).is_none(),
            "expected session NOT to be saved after Canceled"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_does_not_count_tokens_on_error() {
        // Given a session with a token record in streaming state.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_token_record(TokenRecord {
                timestamp: jiff::Timestamp::now(),
                tokens_sent: 100,
                tokens_received: 0,
                cost: None,
            });
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted with Error reason and some content.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Error,
            assistant_content: Some("some error content".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then no tokens were counted (Error skips token counting).
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
        // Given a session in streaming state with tokens appended via on_stream_token.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("do something"));
            session.begin_streaming();
            state.session.active_session_id().clone()
        };
        // Append tokens so there's a non-empty assistant entry.
        actor.on_stream_token(&StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "I will help".to_owned(),
            is_thinking: false,
        });

        // When handling StreamCompleted with ToolUse reason.
        let event = StreamCompleted {
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
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the assistant entry is preserved (not removed from history).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let has_assistant = session
            .history()
            .iter()
            .any(|e| matches!(&e.kind, crate::protocol::ChatEntryKind::Assistant(t) if t == "I will help"));
        assert!(
            has_assistant,
            "expected assistant entry 'I will help' to be preserved after ToolUse"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_tool_use_counts_tool_call_arguments() {
        // Given a session with a token record (from prompt assembly) in streaming state.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_token_record(TokenRecord {
                timestamp: jiff::Timestamp::now(),
                tokens_sent: 100,
                tokens_received: 0,
                cost: None,
            });
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted(ToolUse) with tool calls.
        let event = StreamCompleted {
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
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the token record includes tokens from both text and tool call arguments.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let ledger = session.token_ledger();
        assert_eq!(ledger.len(), 1);
        // tokens_received should be > just "checking" (2 tokens with tiktoken).
        // It must include the tool call arguments and name.
        assert!(
            ledger[0].tokens_received > 2,
            "expected tokens_received > 2 (text only), got {}",
            ledger[0].tokens_received
        );
    }

    #[tokio::test]
    async fn on_stream_completed_finished_preserves_assistant_entry() {
        // Given a session in streaming state with tokens appended.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.begin_streaming();
            state.session.active_session_id().clone()
        };
        // Append a token to create the assistant entry.
        actor.on_stream_token(&StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "world".to_owned(),
            is_thinking: false,
        });

        // When handling StreamCompleted with Finished reason.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("world".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the assistant entry is preserved in history (not removed).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let has_world = session
            .history()
            .iter()
            .any(|e| matches!(&e.kind, crate::protocol::ChatEntryKind::Assistant(t) if t.contains("world")));
        assert!(
            has_world,
            "expected assistant entry with 'world' to be preserved after Finished"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_canceled_with_complete_tool_loop_does_not_exclude() {
        // Given a session in streaming state with a complete tool loop in history.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
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

        // When handling StreamCompleted with Canceled reason.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Canceled,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then no entries are ForcedExclude.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let history = session.history();
        for entry in history {
            assert_eq!(
                entry.context_override,
                crate::protocol::ContextOverride::Default,
                "expected Default for entry {:?}",
                entry.kind
            );
        }
    }

    // --- Phase 7: Comprehensive transition tests ---

    #[tokio::test]
    async fn on_stream_completed_finished_without_auto_compaction_goes_to_idle() {
        // Given a session in streaming state WITHOUT auto-compaction requested.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.begin_streaming();
            // Do NOT request auto-compaction.
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted with Finished reason.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("response".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the session is in Idle (normal path).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            matches!(session.phase(), SessionPhase::Idle),
            "expected Idle after Finished without auto-compaction, got {:?}",
            session.phase()
        );
    }

    // --- Phase 2: History mutation application hooks ---

    #[tokio::test]
    async fn on_stream_completed_finished_applies_pending_mutations() {
        // Given a session in streaming state with pending mutations.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let (entry_id, session_id) = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            let entry = ChatEntry::assistant("response");
            let entry_id = entry.id.clone();
            session.push_entry(entry);
            session.begin_streaming();
            // Queue a mutation to exclude the assistant entry.
            session.queue_mutations(vec![
                crate::feat::session::history_mutation::HistoryMutation::SetContextOverride {
                    entry_id: entry_id.clone(),
                    value: crate::protocol::ContextOverride::ForcedExclude,
                },
            ]);
            (entry_id, state.session.active_session_id().clone())
        };

        // When handling StreamCompleted with Finished reason.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("response".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the mutation was applied — the assistant entry is now ForcedExclude.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let assistant = session
            .history()
            .iter()
            .find(|e| e.id == entry_id)
            .expect("assistant entry exists");
        assert_eq!(
            assistant.context_override,
            crate::protocol::ContextOverride::ForcedExclude,
            "expected mutation to be applied at stream completion"
        );

        // And a HistoryAppended event was emitted.
        let events = sink.events();
        let has_history = events.iter().any(
            |e| matches!(e, Event::HistoryAppended(payload) if payload.session_id == session_id),
        );
        assert!(
            has_history,
            "expected HistoryAppended event after mutation application"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_error_applies_pending_mutations() {
        // Given a session in streaming state with pending mutations.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
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
                },
            ]);
            (entry_id, state.session.active_session_id().clone())
        };

        // When handling StreamCompleted with Error reason.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Error,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the mutation was applied.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let assistant = session
            .history()
            .iter()
            .find(|e| e.id == entry_id)
            .expect("assistant entry exists");
        assert_eq!(
            assistant.context_override,
            crate::protocol::ContextOverride::ForcedExclude,
            "expected mutation to be applied at stream error"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_canceled_applies_pending_mutations() {
        // Given a session in streaming state with pending mutations.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
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
                },
            ]);
            (entry_id, state.session.active_session_id().clone())
        };

        // When handling StreamCompleted with Canceled reason.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Canceled,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the mutation was applied.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let assistant = session
            .history()
            .iter()
            .find(|e| e.id == entry_id)
            .expect("assistant entry exists");
        assert_eq!(
            assistant.context_override,
            crate::protocol::ContextOverride::ForcedExclude,
            "expected mutation to be applied at stream canceled"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_tool_use_does_not_apply_mutations() {
        // Given a session in streaming state with pending mutations.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
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
                },
            ]);
            (entry_id, state.session.active_session_id().clone())
        };

        // When handling StreamCompleted with ToolUse reason.
        let event = StreamCompleted {
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
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the mutation was NOT applied (ToolUse defers to tool batch).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let assistant = session
            .history()
            .iter()
            .find(|e| e.id == entry_id)
            .expect("assistant entry exists");
        assert_eq!(
            assistant.context_override,
            crate::protocol::ContextOverride::Default,
            "expected mutation to NOT be applied for ToolUse reason"
        );

        // And the mutations are still in the queue (not drained).
        // Note: we can't easily check the queue directly from outside,
        // but the entry not being modified is sufficient proof.
    }

    // --- Hybrid token counting tests ---

    #[tokio::test]
    async fn on_stream_completed_provider_tokens_used_directly() {
        // Given a session with a token record in streaming state.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_token_record(TokenRecord {
                timestamp: jiff::Timestamp::now(),
                tokens_sent: 100,
                tokens_received: 0,
                cost: None,
            });
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted with provider_completion_tokens set.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("short".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: Some(5000),
            thinking_content: Some("very long thinking content here".to_owned()),
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then tokens_received equals the provider value (not local count).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let ledger = session.token_ledger();
        assert_eq!(ledger.len(), 1);
        assert_eq!(
            ledger[0].tokens_received, 5000,
            "expected provider-reported 5000, got {}",
            ledger[0].tokens_received
        );
    }

    #[tokio::test]
    async fn on_stream_completed_local_fallback_includes_thinking() {
        // Given a session with a token record in streaming state.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_token_record(TokenRecord {
                timestamp: jiff::Timestamp::now(),
                tokens_sent: 100,
                tokens_received: 0,
                cost: None,
            });
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted without provider tokens but with thinking.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("short".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: Some("a substantial amount of reasoning text".to_owned()),
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then tokens_received includes thinking tokens (> just "short").
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let ledger = session.token_ledger();
        assert_eq!(ledger.len(), 1);
        assert!(
            ledger[0].tokens_received > 2,
            "expected tokens_received > 2 (text only), got {} — thinking not counted",
            ledger[0].tokens_received
        );
    }

    #[tokio::test]
    async fn on_stream_completed_local_fallback_without_thinking_backward_compat() {
        // Given a session with a token record in streaming state.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_token_record(TokenRecord {
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
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("response text".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

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
    async fn on_stream_completed_provider_tokens_preferred_over_local() {
        // Given a session with a token record in streaming state.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_token_record(TokenRecord {
                timestamp: jiff::Timestamp::now(),
                tokens_sent: 100,
                tokens_received: 0,
                cost: None,
            });
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When both provider tokens and thinking content are present.
        // Local count would be much higher than 9999 due to thinking.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("short".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: Some(9999),
            thinking_content: Some(
                "extremely long thinking content that would produce many tokens".to_owned(),
            ),
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the provider value wins (not local counting).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let ledger = session.token_ledger();
        assert_eq!(ledger[0].tokens_received, 9999);
    }
}
