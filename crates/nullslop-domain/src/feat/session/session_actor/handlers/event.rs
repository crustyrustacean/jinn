//! Event handlers — process streaming and tool call events.

use crate::common::actor::ActorContext;
use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
use crate::feat::context::protocol::command::AssemblePrompt;
use crate::feat::context::protocol::event::PromptAssembled;
use crate::feat::context::strategy::token_estimator::TokenCounter;
use crate::feat::provider::protocol::command::SendToLlmProvider;
use crate::feat::provider::protocol::event::{
    ModelsRefreshed, StreamCompleted, StreamCompletedReason, StreamToken,
};
use crate::feat::tools_actor::protocol::event::{
    ToolBatchCompleted, ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted,
    ToolExecutionOutput, ToolExecutionStarted, ToolUseStarted,
};
use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::protocol::{ChatEntry, Command, Event, SessionId, TableData};

use super::super::SessionPersistenceActor;
use crate::feat::session::chat_session::SessionPhase;

impl SessionPersistenceActor {
    /// PromptAssembled (event): transition session from assembling to streaming,
    /// count input tokens, record in ledger, emit SendToLlmProvider.
    pub(in crate::feat::session::session_actor) async fn handle_prompt_assembled(
        &self,
        payload: &PromptAssembled,
        ctx: &crate::common::actor::ActorContext,
    ) {
        // Count tokens in all assembled messages (CPU-bound — offload to blocking thread).
        let messages = payload.messages.clone();
        let counter = self.counter;
        let input_tokens: usize = tokio::task::spawn_blocking(move || {
            messages
                .iter()
                .map(|msg| match msg {
                    crate::protocol::LlmMessage::System { content }
                    | crate::protocol::LlmMessage::User { content }
                    | crate::protocol::LlmMessage::Assistant { content, .. }
                    | crate::protocol::LlmMessage::Tool { content, .. } => counter.count(content),
                })
                .sum()
        })
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(err = ?e, "spawn_blocking panicked during token counting");
            0
        });

        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            match session.phase() {
                SessionPhase::Sending => {
                    // Sending → Streaming transition.
                    // Don't call finish_sending() here — begin_streaming() handles
                    // the Sending → Streaming transition directly.
                    session.begin_streaming();
                }
                SessionPhase::Assembling => {
                    session.finish_assembling();
                    session.begin_sending();
                    session.begin_streaming();
                }
                other => {
                    tracing::warn!(
                        phase = ?other,
                        "PromptAssembled received in unexpected phase, transitioning to Streaming"
                    );
                    session.begin_streaming();
                }
            }

            session.push_token_record(crate::feat::session::token_stats::TokenRecord {
                timestamp: jiff::Timestamp::now(),
                tokens_sent: input_tokens as u32,
                tokens_received: 0,
                cost: None,
            });
            session.set_context_size(input_tokens as u32);
        }

        let provider_id = {
            let state = self.state.read();
            let model = state.session(&payload.session_id).profile().model.clone();
            if model == crate::feat::provider_infra::NO_PROVIDER_ID {
                None
            } else {
                Some(model)
            }
        };

        if let Err(e) = ctx.send_command(Command::SendToLlmProvider(SendToLlmProvider {
            session_id: payload.session_id.clone(),
            messages: payload.messages.clone(),
            provider_id,
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit SendToLlmProvider");
        }
    }

    /// Appends a streaming token to the session's assistant entry,
    /// or to the thinking entry if the token is flagged as reasoning.
    pub(in crate::feat::session::session_actor) fn on_stream_token(&self, event: &StreamToken) {
        let lock_start = std::time::Instant::now();
        let mut state = self.state.write();
        let lock_wait = lock_start.elapsed();
        let session = state.session_mut_or_create(&event.session_id);
        match session.phase() {
            SessionPhase::Streaming => {}
            SessionPhase::Sending => {
                // Defensive: stream token arrived without PromptAssembled.
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
        } else {
            if let Err(e) = session.append_stream_token(&event.token) {
                tracing::error!(err = ?e, "failed to append stream token");
            }
        }
        drop(state);
        let total = lock_start.elapsed();
        if total.as_micros() > 500 {
            tracing::warn!(
                lock_wait_us = lock_wait.as_micros() as u64,
                total_us = total.as_micros() as u64,
                token_len = event.token.len(),
                "PERF: on_stream_token slow (>500µs)"
            );
        }
    }

    /// Marks the session's stream as finished, records output tokens, and
    /// drains any queued messages into a new turn.
    ///
    /// For `Finished` reason, drains the message queue. If messages were queued,
    /// pushes each as a separate user entry and starts a new `AssemblePrompt`.
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

        // Count output tokens off the async thread (CPU-bound — offload to blocking thread).
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

        let drained_entries: Vec<ChatEntry>;
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&event.session_id);
            if event.reason == StreamCompletedReason::Canceled {
                session.push_entry(ChatEntry::error("Cancelled"));
            } else if event.reason == StreamCompletedReason::Error {
                // Error entry is pushed by the LLM actor via PushChatEntry before
                // emitting StreamCompleted(Error). Nothing to push here.
            } else if let Some(output_tokens) = output_tokens {
                // Finalize the last record if one exists (i.e., PromptAssembled fired first).
                // If no record exists (e.g., session restored mid-stream), skip silently.
                if !session.token_ledger().is_empty() {
                    if let Err(e) = session.finalize_last_token_record(output_tokens, event.cost) {
                        tracing::error!(err = ?e, "failed to finalize token record");
                    }
                }
            }
            let preserve_assistant = event.reason == StreamCompletedReason::Finished
                || event.reason == StreamCompletedReason::ToolUse;
            session.finish_streaming(preserve_assistant);

            // Tool use means the conversation continues — transition to sending
            // so the indicator shows activity while awaiting the followup.
            if event.reason == StreamCompletedReason::ToolUse {
                session.begin_sending();
            }

            // Drain queue only on Finished — the turn has ended successfully.
            // Error and Canceled do not drain queued messages.
            drained_entries = if event.reason == StreamCompletedReason::Finished {
                session.drain_queue().into_iter().collect()
            } else {
                vec![]
            };
        }

        // If messages were drained, start a new turn.
        if !drained_entries.is_empty() {
            self.start_turn_from_queued(&event.session_id, &drained_entries, ctx);
        }

        // Persist session after stream finishes (not on cancel).
        if should_save {
            self.save_active_session(&event.session_id).await;
        }
    }

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
            );
        }
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
        let lock_start = std::time::Instant::now();
        let mut state = self.state.write();
        let lock_wait = lock_start.elapsed();
        let session = state.session_mut_or_create(&event.session_id);
        session.append_tool_result_output(&event.tool_call_id, &event.output);
        drop(state);
        let total = lock_start.elapsed();
        if total.as_micros() > 500 {
            tracing::warn!(
                lock_wait_us = lock_wait.as_micros() as u64,
                total_us = total.as_micros() as u64,
                output_len = event.output.len(),
                "PERF: on_tool_execution_output slow (>500µs)"
            );
        }
    }

    /// All tools in a batch have finished — route the continuation through
    /// context assembly so token counting and prompt strategy apply.
    ///
    /// By this point, the session history already contains `ToolCall`,
    /// `ToolResult`, and `Assistant` entries from earlier event handlers,
    /// and the session is already in sending state (set by `on_stream_completed`
    /// for the `ToolUse` reason). We just need to emit `AssemblePrompt` with
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

        // Read history and model, then emit AssemblePrompt.
        // Note: the session is already in sending state, set by on_stream_completed(ToolUse).
        let history_start = std::time::Instant::now();
        let (history, model_name) = {
            let state = self.state.read();
            let session = state.session(&event.session_id);
            (session.history().to_vec(), session.profile().model.clone())
        };
        let history_dur = history_start.elapsed();
        tracing::info!(
            history_dur_us = history_dur.as_micros() as u64,
            history_len = history.len(),
            "PERF: on_tool_batch_completed history clone"
        );

        if let Err(e) = ctx.send_command(Command::AssemblePrompt(AssemblePrompt {
            session_id: event.session_id.clone(),
            history,
            tools: vec![],
            model_name,
        })) {
            tracing::warn!(
                err = ?e,
                "session-actor failed to emit AssemblePrompt from tool batch completion"
            );
        }
    }

    /// Pushes a table entry after model refresh.
    pub(in crate::feat::session::session_actor) fn on_models_refreshed(
        &self,
        event: &ModelsRefreshed,
    ) {
        // No providers at all — push a simple system entry.
        if event.results.is_empty() && event.errors.is_empty() {
            let mut state = self.state.write();
            state
                .session_mut_or_create(&event.session_id)
                .push_entry(ChatEntry::system("Models refreshed: no providers found"));
            return;
        }

        let headers = vec![
            Span::raw("Provider"),
            Span::raw("Model Count"),
            Span::raw("Status"),
        ];

        // Collect all provider names and sort alphabetically.
        let mut all_providers: Vec<&str> = event
            .results
            .keys()
            .chain(event.errors.keys())
            .map(std::string::String::as_str)
            .collect();
        all_providers.sort_unstable();
        all_providers.dedup();

        let mut rows = Vec::new();
        for provider in all_providers {
            if let Some(models) = event.results.get(provider) {
                rows.push(vec![
                    Span::raw(provider.to_owned()),
                    Span::raw(models.len().to_string()),
                    Span::styled("\u{2705}".to_owned(), Style::default().fg(Color::Green)),
                ]);
            } else if let Some(err) = event.errors.get(provider) {
                rows.push(vec![
                    Span::raw(provider.to_owned()),
                    Span::raw("0".to_owned()),
                    Span::styled(format!("\u{274c} {err}"), Style::default().fg(Color::Red)),
                ]);
            }
        }

        let data = TableData { headers, rows };
        let mut state = self.state.write();
        state
            .session_mut_or_create(&event.session_id)
            .push_entry(ChatEntry::table(data));
    }

    /// Drain queued messages into a new turn: push each entry, then emit
    /// `AssemblePrompt` with the full session history.
    pub(in crate::feat::session::session_actor) fn start_turn_from_queued(
        &self,
        session_id: &SessionId,
        entries: &[ChatEntry],
        ctx: &ActorContext,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(session_id);
            for entry in entries {
                session.push_entry(entry.clone());
            }
            session.begin_sending();
        }

        let (history, model_name) = {
            let state = self.state.read();
            let session = state.session(session_id);
            (session.history().to_vec(), session.profile().model.clone())
        };

        if let Err(e) = ctx.send_command(Command::AssemblePrompt(AssemblePrompt {
            session_id: session_id.clone(),
            history,
            tools: vec![],
            model_name,
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit AssemblePrompt from queue drain");
        }

        // Emit ChatEntrySubmitted for each queued entry.
        for entry in entries {
            if let Err(e) = ctx.send_event(Event::ChatEntrySubmitted(ChatEntrySubmitted {
                session_id: session_id.clone(),
                entry: entry.clone(),
            })) {
                tracing::warn!(err = ?e, "session-actor failed to emit ChatEntrySubmitted for queued message");
            }
        }
    }
}
