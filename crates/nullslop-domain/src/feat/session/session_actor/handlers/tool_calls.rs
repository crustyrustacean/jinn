//! Tool call state tracking handlers — manage tool call lifecycle during streaming.
//!
//! Handles the full tool call lifecycle: creation via streaming, argument assembly,
//! execution tracking, result collection, and batch completion routing.

use crate::common::actor::ActorContext;
use crate::feat::context::assemble::assemble_prompt;
use crate::feat::provider::protocol::command::SendToLlmProvider;
use crate::feat::session::token_stats::TokenRecord;
use crate::feat::tools_actor::protocol::event::{
    ToolBatchCompleted, ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted,
    ToolExecutionOutput, ToolExecutionStarted, ToolUseStarted,
};

use super::super::SessionPersistenceActor;
use crate::protocol::Command;

impl SessionPersistenceActor {
    /// Begins tracking a streaming tool call.
    pub(in crate::feat::session::session_actor) fn on_tool_use_started(
        &self,
        event: &ToolUseStarted,
    ) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        session.begin_tool_call(event.index, &event.id, &event.name);
    }

    /// Finalizes the tool call entry with complete arguments.
    ///
    /// The placeholder entry was created by `on_tool_use_started`. This updates
    /// it in place with the full arguments string, avoiding a duplicate entry.
    pub(in crate::feat::session::session_actor) fn on_tool_call_received(
        &self,
        event: &ToolCallReceived,
    ) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        session.finalize_tool_call(
            &event.tool_call.id,
            &event.tool_call.name,
            &event.tool_call.arguments,
        );
    }

    /// Appends a partial JSON delta to a streaming tool call.
    pub(in crate::feat::session::session_actor) fn on_tool_call_streaming(
        &self,
        event: &ToolCallStreaming,
    ) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        if let Err(e) = session.append_tool_call_delta(event.index, &event.partial_json) {
            tracing::error!(err = ?e, "failed to append tool call delta");
        }
    }

    /// Pushes a tool result entry into the session history.
    pub(in crate::feat::session::session_actor) async fn on_tool_execution_completed(
        &self,
        event: &ToolExecutionCompleted,
        ctx: &ActorContext,
    ) {
        let total_tokens = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&event.session_id);
            session.finalize_tool_result(
                &event.result.tool_call_id,
                &event.result.name,
                &event.result.content,
                event.result.success,
                event.result.full_content.clone(),
                event.result.truncation.clone(),
            );
            super::super::helpers::estimate_total_tokens(session)
        };

        super::super::helpers::emit_history_appended(ctx, &event.session_id, total_tokens);
        self.save_active_session(&event.session_id).await;
    }

    /// Creates a pending ToolResult entry when a streaming tool starts executing.
    pub(in crate::feat::session::session_actor) fn on_tool_execution_started(
        &self,
        event: &ToolExecutionStarted,
    ) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        session.begin_tool_result(&event.tool_call_id, &event.name);
    }

    /// Appends incremental output to a pending ToolResult entry.
    pub(in crate::feat::session::session_actor) fn on_tool_execution_output(
        &self,
        event: &ToolExecutionOutput,
    ) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        session.append_tool_result_output(&event.tool_call_id, &event.output);
    }

    /// All tools in a batch have finished — route the continuation through
    /// context assembly so token counting and prompt strategy apply.
    ///
    /// By this point, the session history already contains `ToolCall`,
    /// `ToolResult`, and `Assistant` entries from earlier event handlers,
    /// and the session is already in sending state (set by `on_stream_completed`
    /// for the `ToolUse` reason). We just need to assemble the prompt via
    /// the full session history.
    pub(in crate::feat::session::session_actor) fn on_tool_batch_completed(
        &self,
        event: &ToolBatchCompleted,
        ctx: &ActorContext,
    ) {
        tracing::trace!(
            session_id = ?event.session_id,
            result_count = event.results.len(),
            "on_tool_batch_completed"
        );

        // Check soft cancel: if requested, end the turn instead of continuing.
        let soft_cancelled = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&event.session_id);
            session.take_soft_cancel()
        };

        if soft_cancelled {
            // Soft cancel: don't assemble prompt. Session is already in
            // Sending phase; it will return to Idle when the QueueActor sees
            // the SessionPhaseChanged event. But we need to explicitly end the
            // turn — finish sending and go to Idle.
            let (old_phase, new_phase) = {
                let mut state = self.state.write();
                let session = state.session_mut_or_create(&event.session_id);
                let old_phase = session.phase();
                // Finish the sending phase to return to Idle.
                session.finish_sending();
                (old_phase, session.phase())
            };
            super::super::helpers::emit_phase_changed(ctx, &event.session_id, old_phase, new_phase);
            return;
        }

        // Assemble the prompt directly and emit SendToLlmProvider.
        // Note: the session is already in sending state, set by on_stream_completed(ToolUse).
        let workflow_overrides: Option<crate::feat::context::assemble::AssemblyOverrides> = {
            let state = self.state.read();
            let session = state.session(&event.session_id);
            if session.is_workflow() {
                session.core.workflow_overrides.clone()
            } else {
                None
            }
        };
        let assembled = {
            let guard = self.state.read();
            assemble_prompt(&guard, &event.session_id, &self.counter, workflow_overrides.as_ref())
        };

        let (old_phase, new_phase) = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&event.session_id);
            let old_phase = session.phase();
            session.begin_streaming();
            session.push_token_record(TokenRecord {
                timestamp: jiff::Timestamp::now(),
                tokens_sent: assembled.estimated_tokens(),
                tokens_received: 0,
                cost: None,
            });
            session.set_context_size(assembled.estimated_tokens());
            (old_phase, session.phase())
        };
        super::super::helpers::emit_phase_changed(ctx, &event.session_id, old_phase, new_phase);

        let provider_id = {
            let state = self.state.read();
            let model = state.session(&event.session_id).profile().model.clone();
            if model == crate::feat::provider_infra::NO_PROVIDER_ID {
                None
            } else {
                Some(model)
            }
        };

        let estimated_tokens = assembled.estimated_tokens();

        if let Err(e) = ctx.send_command(Command::SendToLlmProvider(SendToLlmProvider {
            session_id: event.session_id.clone(),
            messages: assembled.messages,
            provider_id,
            estimated_tokens,
            tool_definitions: assembled.tool_definitions,
        })) {
            tracing::warn!(
                err = ?e,
                "session-actor failed to emit SendToLlmProvider from tool batch completion"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::super::super::helpers::{test_actor, test_context};
    use crate::feat::provider::protocol::event::{StreamCompleted, StreamCompletedReason};
    use crate::feat::session::chat_session::SessionPhase;
    use crate::feat::session::token_stats::TokenRecord;
    use crate::feat::tools_actor::protocol::event::ToolBatchCompleted;
    use crate::feat::tools_actor::tool_types::{ToolCall, ToolResult};
    use crate::protocol::{ChatEntry, Command, Event};

    #[tokio::test]
    async fn on_tool_batch_completed_emits_send_to_llm_provider() {
        // Given a session with tool call and result entries in history.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("list files"));
            session.push_entry(ChatEntry::assistant("checking"));
            session.push_entry(ChatEntry::tool_call("tc-1", "bash", r#"{"command":"ls"}"#));
            session.push_entry(ChatEntry::assistant("here are the files"));
            state.session.active_session_id().clone()
        };

        // When handling ToolBatchCompleted.
        let event = ToolBatchCompleted {
            session_id: session_id.clone(),
            results: vec![ToolResult {
                tool_call_id: "tc-1".to_owned(),
                name: "bash".to_owned(),
                content: "file1.txt".to_owned(),
                success: true,
                full_content: None,
                truncation: None,
            }],
        };
        actor.on_tool_batch_completed(&event, &ctx);

        // Then a SendToLlmProvider command was emitted.
        let commands = sink.commands();
        let send = commands
            .iter()
            .find(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(
            send.is_some(),
            "expected SendToLlmProvider command to be emitted"
        );
    }

    #[tokio::test]
    async fn on_tool_batch_completed_transitions_session_to_sending() {
        // Given a session in sending state (set by on_stream_completed for ToolUse).
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            session.finish_streaming(true);
            session.begin_sending();
            state.session.active_session_id().clone()
        };

        // When handling ToolBatchCompleted.
        let event = ToolBatchCompleted {
            session_id: session_id.clone(),
            results: vec![],
        };
        actor.on_tool_batch_completed(&event, &ctx);

        // Then the session transitions to streaming (assemble + send are now synchronous).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(matches!(session.phase(), SessionPhase::Streaming));
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
            tool_calls: Some(vec![ToolCall {
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
    async fn on_tool_batch_completed_with_soft_cancel_goes_to_idle() {
        // Given a session in sending state with soft cancel requested.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            session.finish_streaming(true);
            session.begin_sending();
            session.request_soft_cancel();
            state.session.active_session_id().clone()
        };

        // When handling ToolBatchCompleted.
        let event = ToolBatchCompleted {
            session_id: session_id.clone(),
            results: vec![ToolResult {
                tool_call_id: "tc-1".to_owned(),
                name: "bash".to_owned(),
                content: "file1.txt".to_owned(),
                success: true,
                full_content: None,
                truncation: None,
            }],
        };
        actor.on_tool_batch_completed(&event, &ctx);

        // Then the session is in Idle phase.
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
    async fn on_tool_execution_completed_emits_history_appended() {
        // Given a session actor with a pending tool result.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("run it"));
            session.push_entry(ChatEntry::tool_call("tc-1", "bash", r#"{\"command\":\"ls\"}"#));
            session.begin_tool_result("tc-1", "bash");
            state.session.active_session_id().clone()
        };

        // When handling ToolExecutionCompleted.
        let event = crate::feat::tools_actor::protocol::event::ToolExecutionCompleted {
            session_id: session_id.clone(),
            result: crate::feat::tools_actor::tool_types::ToolResult {
                tool_call_id: "tc-1".to_owned(),
                name: "bash".to_owned(),
                content: "file1.txt".to_owned(),
                success: true,
                full_content: None,
                truncation: None,
            },
        };
        actor.on_tool_execution_completed(&event, &ctx).await;

        // Then a HistoryAppended event was emitted.
        let events = sink.events();
        let has_history = events
            .iter()
            .any(|e| matches!(e, Event::HistoryAppended(payload) if payload.session_id == session_id));
        assert!(has_history, "expected HistoryAppended event after tool execution completed");
    }
}
