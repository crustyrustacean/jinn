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
    ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted, ToolUseStarted,
};
use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::protocol::{ChatEntry, Command, Event, SessionId, TableData};

use super::super::SessionPersistenceActor;

impl SessionPersistenceActor {
    /// PromptAssembled (event): transition session from assembling to streaming,
    /// count input tokens, record in ledger, emit SendToLlmProvider.
    pub(in crate::feat::session::session_actor) fn handle_prompt_assembled(
        &self,
        payload: &PromptAssembled,
        ctx: &crate::common::actor::ActorContext,
    ) {
        let input_tokens: usize;
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            if session.is_assembling() {
                session.finish_assembling();
            }
            if session.is_sending() {
                session.finish_sending();
            }

            // Count tokens in all assembled messages.
            input_tokens = payload
                .messages
                .iter()
                .map(|msg| match msg {
                    crate::protocol::LlmMessage::System { content }
                    | crate::protocol::LlmMessage::User { content } => self.counter.count(content),
                    crate::protocol::LlmMessage::Assistant { content, .. }
                    | crate::protocol::LlmMessage::Tool { content, .. } => {
                        self.counter.count(content)
                    }
                })
                .sum();

            session.push_token_record(crate::feat::session::token_stats::TokenRecord {
                timestamp: jiff::Timestamp::now(),
                tokens_sent: input_tokens as u32,
                tokens_received: 0,
            });
            session.set_context_size(input_tokens as u32);

            session.begin_streaming();
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
        if !session.is_streaming() {
            session.begin_streaming();
        }
        if event.is_thinking {
            if session.streaming_thinking_entry_index().is_none() {
                session.begin_thinking();
            }
            session.append_thinking_token(&event.token);
        } else {
            session.append_stream_token(&event.token);
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
    pub(in crate::feat::session::session_actor) fn on_stream_completed(
        &self,
        event: &StreamCompleted,
        ctx: &ActorContext,
    ) {
        let should_save = event.reason == StreamCompletedReason::Finished;
        let drained_messages: Vec<String>;
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&event.session_id);
            if event.reason == StreamCompletedReason::Canceled {
                session.push_entry(ChatEntry::error("Cancelled"));
            } else if let Some(ref content) = event.assistant_content {
                let output_tokens = self.counter.count(content) as u32;
                // Finalize the last record if one exists (i.e., PromptAssembled fired first).
                // If no record exists (e.g., session restored mid-stream), skip silently.
                if !session.token_ledger().is_empty() {
                    session.finalize_last_token_record(output_tokens);
                }
            }
            session.finish_streaming();

            // Tool use means the conversation continues — transition to sending
            // so the indicator shows activity while awaiting the followup.
            if event.reason == StreamCompletedReason::ToolUse {
                session.begin_sending();
            }

            // Drain queue on Finished — the turn has ended.
            drained_messages = if event.reason == StreamCompletedReason::Finished {
                session.drain_queue().into_iter().collect()
            } else {
                vec![]
            };
        }

        // If messages were drained, start a new turn.
        if !drained_messages.is_empty() {
            self.start_turn_from_queued(&event.session_id, &drained_messages, ctx);
        }

        // Persist session after stream finishes (not on cancel).
        if should_save {
            self.save_active_session(&event.session_id);
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
        session.append_tool_call_delta(event.index, &event.partial_json);
    }

    /// Pushes a tool result entry into the session history.
    pub(in crate::feat::session::session_actor) fn on_tool_execution_completed(
        &self,
        event: &ToolExecutionCompleted,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&event.session_id);
            session.push_entry(ChatEntry::tool_result(
                &event.result.tool_call_id,
                &event.result.name,
                &event.result.content,
                event.result.success,
            ));
        }
        self.save_active_session(&event.session_id);
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

    /// Drain queued messages into a new turn: push each as a separate User
    /// entry, then emit `AssemblePrompt` with the full session history.
    pub(in crate::feat::session::session_actor) fn start_turn_from_queued(
        &self,
        session_id: &SessionId,
        messages: &[String],
        ctx: &ActorContext,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(session_id);
            for text in messages {
                session.push_entry(ChatEntry::user(text));
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

        // Emit ChatEntrySubmitted for each queued message.
        for text in messages {
            if let Err(e) = ctx.send_event(Event::ChatEntrySubmitted(ChatEntrySubmitted {
                session_id: session_id.clone(),
                entry: ChatEntry::user(text),
            })) {
                tracing::warn!(err = ?e, "session-actor failed to emit ChatEntrySubmitted for queued message");
            }
        }
    }
}
