//! LLM streaming actor with tool support.
//!
//! Subscribes to [`SendToLlmProvider`] and [`CancelStream`] commands, and
//! [`ToolBatchCompleted`], [`ToolsRegistered`], and [`StreamCompleted`] events.
//! On send, creates an LLM service via the factory and streams tokens and tool
//! call events back as bus commands. When the LLM requests tool use, emits
//! [`ExecuteToolBatch`] and awaits results before continuing the conversation.

mod session;

use std::collections::HashMap;
use std::sync::Arc;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::chat_input::protocol::command::PushChatEntry;
use crate::feat::provider::llm_message::LlmMessage;
use crate::feat::provider::protocol::command::{CancelStream, SendToLlmProvider};
use crate::feat::provider::protocol::event::{StreamCompleted, StreamCompletedReason, StreamToken};
use crate::feat::provider_infra::LlmServiceFactoryService;
use crate::feat::provider_infra::StopReason;
use crate::feat::provider_infra::StreamEvent;
use crate::feat::tools_actor::protocol::command::{CancelToolBatch, ExecuteToolBatch};
use crate::feat::tools_actor::protocol::event::{
    ToolBatchCompleted, ToolCallReceived, ToolCallStreaming, ToolUseStarted, ToolsRegistered,
};
use crate::feat::tools_actor::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::protocol::{ChatEntry, Command, Event, SessionId};
use futures::StreamExt as _;

use session::{SessionData, SessionState};

/// LLM streaming actor with tool support.
///
/// Holds a reference to the LLM service factory and tracks active
/// streaming tasks and per-session state.
pub struct LlmActor {
    /// Factory for creating LLM service instances.
    factory: LlmServiceFactoryService,
    /// Runtime services (provider registry, API keys for per-request factory creation).
    services: Option<Services>,
    /// Shared application state (for reading tool definitions).
    state: State,
    /// Active stream tasks, keyed by session ID.
    tasks: HashMap<SessionId, tokio::task::JoinHandle<()>>,
    /// Per-session state.
    sessions: HashMap<SessionId, SessionData>,
}

impl Actor for LlmActor {
    type Message = NoDirectMsg;

    #[expect(
        clippy::expect_used,
        reason = "data is injected by the host before activate is called"
    )]
    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<SendToLlmProvider>();
        ctx.subscribe_command::<CancelStream>();
        ctx.subscribe_event::<ToolBatchCompleted>();
        ctx.subscribe_event::<ToolsRegistered>();
        ctx.subscribe_event::<StreamCompleted>();

        let factory = ctx
            .take_data::<LlmServiceFactoryService>()
            .expect("LlmServiceFactoryService must be injected via ctx.set_data() before activate");
        let services = ctx.take_data::<Services>();
        let state = ctx
            .take_data::<State>()
            .expect("State must be injected via ctx.set_data() before activate");

        Self {
            factory,
            services,
            state,
            tasks: HashMap::new(),
            sessions: HashMap::new(),
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<NoDirectMsg>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(command) => self.handle_command(&command, ctx),
            ActorEnvelope::Event(event) => self.handle_event(&event, ctx),
            _ => {}
        }
    }

    async fn on_shutdown(&mut self, _ctx: &ActorContext) {
        self.cancel_all();
    }
}

impl LlmActor {
    /// Dispatches incoming commands to the appropriate handler.
    fn handle_command(&mut self, command: &Command, ctx: &ActorContext) {
        match command {
            Command::SendToLlmProvider(payload) => {
                self.start_stream(
                    payload.session_id.clone(),
                    payload.messages.clone(),
                    payload.provider_id.as_deref(),
                    ctx,
                );
            }
            Command::CancelStream(payload) => {
                self.cancel_stream(&payload.session_id, ctx);
            }
            _ => {}
        }
    }

    /// Dispatches incoming events to the appropriate handler.
    fn handle_event(&mut self, event: &Event, ctx: &ActorContext) {
        match event {
            Event::ToolsRegistered(payload) => {
                self.handle_tools_registered(&payload.definitions);
            }
            Event::ToolBatchCompleted(payload) => {
                self.handle_tool_batch_completed(payload.session_id.clone(), &payload.results, ctx);
            }
            Event::StreamCompleted(payload) => {
                self.handle_stream_completed(payload);
            }
            _ => {}
        }
    }

    /// Starts an LLM streaming response for a session, aborting any existing stream.
    #[expect(
        clippy::too_many_lines,
        reason = "stream handling is inherently linear; splitting would obscure the flow"
    )]
    fn start_stream(
        &mut self,
        session_id: SessionId,
        messages: Vec<LlmMessage>,
        provider_id: Option<&str>,
        ctx: &ActorContext,
    ) {
        // Collect current tool definitions from shared state.
        let tools: Vec<ToolDefinition> = {
            let guard = self.state.read();
            guard.context.tool_definitions.values().cloned().collect()
        };

        let message_count = messages.len();
        tracing::trace!(
            session_id = ?session_id,
            message_count,
            tool_count = tools.len(),
            "start_stream"
        );

        // Abort any existing stream for this session.
        if let Some(handle) = self.tasks.remove(&session_id) {
            handle.abort();
        }

        // Create or reset session data.
        // Clone messages before inserting into the session so the stream task
        // can take ownership of its copy.
        let messages_for_stream = messages.clone();
        let mut session = SessionData::new(messages);
        if provider_id.is_some() {
            session.provider_id = provider_id.map(std::borrow::ToOwned::to_owned);
        }
        self.sessions.insert(session_id.clone(), session);

        // Resolve the factory: per-request if provider_id is set, global fallback otherwise.
        let factory = if let Some(pid) = provider_id {
            if let Some(ref services) = self.services {
                let id = crate::feat::provider_infra::ProviderId::new(pid.to_owned());
                let api_keys = services.api_keys.read();
                match services.provider_registry.create_factory(&id, &api_keys) {
                    Ok(f) => {
                        tracing::debug!(provider_id = %pid, "created per-request LLM factory");
                        LlmServiceFactoryService::new(Arc::from(f))
                    }
                    Err(e) => {
                        tracing::error!(err = ?e, provider_id = %pid, "failed to create per-request factory");
                        let sink = ctx.sink();
                        let sid = session_id.clone();
                        let _ = sink.send_command(Command::PushChatEntry(PushChatEntry {
                            session_id: sid.clone(),
                            entry: ChatEntry::error(format!(
                                "LLM factory creation failed for {pid}: {e:?}"
                            )),
                        }));
                        let _ = sink.send_event(Event::StreamCompleted(StreamCompleted {
                            session_id: sid,
                            reason: StreamCompletedReason::Error,
                            assistant_content: None,
                            tool_calls: None,
                        }));
                        return;
                    }
                }
            } else {
                // No services — fall through to global factory
                self.factory.clone()
            }
        } else {
            self.factory.clone()
        };
        let model_id = provider_id
            .map(std::borrow::ToOwned::to_owned)
            .unwrap_or_default();
        let sink = ctx.sink();
        let sid = session_id.clone();

        let handle = tokio::spawn(async move {
            let service = match factory.create() {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(err = ?e, "failed to create LLM service");
                    let _ = sink.send_command(Command::PushChatEntry(PushChatEntry {
                        session_id: sid.clone(),
                        entry: ChatEntry::error(format!("LLM service creation failed: {e:?}")),
                    }));
                    let _ = sink.send_event(Event::StreamCompleted(StreamCompleted {
                        session_id: sid,
                        reason: StreamCompletedReason::Error,
                        assistant_content: None,
                        tool_calls: None,
                    }));
                    return;
                }
            };

            let stream = match service
                .chat_stream_with_tools(messages_for_stream, tools)
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(err = ?e, "failed to start LLM stream");
                    let _ = sink.send_command(Command::PushChatEntry(PushChatEntry {
                        session_id: sid.clone(),
                        entry: ChatEntry::error(format!("LLM stream error: {e:?}")),
                    }));
                    let _ = sink.send_event(Event::StreamCompleted(StreamCompleted {
                        session_id: sid,
                        reason: StreamCompletedReason::Error,
                        assistant_content: None,
                        tool_calls: None,
                    }));
                    return;
                }
            };

            // Accumulate text and tool calls from the stream.
            let mut accumulated_text = String::new();
            let mut accumulated_tool_calls: Vec<ToolCall> = Vec::new();
            let mut token_index = 0usize;
            let mut parser = reasoning_parser::ParserFactory::new().create(&model_id);

            let mut stream = std::pin::pin!(stream);
            while let Some(result) = stream.next().await {
                match result {
                    Ok(event) => match event {
                        StreamEvent::Text(token) => {
                            accumulated_text.push_str(&token);
                            let parsed = match parser.parse_reasoning_streaming_incremental(&token)
                            {
                                Ok(r) => r,
                                Err(e) => {
                                    tracing::warn!(
                                        err = ?e,
                                        "reasoning parser error, treating as normal text"
                                    );
                                    reasoning_parser::ParserResult::normal(token.clone())
                                }
                            };
                            if !parsed.reasoning_text.is_empty() {
                                let _ = sink.send_event(Event::StreamToken(StreamToken {
                                    session_id: sid.clone(),
                                    index: token_index,
                                    token: parsed.reasoning_text,
                                    is_thinking: true,
                                }));
                                token_index += 1;
                            }
                            if !parsed.normal_text.is_empty() {
                                let _ = sink.send_event(Event::StreamToken(StreamToken {
                                    session_id: sid.clone(),
                                    index: token_index,
                                    token: parsed.normal_text,
                                    is_thinking: false,
                                }));
                                token_index += 1;
                            }
                        }
                        StreamEvent::Reasoning(token) => {
                            let _ = sink.send_event(Event::StreamToken(StreamToken {
                                session_id: sid.clone(),
                                index: token_index,
                                token,
                                is_thinking: true,
                            }));
                            token_index += 1;
                        }
                        StreamEvent::ToolUseStart { index, id, name } => {
                            let _ = sink.send_event(Event::ToolUseStarted(ToolUseStarted {
                                session_id: sid.clone(),
                                index,
                                id,
                                name,
                            }));
                        }
                        StreamEvent::ToolUseInputDelta {
                            index,
                            partial_json,
                        } => {
                            let _ = sink.send_event(Event::ToolCallStreaming(ToolCallStreaming {
                                session_id: sid.clone(),
                                index,
                                partial_json,
                            }));
                        }
                        StreamEvent::ToolUseComplete { tool_call, .. } => {
                            accumulated_tool_calls.push(tool_call.clone());
                            let _ = sink.send_event(Event::ToolCallReceived(ToolCallReceived {
                                session_id: sid.clone(),
                                tool_call,
                            }));
                        }
                        StreamEvent::Done { stop_reason } => {
                            tracing::trace!(
                                session_id = ?sid,
                                stop_reason = %stop_reason,
                                tool_call_count = accumulated_tool_calls.len(),
                                "stream Done"
                            );
                            if stop_reason == StopReason::ToolUse {
                                // Emit ExecuteToolBatch for the orchestrator.
                                let _ = sink.send_command(Command::ExecuteToolBatch(
                                    ExecuteToolBatch {
                                        session_id: sid.clone(),
                                        tool_calls: accumulated_tool_calls.clone(),
                                    },
                                ));

                                // Emit StreamCompleted with ToolUse reason so the actor
                                // can transition state.
                                let _ = sink.send_event(Event::StreamCompleted(StreamCompleted {
                                    session_id: sid.clone(),
                                    reason: StreamCompletedReason::ToolUse,
                                    assistant_content: Some(accumulated_text.clone()),
                                    tool_calls: Some(accumulated_tool_calls.clone()),
                                }));
                            } else {
                                // Normal end_turn — emit StreamCompleted.
                                let _ = sink.send_event(Event::StreamCompleted(StreamCompleted {
                                    session_id: sid.clone(),
                                    reason: StreamCompletedReason::Finished,
                                    assistant_content: Some(accumulated_text.clone()),
                                    tool_calls: None,
                                }));
                            }
                        }
                    },
                    Err(e) => {
                        tracing::error!(err = ?e, "LLM stream error");
                        let _ = sink.send_command(Command::PushChatEntry(PushChatEntry {
                            session_id: sid.clone(),
                            entry: ChatEntry::error(format!("LLM stream error: {e:?}")),
                        }));
                        let _ = sink.send_event(Event::StreamCompleted(StreamCompleted {
                            session_id: sid,
                            reason: StreamCompletedReason::Error,
                            assistant_content: None,
                            tool_calls: None,
                        }));
                        break;
                    }
                }
            }
        });

        // Update session state.
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.state = SessionState::Streaming;
        }

        self.tasks.insert(session_id, handle);
    }

    /// Handles stream completion events to transition session state.
    ///
    /// When the stream task sends [`StreamCompleted`] through the sink, it
    /// arrives back on the bus and the actor receives it here. For
    /// [`ToolUse`](StreamCompletedReason::ToolUse), the actor stores the
    /// accumulated data and transitions to [`AwaitingToolResults`](SessionState::AwaitingToolResults).
    /// For [`Finished`](StreamCompletedReason::Finished), the session is cleaned up.
    fn handle_stream_completed(&mut self, payload: &StreamCompleted) {
        let Some(session) = self.sessions.get_mut(&payload.session_id) else {
            return;
        };

        match payload.reason {
            StreamCompletedReason::ToolUse => {
                // Store accumulated data from the stream task.
                if let Some(ref text) = payload.assistant_content {
                    session.accumulated_text.clone_from(text);
                }
                if let Some(ref calls) = payload.tool_calls {
                    session.accumulated_tool_calls.clone_from(calls);
                }
                session.state = SessionState::AwaitingToolResults;
                tracing::trace!(
                    session_id = ?payload.session_id,
                    reason = "ToolUse",
                    new_state = ?session.state,
                    "handle_stream_completed"
                );
            }
            StreamCompletedReason::Error | StreamCompletedReason::Finished => {
                // Defensive guard: if the session is awaiting tool results, a
                // duplicate Done from the provider should not remove the session.
                if session.state == SessionState::AwaitingToolResults {
                    tracing::warn!(
                        session_id = ?payload.session_id,
                        state = ?session.state,
                        "received StreamCompleted({:?}) while awaiting tool results — ignoring",
                        payload.reason
                    );
                    return;
                }
                // Clean up the completed session.
                tracing::trace!(
                    session_id = ?payload.session_id,
                    reason = ?payload.reason,
                    "handle_stream_completed — removing session"
                );
                self.sessions.remove(&payload.session_id);
            }
            StreamCompletedReason::Canceled => {
                // Already cleaned up by cancel_stream.
            }
        }
    }

    /// Handles tool batch completion by continuing the conversation with results.
    fn handle_tool_batch_completed(
        &mut self,
        session_id: SessionId,
        results: &[ToolResult],
        ctx: &ActorContext,
    ) {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            tracing::warn!(
                session_id = ?session_id,
                "received ToolBatchCompleted for unknown session"
            );
            return;
        };

        if session.state != SessionState::AwaitingToolResults {
            tracing::warn!(
                session_id = ?session_id,
                state = ?session.state,
                "received ToolBatchCompleted while not awaiting tool results"
            );
            return;
        }

        tracing::trace!(
            session_id = ?session_id,
            result_count = results.len(),
            "handle_tool_batch_completed"
        );

        // Build the assistant message with tool calls and text from the previous stream.
        let assistant_message = LlmMessage::Assistant {
            content: std::mem::take(&mut session.accumulated_text),
            tool_calls: Some(std::mem::take(&mut session.accumulated_tool_calls)),
        };
        session.messages.push(assistant_message);

        // Build tool result messages.
        for result in results {
            session.messages.push(LlmMessage::Tool {
                tool_call_id: result.tool_call_id.clone(),
                name: result.name.clone(),
                content: result.content.clone(),
            });
        }

        // Take the accumulated messages and start a new stream.
        // Preserve the provider_id from the initial stream so the continuation
        // uses the same provider instead of falling back to the global factory.
        let provider_id = session.provider_id.clone();
        let messages = std::mem::take(&mut session.messages);
        self.start_stream(session_id, messages, provider_id.as_deref(), ctx);
    }

    /// Caches tool definitions from a [`ToolsRegistered`] event into shared state.
    fn handle_tools_registered(&self, definitions: &[ToolDefinition]) {
        let mut state = self.state.write();
        for def in definitions {
            state
                .context
                .tool_definitions
                .insert(def.name.clone(), def.clone());
        }
    }

    /// Cancels the active stream for a session and emits a completion event.
    fn cancel_stream(&mut self, session_id: &SessionId, ctx: &ActorContext) {
        // If the session is awaiting tool results, tell the orchestrator to cancel them.
        let awaiting_tools = self
            .sessions
            .get(session_id)
            .is_some_and(|s| s.state == SessionState::AwaitingToolResults);

        if awaiting_tools
            && let Err(e) = ctx.send_command(Command::CancelToolBatch(CancelToolBatch {
                session_id: session_id.clone(),
            }))
        {
            tracing::warn!(
                err = ?e,
                "failed to emit CancelToolBatch during stream cancellation"
            );
        }

        if let Some(handle) = self.tasks.remove(session_id) {
            handle.abort();
        }
        let had_session = self.sessions.remove(session_id).is_some();
        if let Some(handle) = self.tasks.remove(session_id) {
            handle.abort();
        }
        // Only emit StreamCompleted if there was actually an active session
        // to cancel. Avoids pushing a spurious "Cancelled" error entry when
        // the user presses ESC with nothing streaming.
        if had_session {
            let _ = ctx.send_event(Event::StreamCompleted(StreamCompleted {
                session_id: session_id.clone(),
                reason: StreamCompletedReason::Canceled,
                assistant_content: None,
                tool_calls: None,
            }));
        }
    }

    /// Cancels all active streams across all sessions.
    fn cancel_all(&self) {
        for handle in self.tasks.values() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::actor::{ActorContext, RecordingSink};
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::provider_infra::LlmServiceFactoryService;
    use nullslop_provider::FakeLlmServiceFactory;

    fn test_llm_actor() -> LlmActor {
        let factory = LlmServiceFactoryService::new(Arc::new(FakeLlmServiceFactory::new(vec![])));
        LlmActor {
            factory,
            services: None,
            state: State::new(AppState::default()),
            tasks: HashMap::new(),
            sessions: HashMap::new(),
        }
    }

    #[test]
    fn handle_stream_completed_error_reason_removes_session() {
        // Given an LLM actor with a streaming session.
        let mut actor = test_llm_actor();
        let session_id = SessionId::new();
        actor
            .sessions
            .insert(session_id.clone(), SessionData::new(vec![]));

        // When handling StreamCompleted with Error reason.
        let payload = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Error,
            assistant_content: None,
            tool_calls: None,
        };
        actor.handle_stream_completed(&payload);

        // Then the session is removed from the sessions map.
        assert!(actor.sessions.is_empty());
    }
}
