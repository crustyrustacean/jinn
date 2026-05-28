//! Streaming lifecycle handlers — manage token streaming and stream completion.
//!
//! Handles appending individual tokens to the assistant entry (including
//! reasoning/thinking tokens), and finalizing the stream with token accounting
//! and queue draining on `StreamCompleted`.

use crate::common::actor::ActorContext;
use crate::feat::compaction_actor::protocol::command::CompactContext;
use crate::feat::context::strategy::token_estimator::TokenCounter;
use crate::feat::provider::protocol::event::{StreamCompleted, StreamCompletedReason, StreamToken};

use crate::protocol::{ChatEntry, Command};

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

        let (old_phase, new_phase, should_emit_compact_context);
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

            // For ToolUse: do NOT consume auto-compaction flag here. The tool
            // batch is still executing. Let on_tool_batch_completed handle the
            // flag after tool results are in. Peek at the flag to know if
            // compaction is pending.
            // For Finished/Error/Canceled: consume the flag to clean up.
            let auto_compaction_requested = if event.reason == StreamCompletedReason::ToolUse {
                session.is_auto_compaction_requested()
            } else {
                session.take_auto_compaction_requested()
            };

            // Tool use means the conversation continues — always transition to sending
            // so the tool loop runs. Auto-compaction is handled later in
            // on_tool_batch_completed.
            if event.reason == StreamCompletedReason::ToolUse {
                session.begin_sending();
            }

            // If auto-compaction requested (non-ToolUse),
            // skip Idle entirely and transition directly to Compacting.
            // For ToolUse, this is deferred — on_tool_batch_completed will handle
            // the flag after tools finish.
            let mut should_emit_compact_context_local = false;
            if auto_compaction_requested
                && event.reason != StreamCompletedReason::ToolUse
            {
                session.core.ephemeral.phase = SessionPhase::Compacting;
                should_emit_compact_context_local = true;
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
            should_emit_compact_context = should_emit_compact_context_local;
        }

        super::super::helpers::emit_phase_changed(ctx, &event.session_id, old_phase, new_phase);
        super::super::helpers::emit_history_appended(ctx, &event.session_id);

        // If we transitioned directly to Compacting, emit CompactContext to kick off
        // the compaction actor.
        if should_emit_compact_context
            && let Err(e) = ctx.send_command(Command::CompactContext(CompactContext {
                session_id: event.session_id.clone(),
                compact_all: false,
            }))
        {
            tracing::warn!(err = ?e, "failed to emit CompactContext after soft cancel");
        }

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
    use crate::feat::provider::protocol::event::{StreamCompleted, StreamCompletedReason, StreamToken};
    use crate::feat::session::chat_session::SessionPhase;
    use crate::feat::session::token_stats::TokenRecord;
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
    async fn on_stream_completed_tool_use_with_auto_compaction_begins_sending() {
        // Given a session in streaming state with auto-compaction requested.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            session.request_auto_compaction();
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

        // Then the session is in Sending phase (tools still execute).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            matches!(session.phase(), SessionPhase::Sending),
            "expected Sending after auto-compaction during ToolUse, got {:?}",
            session.phase()
        );

        // And the auto-compaction flag is still set (not consumed yet — peeked for ToolUse).
        assert!(
            session.is_auto_compaction_requested(),
            "expected auto-compaction flag to still be set"
        );

        // And no SendToLlmProvider was emitted.
        let commands = sink.commands();
        let has_send = commands
            .iter()
            .any(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(!has_send, "expected no SendToLlmProvider after auto-compaction peek");
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
    async fn on_stream_completed_tool_use_with_auto_compaction_defers_compaction() {
        // Given a session in streaming state with auto-compaction requested.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            session.request_auto_compaction();
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted with ToolUse reason and auto-compaction.
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

        // Then the session is in Sending phase (tools execute before compaction).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            matches!(session.phase(), SessionPhase::Sending),
            "expected Sending after ToolUse with auto-compaction, got {:?}",
            session.phase()
        );

        // And the auto-compaction flag is still set (not consumed yet).
        assert!(
            session.is_auto_compaction_requested(),
            "expected auto-compaction flag to still be set after ToolUse"
        );

        // And no CompactContext was emitted (compaction deferred).
        let commands = sink.commands();
        let has_compact = commands
            .iter()
            .any(|c| matches!(c, Command::CompactContext(_)));
        assert!(
            !has_compact,
            "expected no CompactContext — compaction should be deferred"
        );

        // And no SendToLlmProvider was emitted.
        let has_send = commands
            .iter()
            .any(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(!has_send, "expected no SendToLlmProvider");
    }

    #[tokio::test]
    async fn on_stream_completed_finished_with_auto_compaction_transitions_to_compacting() {
        // Given a session in streaming state with auto-compaction requested.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.begin_streaming();
            session.request_auto_compaction();
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted with Finished reason and auto-compaction.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("response".to_owned()),
            tool_calls: None,
            cost: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the session transitions directly to Compacting (NOT Idle).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            matches!(session.phase(), SessionPhase::Compacting),
            "expected Compacting after Finished with auto-compaction, got {:?}",
            session.phase()
        );

        // And CompactContext was emitted.
        let commands = sink.commands();
        let has_compact = commands
            .iter()
            .any(|c| matches!(c, Command::CompactContext(_)));
        assert!(
            has_compact,
            "expected CompactContext command after Finished with auto-compaction"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_then_tool_batch_auto_compaction_full_flow() {
        // Given a session streaming with auto-compaction requested.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            session.request_auto_compaction();
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted(ToolUse) then ToolBatchCompleted.
        let stream_event = StreamCompleted {
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
        actor.on_stream_completed(&stream_event, &ctx).await;

        // Then session is in Sending (tool loop continues).
        {
            let state = actor.state.read();
            let session = state.session.get(&session_id).expect("session exists");
            assert!(matches!(session.phase(), SessionPhase::Sending));
        }

        // When tool batch completes.
        let batch_event = crate::feat::tools_actor::protocol::event::ToolBatchCompleted {
            session_id: session_id.clone(),
            results: vec![crate::feat::tools_actor::tool_types::ToolResult {
                tool_call_id: "tc-1".to_owned(),
                name: "bash".to_owned(),
                content: "file1.txt".to_owned(),
                success: true,
                full_content: None,
                truncation: None,
            }],
        };
        actor.on_tool_batch_completed(&batch_event, &ctx);

        // Then session is in Compacting (NOT Idle — the whole point of the fix).
        {
            let state = actor.state.read();
            let session = state.session.get(&session_id).expect("session exists");
            assert!(
                matches!(session.phase(), SessionPhase::Compacting),
                "expected Compacting after tool batch auto-compaction, got {:?}",
                session.phase()
            );
        }

        // And CompactContext was emitted.
        let commands = sink.commands();
        let has_compact = commands
            .iter()
            .any(|c| matches!(c, Command::CompactContext(_)));
        assert!(has_compact, "expected CompactContext during full flow");

        // And no SendToLlmProvider was emitted throughout.
        let has_send = commands
            .iter()
            .any(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(!has_send, "expected no SendToLlmProvider during full flow");
    }

    #[tokio::test]
    async fn on_stream_completed_canceled_force_excludes_dangling_tool_calls() {
        // Given a session in streaming state with dangling tool calls in history.
        let actor = test_actor();
        let (sink, ctx) = test_context();
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
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the dangling ToolCall and empty Assistant are ForcedExclude.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let history = session.history();
        // User entry is not excluded.
        assert_eq!(history[0].context_override, crate::protocol::ContextOverride::Default);
        // Empty Assistant and ToolCall are excluded.
        assert_eq!(history[1].context_override, crate::protocol::ContextOverride::ForcedExclude);
        assert_eq!(history[2].context_override, crate::protocol::ContextOverride::ForcedExclude);
        // Error entry ("Cancelled") is not excluded.
        assert_eq!(history[3].context_override, crate::protocol::ContextOverride::Default);

        // And a CompactContext was not emitted.
        let commands = sink.commands();
        let has_compact = commands
            .iter()
            .any(|c| matches!(c, Command::CompactContext(_)));
        assert!(!has_compact, "expected no CompactContext after cancel");
    }

    // --- Mutant killers: on_stream_token ---

    #[tokio::test]
    async fn on_stream_token_appends_text_to_assistant_entry() {
        // Given a session in streaming state.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
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
        let (actor, store) =
            super::super::super::helpers::test_actor_with_store(vec![]);
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
        let (actor, store) =
            super::super::super::helpers::test_actor_with_store(vec![]);
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
        let (actor, store) =
            super::super::super::helpers::test_actor_with_store(vec![]);
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
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the assistant entry is preserved in history (not removed).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        let has_world = session
            .history()
            .iter()
            .any(|e| matches!(&e.kind, crate::protocol::ChatEntryKind::Assistant(t) if t.contains("world")));
        assert!(has_world, "expected assistant entry with 'world' to be preserved after Finished");
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
            session.push_entry(
                ChatEntry::tool_result(
                    "tc-1",
                    "bash",
                    "file.txt",
                    crate::feat::session::tool_result_status::ToolResultStatus::Success,
                ),
            );
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
        let (sink, ctx) = test_context();
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

        // And no CompactContext was emitted.
        let commands = sink.commands();
        let has_compact = commands
            .iter()
            .any(|c| matches!(c, Command::CompactContext(_)));
        assert!(!has_compact, "expected no CompactContext");
    }

    #[tokio::test]
    async fn on_stream_completed_error_with_auto_compaction_goes_to_compacting() {
        // Given a session in streaming state with auto-compaction requested.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            session.request_auto_compaction();
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted with Error reason and auto-compaction.
        // Error is a turn boundary, so auto-compaction is honored.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Error,
            assistant_content: None,
            tool_calls: None,
            cost: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the session transitions to Compacting (auto-compaction honored even on Error).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            matches!(session.phase(), SessionPhase::Compacting),
            "expected Compacting after Error with auto-compaction, got {:?}",
            session.phase()
        );

        // And the auto-compaction flag is consumed (cleared).
        assert!(
            !session.is_auto_compaction_requested(),
            "expected auto-compaction flag to be consumed after Error"
        );

        // And CompactContext was emitted.
        let commands = sink.commands();
        let has_compact = commands
            .iter()
            .any(|c| matches!(c, Command::CompactContext(_)));
        assert!(
            has_compact,
            "expected CompactContext after Error with auto-compaction"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_canceled_with_auto_compaction_goes_to_compacting() {
        // Given a session in streaming state with auto-compaction requested.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.begin_streaming();
            session.request_auto_compaction();
            state.session.active_session_id().clone()
        };

        // When handling StreamCompleted with Canceled reason and auto-compaction.
        let event = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Canceled,
            assistant_content: None,
            tool_calls: None,
            cost: None,
        };
        actor.on_stream_completed(&event, &ctx).await;

        // Then the session transitions to Compacting (NOT Idle).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            matches!(session.phase(), SessionPhase::Compacting),
            "expected Compacting after Canceled with auto-compaction, got {:?}",
            session.phase()
        );

        // And CompactContext was emitted.
        let commands = sink.commands();
        let has_compact = commands
            .iter()
            .any(|c| matches!(c, Command::CompactContext(_)));
        assert!(
            has_compact,
            "expected CompactContext after Canceled with auto-compaction"
        );
    }

    #[tokio::test]
    async fn on_stream_completed_auto_compaction_flag_consumed_on_finished() {
        // Given a session in streaming state with auto-compaction requested.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("hello"));
            session.begin_streaming();
            session.request_auto_compaction();
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

        // Then the auto-compaction flag is consumed (cleared).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            !session.is_auto_compaction_requested(),
            "expected auto-compaction flag to be consumed after Finished"
        );
    }
}
