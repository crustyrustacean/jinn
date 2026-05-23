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
    pub(in crate::feat::session::session_actor) async fn on_stream_completed(
        &self,
        event: &StreamCompleted,
        ctx: &ActorContext,
    ) {
        let should_save = event.reason == StreamCompletedReason::Finished
            || event.reason == StreamCompletedReason::Error;

        // Count output tokens off the async thread
        // Count both text content and tool call arguments (JSON) which are a
        // significant portion of the model's output.
        let output_tokens: Option<tokio::task::JoinHandle<u32>> = if event.reason
            != StreamCompletedReason::Canceled
            && event.reason != StreamCompletedReason::Error
        {
            event.assistant_content.as_ref().map(|content| {
                let content = content.clone();
                let tool_calls = event.tool_calls.clone();
                let counter = self.counter;
                tokio::task::spawn_blocking(move || {
                    let mut tokens = counter.count(&content) as u32;
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

        // Await token counting outside the lock.
        let output_tokens: Option<u32> = match output_tokens {
            Some(handle) => Some(handle.await.unwrap_or_else(|e| {
                tracing::warn!(err = ?e, "spawn_blocking panicked during output token counting");
                0
            })),
            None => None,
        };

        let (old_phase, new_phase, total_tokens);
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

            // Tool use means the conversation continues — transition to sending
            // so the indicator shows activity while awaiting the followup.
            // Unless soft-cancel was requested, in which case end the turn.
            if event.reason == StreamCompletedReason::ToolUse {
                if session.take_soft_cancel() {
                    // Soft cancel: end turn, go to Idle.
                    // QueueActor will see SessionPhaseChanged(Idle) and pop CompactionNeeded.
                } else {
                    session.begin_sending();
                }
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
                        _ => None,
                    })
                    .collect();
                let drained_text = display_texts.join("\n");
                if !drained_text.is_empty() {
                    session.chat_input_mut().replace_all(drained_text);
                }
            }

            new_phase = session.phase();
            total_tokens = super::super::helpers::estimate_total_tokens(session);
        }

        super::super::helpers::emit_phase_changed(ctx, &event.session_id, old_phase, new_phase);
        super::super::helpers::emit_history_appended(ctx, &event.session_id, total_tokens);

        // Persist session after stream finishes (not on cancel).
        if should_save {
            self.save_active_session(&event.session_id).await;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::super::super::helpers::{test_actor, test_context};
    use crate::feat::provider::protocol::event::{StreamCompleted, StreamCompletedReason};
    use crate::feat::session::chat_session::SessionPhase;
    use crate::protocol::{ChatEntry, Command, Event};

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
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the queue is empty and the message text is in the input buffer.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert_eq!(session.queue_len(), 0);
        assert_eq!(session.chat_input().text(), "queued message");
    }

    #[tokio::test]
    async fn on_stream_completed_tool_use_with_soft_cancel_goes_to_idle() {
        // Given a session in streaming state with soft cancel requested.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            session.request_soft_cancel();
            state.session.active_session_id().clone()
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
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the session is in Idle phase (not Sending).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            matches!(session.phase(), SessionPhase::Idle),
            "expected Idle after soft cancel, got {:?}",
            session.phase()
        );

        // And no SendToLlmProvider was emitted.
        let commands = sink.commands();
        let has_send = commands
            .iter()
            .any(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(!has_send, "expected no SendToLlmProvider after soft cancel");
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
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then a HistoryAppended event was emitted.
        let events = sink.events();
        let has_history = events
            .iter()
            .any(|e| matches!(e, Event::HistoryAppended(payload) if payload.session_id == session_id));
        assert!(has_history, "expected HistoryAppended event after stream completed");
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
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then a HistoryAppended event was emitted.
        let events = sink.events();
        let has_history = events
            .iter()
            .any(|e| matches!(e, Event::HistoryAppended(payload) if payload.session_id == session_id));
        assert!(has_history, "expected HistoryAppended event after stream error");
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
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then a HistoryAppended event was emitted.
        let events = sink.events();
        let has_history = events
            .iter()
            .any(|e| matches!(e, Event::HistoryAppended(payload) if payload.session_id == session_id));
        assert!(has_history, "expected HistoryAppended event after stream canceled");
    }
}
