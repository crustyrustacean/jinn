//! Event handlers — process streaming and model refresh events.

use crate::common::actor::ActorContext;
use crate::feat::chat_input::protocol::command::PushChatEntry;
use crate::feat::context::protocol::event::PromptAssembled;
use crate::feat::context::strategy::token_estimator::TokenCounter;
use crate::feat::provider::protocol::command::SendToLlmProvider;
use crate::feat::provider::protocol::event::{
    ModelsRefreshed, StreamCompleted, StreamCompletedReason, StreamToken,
};
use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::protocol::{ChatEntry, Command, TableData};

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
        let mut state = self.state.write();
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
        } else if let Err(e) = session.append_stream_token(&event.token) {
            tracing::error!(err = ?e, "failed to append stream token");
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
            self.start_turn_from_queued(&event.session_id, &drained_entries, ctx)
                .await;
        }

        // Persist session after stream finishes (not on cancel).
        if should_save {
            self.save_active_session(&event.session_id).await;
        }
    }

    /// Pushes a table entry after model refresh.
    ///
    /// Emits `PushChatEntry` commands so the entries are persisted.
    pub(in crate::feat::session::session_actor) fn on_models_refreshed(
        &self,
        event: &ModelsRefreshed,
        ctx: &ActorContext,
    ) {
        // No providers at all — push a simple system entry.
        if event.results.is_empty() && event.errors.is_empty() {
            if let Err(e) = ctx.send_command(Command::PushChatEntry(PushChatEntry {
                session_id: event.session_id.clone(),
                entry: ChatEntry::system("Models refreshed: no providers found"),
            })) {
                tracing::warn!(err = ?e, "session-actor failed to emit PushChatEntry for models refresh");
            }
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
        if let Err(e) = ctx.send_command(Command::PushChatEntry(PushChatEntry {
            session_id: event.session_id.clone(),
            entry: ChatEntry::table(data),
        })) {
            tracing::warn!(err = ?e, "session-actor failed to emit PushChatEntry for models refresh");
        }
    }
}
