//! Event handlers — process streaming and tool call events.

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

use crate::protocol::{ChatEntry, Command, TableData};

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

        if let Err(e) = ctx.send_command(Command::SendToLlmProvider {
            payload: SendToLlmProvider {
                session_id: payload.session_id.clone(),
                messages: payload.messages.clone(),
                provider_id: None,
            },
        }) {
            tracing::warn!(err = ?e, "session-actor failed to emit SendToLlmProvider");
        }
    }

    /// Appends a streaming token to the session's assistant entry.
    pub(in crate::feat::session::session_actor) fn on_stream_token(&self, event: &StreamToken) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        if !session.is_streaming() {
            session.begin_streaming();
        }
        session.append_stream_token(&event.token);
    }

    /// Marks the session's stream as finished and records output tokens.
    pub(in crate::feat::session::session_actor) fn on_stream_completed(
        &self,
        event: &StreamCompleted,
    ) {
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

    /// Pushes a tool call entry into the session history.
    pub(in crate::feat::session::session_actor) fn on_tool_call_received(
        &self,
        event: &ToolCallReceived,
    ) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        session.push_entry(ChatEntry::tool_call(
            &event.tool_call.id,
            &event.tool_call.name,
            &event.tool_call.arguments,
        ));
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
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        session.push_entry(ChatEntry::tool_result(
            &event.result.tool_call_id,
            &event.result.name,
            &event.result.content,
            event.result.success,
        ));
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
}
