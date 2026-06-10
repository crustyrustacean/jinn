//! Tool call state tracking handlers - manage tool call lifecycle during streaming.
//!
//! Handles the full tool call lifecycle: creation via streaming, argument assembly,
//! execution tracking, result collection, and batch completion routing.

use crate::common::actor::ActorContext;
use crate::feat::context::assemble::assemble_prompt;
use crate::feat::provider::protocol::command::SendToLlmProvider;
use crate::feat::session::chat_entry::PinPosition;
use crate::feat::session::token_stats::TokenRecord;
use crate::feat::tools_actor::protocol::event::{
    ToolBatchCompleted, ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted,
    ToolExecutionOutput, ToolExecutionStarted, ToolUseStarted,
};

use super::super::SessionPersistenceActor;
use crate::protocol::{Command, Event};

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
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&event.session_id);
            session.finalize_tool_result(
                &event.result.tool_call_id,
                &event.result.name,
                &event.result.content,
                event.result.success,
                event.result.full_content.clone(),
                event.result.truncation.clone(),
                event.result.pin_position.map(PinPosition::from),
            );
        };

        super::super::helpers::emit_history_appended(ctx, &event.session_id);
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

    /// Drains pending history mutations and steering buffer entries, emitting
    /// ContextOverrideChanged events for any modified entries.
    fn apply_pending_mutations_and_steering(
        &self,
        session_id: &crate::protocol::SessionId,
        ctx: &ActorContext,
    ) {
        // Drain and apply pending history mutations.
        let changed = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(session_id);
            let (count, changed) = session.drain_and_apply_pending_mutations();
            if count > 0 {
                tracing::debug!(
                    session_id = %session_id,
                    applied = count,
                    "applied pending history mutations"
                );
            }
            changed
        };
        // Emit ContextOverrideChanged events outside the write lock.
        for entry_id in changed {
            if let Err(e) = ctx.send_event(Event::ContextOverrideChanged(
                crate::feat::context::protocol::event::ContextOverrideChanged {
                    session_id: session_id.clone(),
                    entry_id,
                },
            )) {
                tracing::warn!(err = ?e, "failed to emit ContextOverrideChanged");
            }
        }

        // Drain any pending steering fragments into history.
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(session_id);
            if let Some(entry) = session.steering_buffer_mut().drain_into_entry() {
                let entry_id = entry.id.clone();
                let index = session.push_entry(entry);
                tracing::debug!(
                    session_id = %session_id,
                    entry_id = %entry_id,
                    history_index = index,
                    "drained steering entry into history at tool-batch boundary"
                );
            }
        }
    }

    /// Assembles the continuation prompt, transitions to streaming phase,
    /// and emits the SendToLlmProvider command.
    fn assemble_and_send_continuation(
        &self,
        session_id: &crate::protocol::SessionId,
        assembly_overrides: Option<&crate::feat::context::assemble::AssemblyOverrides>,
        ctx: &ActorContext,
    ) {
        let assembled = {
            let guard = self.state.read();
            assemble_prompt(&guard, session_id, &self.counter, assembly_overrides)
        };

        let (old_phase, new_phase) = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(session_id);
            let old_phase = session.phase();
            session.begin_streaming();
            session.push_token_record(TokenRecord {
                timestamp: jiff::Timestamp::now(),
                tokens_sent: assembled.estimated_tokens(),
                tokens_received: 0,
                cost: None,
            });

            (old_phase, session.phase())
        };
        super::super::helpers::emit_phase_changed(ctx, session_id, old_phase, new_phase);

        let provider_id = {
            let state = self.state.read();
            let model = state.session(session_id).profile().model.clone();
            if model == crate::feat::provider_infra::NO_PROVIDER_ID {
                None
            } else {
                Some(model)
            }
        };

        let estimated_tokens = assembled.estimated_tokens();

        if let Err(e) = ctx.send_command(Command::SendToLlmProvider(SendToLlmProvider {
            session_id: session_id.clone(),
            messages: assembled.messages,
            provider_id,
            estimated_tokens,
            tool_definitions: assembled.tool_definitions,
            dispatched_at: jiff::Timestamp::now(),
        })) {
            tracing::warn!(
                err = ?e,
                "session-actor failed to emit SendToLlmProvider from tool batch completion"
            );
        }
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

        // If tool loop is disabled, end the turn instead of continuing.
        // This is used by judge verdict tools to prevent infinite tool-call loops.
        // finish_sending_via_machine delegates to on_tool_batch_completed() which
        // reads and clears the tool_loop_disabled flag from the machine.
        let tool_loop_disabled = {
            let state = self.state.read();
            let session = state.session(&event.session_id);
            session.is_tool_loop_disabled()
        };

        if tool_loop_disabled {
            let (old_phase, new_phase) = {
                let mut state = self.state.write();
                let session = state.session_mut_or_create(&event.session_id);
                let old_phase = session.phase();
                session.finish_sending_via_machine();
                (old_phase, session.phase())
            };
            super::super::helpers::emit_phase_changed(ctx, &event.session_id, old_phase, new_phase);
            return;
        }

        self.apply_pending_mutations_and_steering(&event.session_id, ctx);

        let assembly_overrides = {
            let state = self.state.read();
            let session = state.session(&event.session_id);
            session
                .is_automated()
                .then(|| session.core.assembly_overrides.clone())
                .flatten()
        };

        self.assemble_and_send_continuation(&event.session_id, assembly_overrides.as_ref(), ctx);
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
    use super::super::super::helpers::{test_actor, test_context};
    use crate::feat::provider::protocol::event::{StreamCompleted, StreamCompletedReason};
    use crate::feat::session::phase_machine::PhaseKind;
    use crate::feat::session::token_stats::TokenRecord;
    use crate::feat::session::tool_result_status::ToolResultStatus;
    use crate::feat::tools_actor::protocol::event::{
        ToolBatchCompleted, ToolCallReceived, ToolCallStreaming, ToolExecutionOutput,
        ToolExecutionStarted, ToolUseStarted,
    };
    use crate::feat::tools_actor::tool_types::{ToolCall, ToolResult};
    use crate::protocol::{ChangeSource, ChatEntry, ChatEntryKind, Command, Event};

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
                pin_position: None,
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
        assert!(matches!(session.phase(), PhaseKind::Streaming));
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
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
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
    async fn on_tool_execution_completed_emits_history_appended() {
        // Given a session actor with a pending tool result.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("run it"));
            session.push_entry(ChatEntry::tool_call(
                "tc-1",
                "bash",
                r#"{\"command\":\"ls\"}"#,
            ));
            session.begin_sending();
            session.begin_streaming();
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
                pin_position: None,
            },
        };
        actor.on_tool_execution_completed(&event, &ctx).await;

        // Then a HistoryAppended event was emitted.
        let events = sink.events();
        let has_history = events.iter().any(
            |e| matches!(e, Event::HistoryAppended(payload) if payload.session_id == session_id),
        );
        assert!(
            has_history,
            "expected HistoryAppended event after tool execution completed"
        );
    }

    #[tokio::test]
    async fn on_tool_batch_completed_skips_send_when_tool_loop_disabled() {
        // Given a session in sending state with tool_loop_disabled set.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            session.finish_streaming(true);
            session.begin_sending();
            session.set_tool_loop_disabled();
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
                pin_position: None,
            }],
        };
        actor.on_tool_batch_completed(&event, &ctx);

        // Then the session transitions to Idle.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(
            matches!(session.phase(), PhaseKind::Idle),
            "expected Idle after tool_loop_disabled, got {:?}",
            session.phase()
        );

        // And no SendToLlmProvider was emitted.
        let commands = sink.commands();
        let has_send = commands
            .iter()
            .any(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(
            !has_send,
            "expected no SendToLlmProvider when tool_loop_disabled"
        );

        // And the flag is cleared.
        drop(state);
        let mut state = actor.state.write();
        let session = state.session_mut_or_create(&session_id);
        assert!(
            !session.take_tool_loop_disabled(),
            "tool_loop_disabled should be cleared after on_tool_batch_completed"
        );
    }

    #[tokio::test]
    async fn on_tool_batch_completed_unaffected_without_tool_loop_disabled() {
        // Given a normal session (no flag set) in sending state with history.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("list files"));
            session.push_entry(ChatEntry::assistant("checking"));
            session.push_entry(ChatEntry::tool_call("tc-1", "bash", r#"{"command":"ls"}"#));
            session.push_entry(ChatEntry::assistant("here are the files"));
            session.begin_streaming();
            session.finish_streaming(true);
            session.begin_sending();
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
                pin_position: None,
            }],
        };
        actor.on_tool_batch_completed(&event, &ctx);

        // Then SendToLlmProvider IS emitted (normal behavior).
        let commands = sink.commands();
        let has_send = commands
            .iter()
            .any(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(
            has_send,
            "expected SendToLlmProvider for normal session without tool_loop_disabled"
        );
    }

    // --- on_tool_use_started ---

    #[test]
    fn on_tool_use_started_creates_tool_call_entry() {
        // Given a session in streaming state.
        let actor = test_actor();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When a tool use starts.
        actor.on_tool_use_started(&ToolUseStarted {
            session_id: session_id.clone(),
            index: 0,
            id: "tc-1".to_owned(),
            name: "bash".to_owned(),
            dispatched_at: jiff::Timestamp::now(),
        });

        // Then a ToolCall entry is in history with empty arguments.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        let tc = session
            .history()
            .iter()
            .find(|e| matches!(&e.kind, ChatEntryKind::ToolCall { id, .. } if id == "tc-1"));
        assert!(tc.is_some(), "expected ToolCall entry with id tc-1");
    }

    // --- on_tool_call_received ---

    #[test]
    fn on_tool_call_received_finalizes_arguments() {
        // Given a session with a started tool call.
        let actor = test_actor();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            session.begin_tool_call(0, "tc-1", "bash");
            state.session.active_session_id().clone()
        };

        // When the complete tool call is received.
        actor.on_tool_call_received(&ToolCallReceived {
            session_id: session_id.clone(),
            tool_call: ToolCall {
                id: "tc-1".to_owned(),
                name: "bash".to_owned(),
                arguments: r#"{"command":"ls"}
"#
                .to_owned(),
            },
            dispatched_at: jiff::Timestamp::now(),
        });

        // Then the tool call entry has the complete arguments.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        let tc = session
            .history()
            .iter()
            .find(|e| matches!(&e.kind, ChatEntryKind::ToolCall { id, .. } if id == "tc-1"))
            .expect("tool call entry");
        if let ChatEntryKind::ToolCall { arguments, .. } = &tc.kind {
            assert!(
                arguments.contains("ls"),
                "expected arguments to contain 'ls', got: {arguments}"
            );
        }
    }

    // --- on_tool_call_streaming ---

    #[test]
    fn on_tool_call_streaming_appends_delta() {
        // Given a session with a started tool call.
        let actor = test_actor();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            session.begin_tool_call(0, "tc-1", "bash");
            state.session.active_session_id().clone()
        };

        // When streaming a delta.
        actor.on_tool_call_streaming(&ToolCallStreaming {
            session_id: session_id.clone(),
            index: 0,
            partial_json: "{\"co".to_owned(),
        });
        actor.on_tool_call_streaming(&ToolCallStreaming {
            session_id: session_id.clone(),
            index: 0,
            partial_json: "mmand\":\"ls\"}".to_owned(),
        });

        // Then the arguments are accumulated.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        let tc = session
            .history()
            .iter()
            .find(|e| matches!(&e.kind, ChatEntryKind::ToolCall { id, .. } if id == "tc-1"))
            .expect("tool call entry");
        if let ChatEntryKind::ToolCall { arguments, .. } = &tc.kind {
            assert_eq!(arguments, "{\"command\":\"ls\"}");
        }
    }

    // --- on_tool_execution_started ---

    #[test]
    fn on_tool_execution_started_creates_pending_result() {
        // Given a session in streaming state.
        let actor = test_actor();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_sending();
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When a tool execution starts.
        actor.on_tool_execution_started(&ToolExecutionStarted {
            session_id: session_id.clone(),
            tool_call_id: "tc-1".to_owned(),
            name: "bash".to_owned(),
            dispatched_at: jiff::Timestamp::now(),
        });

        // Then a ToolResult entry with Pending status is in history.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        let tr = session
            .history()
            .iter()
            .find(|e| matches!(&e.kind, ChatEntryKind::ToolResult { id, .. } if id == "tc-1"));
        assert!(tr.is_some(), "expected ToolResult entry with id tc-1");
    }

    // --- on_tool_execution_output ---

    #[test]
    fn on_tool_execution_output_appends_to_pending_result() {
        // Given a session with a pending tool result.
        let actor = test_actor();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_sending();
            session.begin_streaming();
            session.begin_tool_result("tc-1", "bash");
            state.session.active_session_id().clone()
        };

        // When incremental output arrives.
        actor.on_tool_execution_output(&ToolExecutionOutput {
            session_id: session_id.clone(),
            tool_call_id: "tc-1".to_owned(),
            output: "file1.txt\n".to_owned(),
        });
        actor.on_tool_execution_output(&ToolExecutionOutput {
            session_id: session_id.clone(),
            tool_call_id: "tc-1".to_owned(),
            output: "file2.txt\n".to_owned(),
        });

        // Then the accumulated content is in the pending result.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session");
        let tr = session
            .history()
            .iter()
            .find(|e| matches!(&e.kind, ChatEntryKind::ToolResult { id, .. } if id == "tc-1"))
            .expect("tool result");
        if let ChatEntryKind::ToolResult { content, .. } = &tr.kind {
            assert_eq!(content, "file1.txt\nfile2.txt\n");
        }
    }

    // --- Phase 7: Comprehensive tool batch transition tests ---

    // --- Phase 2: History mutation application hooks ---

    #[tokio::test]
    async fn on_tool_batch_completed_applies_pending_mutations() {
        // Given a session in sending state with pending history mutations.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let (entry_id, session_id) = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("list files"));
            let entry = ChatEntry::assistant("checking");
            let entry_id = entry.id.clone();
            session.push_entry(entry);
            session.push_entry(ChatEntry::tool_call("tc-1", "bash", r#"{"command":"ls"}"#));
            session.push_entry(ChatEntry::assistant("here are the files"));
            session.begin_streaming();
            session.finish_streaming(true);
            session.begin_sending();
            // Queue a mutation to exclude the assistant entry.
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
                pin_position: None,
            }],
        };
        actor.on_tool_batch_completed(&event, &ctx);

        // Then the mutation was applied - the assistant entry is now ForcedExclude.
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
            "expected mutation to be applied at tool batch completion"
        );

        // And SendToLlmProvider was still emitted (normal flow continues).
        let commands = sink.commands();
        let has_send = commands
            .iter()
            .any(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(
            has_send,
            "expected SendToLlmProvider after mutation application"
        );
    }

    #[tokio::test]
    async fn on_tool_batch_completed_empty_mutation_queue_is_noop() {
        // Given a session in sending state with no pending mutations.
        let actor = test_actor();
        let (sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("list files"));
            session.push_entry(ChatEntry::assistant("checking"));
            session.push_entry(ChatEntry::tool_call("tc-1", "bash", r#"{"command":"ls"}"#));
            session.push_entry(ChatEntry::assistant("here are the files"));
            session.begin_streaming();
            session.finish_streaming(true);
            session.begin_sending();
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
                pin_position: None,
            }],
        };
        actor.on_tool_batch_completed(&event, &ctx);

        // Then SendToLlmProvider was emitted (no side effects from empty queue).
        let commands = sink.commands();
        let has_send = commands
            .iter()
            .any(|c| matches!(c, Command::SendToLlmProvider(_)));
        assert!(
            has_send,
            "expected SendToLlmProvider with empty mutation queue"
        );
    }

    #[tokio::test]
    async fn on_tool_batch_completed_drained_steering_entry_lands_after_tool_results() {
        // Given a session with [user][assistant+tool_call][tool_result] in history
        // and a non-empty steering buffer.
        let actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.push_entry(ChatEntry::user("list files"));
            session.push_entry(ChatEntry::tool_call("tc-1", "bash", r#"{"command":"ls"}"#));
            session.push_entry(ChatEntry::assistant("checking"));
            session.push_entry(ChatEntry::tool_result(
                "tc-1",
                "bash",
                "file1.txt",
                ToolResultStatus::Success,
            ));
            session.push_entry(ChatEntry::tool_result(
                "tc-1",
                "bash",
                "file2.txt",
                ToolResultStatus::Success,
            ));
            // Prime steering buffer.
            session
                .steering_buffer_mut()
                .push_fragment("stay at the foo part");
            // Tool batch already completed; transition to Sending so on_tool_batch_completed
            // routes through the drain path.
            session.finish_streaming(true);
            session.begin_sending();
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
                pin_position: None,
            }],
        };
        actor.on_tool_batch_completed(&event, &ctx);

        // Then the drained steering entry's index is greater than both tool_result indices.
        let state = actor.state.read();
        let session = state.session.active_session();
        let history = session.history();
        let tool_result_indices: Vec<usize> = history
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(e.kind, ChatEntryKind::ToolResult { .. }))
            .map(|(i, _)| i)
            .collect();
        let steer_index = history
            .iter()
            .enumerate()
            .find(|(_, e)| matches!(&e.kind, ChatEntryKind::User { expanded, .. } if expanded == "stay at the foo part"))
            .map(|(i, _)| i);
        assert!(
            !tool_result_indices.is_empty(),
            "expected tool_result entries in history"
        );
        let steer_idx = steer_index.expect("drained steering entry must appear in history");
        for &tr_idx in &tool_result_indices {
            assert!(
                steer_idx > tr_idx,
                "steering entry at {steer_idx} must come after tool_result at {tr_idx}"
            );
        }
    }
}
