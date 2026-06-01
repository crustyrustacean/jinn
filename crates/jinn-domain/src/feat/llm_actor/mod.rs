//! LLM streaming actor.
//!
//! Subscribes to [`SendToLlmProvider`] and [`CancelStream`] commands, and
//! [`StreamCompleted`] events. On send, creates an
//! LLM service via the factory and streams tokens and tool call events back
//! as bus commands. When the LLM requests tool use, emits [`ExecuteToolBatch`]
//! - the session actor handles the continuation via context assembly.

mod session;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, MessageSink, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::chat_input::protocol::command::PushChatEntry;
use crate::feat::provider::protocol::command::{CancelStream, SendToLlmProvider};
use crate::feat::provider::protocol::event::{StreamCompleted, StreamCompletedReason, StreamToken};
use crate::feat::provider_infra::LlmServiceFactoryService;
use crate::feat::provider_infra::StopReason;
use crate::feat::provider_infra::StreamEvent;
use crate::feat::tools_actor::protocol::command::CancelToolBatch;
use crate::feat::tools_actor::protocol::command::ExecuteToolBatch;
use crate::feat::tools_actor::protocol::event::{
    ToolCallReceived, ToolCallStreaming, ToolUseStarted,
};
use crate::feat::tools_actor::tool_types::ToolCall;
use crate::protocol::{ChatEntry, Command, Event, SessionId};
use error_stack::Report;
use futures::StreamExt as _;

use jinn_provider::{LlmService, LlmServiceError, OnRetry, RetryingLlmService};
use session::{SessionData, SessionState};

/// OnRetry callback that pushes a system chat entry to notify the user.
struct PushEntryOnRetry {
    sink: Arc<dyn MessageSink>,
    session_id: SessionId,
}

impl PushEntryOnRetry {
    fn new(sink: Arc<dyn MessageSink>, session_id: SessionId) -> Self {
        Self { sink, session_id }
    }
}

impl OnRetry for PushEntryOnRetry {
    fn on_retry(
        &self,
        attempt: u32,
        max_retries: u32,
        wait_duration: Duration,
        error: &Report<LlmServiceError>,
    ) {
        let secs = wait_duration.as_secs();
        let message = format!(
            "LLM request failed ({error}), retrying in {secs}s (attempt {attempt}/{max_retries})"
        );
        let _ = self
            .sink
            .send_command(Command::PushChatEntry(PushChatEntry {
                session_id: self.session_id.clone(),
                entry: ChatEntry::system(message),
            }));
    }
}

/// LLM streaming actor.
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

/// Dependencies for [`LlmActor`].
pub struct LlmActorDeps {
    /// Factory for creating LLM service instances.
    pub factory: LlmServiceFactoryService,
    /// Runtime services (provider registry, API keys for per-request factory creation).
    pub services: Option<Services>,
    /// Shared application state (for reading tool definitions).
    pub state: State,
}

impl Actor for LlmActor {
    type Message = NoDirectMsg;
    type Deps = LlmActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        ctx.set_description("LLM streaming with tool support");
        ctx.subscribe_command::<SendToLlmProvider>();
        ctx.subscribe_command::<CancelStream>();
        ctx.subscribe_event::<StreamCompleted>();

        Self {
            factory: deps.factory,
            services: deps.services,
            state: deps.state,
            tasks: HashMap::new(),
            sessions: HashMap::new(),
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<NoDirectMsg>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(command) => self.handle_command(&command, ctx),
            ActorEnvelope::Event(event) => self.handle_event(&event),
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
                self.start_stream(payload, ctx);
            }
            Command::CancelStream(payload) => {
                self.cancel_stream(&payload.session_id, ctx);
            }
            _ => {}
        }
    }

    /// Dispatches incoming events to the appropriate handler.
    fn handle_event(&mut self, event: &Event) {
        match event {
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
    fn start_stream(&mut self, payload: &SendToLlmProvider, ctx: &ActorContext) {
        let retry_config: jinn_provider::RetryConfig = match self.services.as_ref() {
            Some(services) => services
                .user_preferences_storage
                .load()
                .expect("preferences")
                .request_retry
                .to_retry_config(),
            None => crate::feat::preferences_actor::user_preferences::RequestRetryConfig::default()
                .to_retry_config(),
        };

        let tools = payload.tool_definitions.clone();
        let messages = payload.messages.clone();
        let session_id = payload.session_id.clone();

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

        // Track the session.
        self.sessions.insert(session_id.clone(), SessionData::new());

        // Resolve the factory: per-request if provider_id is set, global fallback otherwise.
        let factory = if let Some(pid) = payload.provider_id.as_deref() {
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
                            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
                        }));
                        return;
                    }
                }
            } else {
                // No services - fall through to global factory
                self.factory.clone()
            }
        } else {
            self.factory.clone()
        };
        let model_id = payload
            .provider_id
            .as_deref()
            .map(std::borrow::ToOwned::to_owned)
            .unwrap_or_default();
        let sink = ctx.sink();
        let sid = session_id.clone();

        let handle = tokio::spawn(async move {
            let service: Box<dyn LlmService> = match factory.create() {
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
                        cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
                    }));
                    return;
                }
            };

            let service = RetryingLlmService::new(
                service,
                retry_config,
                Box::new(PushEntryOnRetry::new(sink.clone(), sid.clone())),
            );

            let stream = match service.chat_stream_with_tools(messages, tools).await {
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
                        cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
                    }));
                    return;
                }
            };

            // Accumulate text and tool calls from the stream.
            let mut accumulated_text = String::new();
            let mut accumulated_thinking = String::new();
            let mut accumulated_tool_calls: Vec<ToolCall> = Vec::new();
            let mut token_index = 0usize;
            let mut parser = reasoning_parser::ParserFactory::new().create(&model_id);

            let mut stream_ended_normally = false;
            let mut stream = std::pin::pin!(stream);
            while let Some(result) = stream.next().await {
                match result {
                    Ok(event) => match event {
                        StreamEvent::Text(token) => {
                            tracing::info!(
                                session_id = ?sid,
                                token_len = token.len(),
                                token_preview = %&token[..token.len().min(50)],
                                "LLM ACTOR StreamEvent::Text"
                            );
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
                                accumulated_thinking.push_str(&parsed.reasoning_text);
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
                            tracing::info!(
                                session_id = ?sid,
                                token_len = token.len(),
                                token_preview = %&token[..token.len().min(50)],
                                "LLM ACTOR StreamEvent::Reasoning"
                            );
                            accumulated_thinking.push_str(&token);
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
                        StreamEvent::Done { stop_reason, usage } => {
                            stream_ended_normally = true;
                            tracing::trace!(
                                session_id = ?sid,
                                stop_reason = %stop_reason,
                                tool_call_count = accumulated_tool_calls.len(),
                                "stream Done"
                            );
                            let cost = usage.as_ref().and_then(|u| u.cost);
                            let provider_completion_tokens =
                                usage.as_ref().and_then(|u| u.completion_tokens);
                            let thinking_content = if accumulated_thinking.is_empty() {
                                None
                            } else {
                                Some(std::mem::take(&mut accumulated_thinking))
                            };
                            if stop_reason == StopReason::ToolUse {
                                // Emit ExecuteToolBatch for the orchestrator.
                                let _ = sink.send_command(Command::ExecuteToolBatch(
                                    ExecuteToolBatch {
                                        session_id: sid.clone(),
                                        tool_calls: accumulated_tool_calls.clone(),
                                    },
                                ));

                                // Emit StreamCompleted with ToolUse reason so the session
                                // actor can finalize output tokens. The continuation is
                                // handled by the session actor via context assembly when
                                // ToolBatchCompleted arrives.
                                let _ = sink.send_event(Event::StreamCompleted(StreamCompleted {
                                    session_id: sid.clone(),
                                    reason: StreamCompletedReason::ToolUse,
                                    assistant_content: Some(accumulated_text.clone()),
                                    tool_calls: Some(accumulated_tool_calls.clone()),
                                    cost,
                                    provider_completion_tokens,
                                    thinking_content,
                                }));
                            } else {
                                // Normal end_turn - emit StreamCompleted.
                                let _ = sink.send_event(Event::StreamCompleted(StreamCompleted {
                                    session_id: sid.clone(),
                                    reason: StreamCompletedReason::Finished,
                                    assistant_content: Some(accumulated_text.clone()),
                                    tool_calls: None,
                                    cost,
                                    provider_completion_tokens,
                                    thinking_content,
                                }));
                            }
                        }
                        StreamEvent::Error {
                            error_type,
                            message,
                        } => {
                            tracing::error!(
                                error_type = %error_type,
                                message = %message,
                                "LLM stream error event from provider"
                            );
                            let _ = sink.send_command(Command::PushChatEntry(PushChatEntry {
                                session_id: sid.clone(),
                                entry: ChatEntry::error(format!(
                                    "LLM error ({error_type}): {message}"
                                )),
                            }));
                            let _ = sink.send_event(Event::StreamCompleted(StreamCompleted {
                                session_id: sid.clone(),
                                reason: StreamCompletedReason::Error,
                                assistant_content: None,
                                tool_calls: None,
                                cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
                            }));
                            stream_ended_normally = true;
                            break;
                        }
                    },
                    Err(e) => {
                        tracing::error!(err = ?e, "LLM stream error");
                        let _ = sink.send_command(Command::PushChatEntry(PushChatEntry {
                            session_id: sid.clone(),
                            entry: ChatEntry::error(format!("LLM stream error: {e:?}")),
                        }));
                        let _ = sink.send_event(Event::StreamCompleted(StreamCompleted {
                            session_id: sid.clone(),
                            reason: StreamCompletedReason::Error,
                            assistant_content: None,
                            tool_calls: None,
                            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
                        }));
                        stream_ended_normally = true;
                        break;
                    }
                }
            }

            // Guard: if the stream ended without a terminal event, emit fallback error.
            if !stream_ended_normally {
                tracing::error!(
                    session_id = ?sid,
                    "LLM stream ended without a terminal event (Done/Error)"
                );
                let _ = sink.send_command(Command::PushChatEntry(PushChatEntry {
                    session_id: sid.clone(),
                    entry: ChatEntry::error(
                        "LLM stream ended unexpectedly. The connection may have been interrupted."
                            .to_owned(),
                    ),
                }));
                let _ = sink.send_event(Event::StreamCompleted(StreamCompleted {
                    session_id: sid.clone(),
                    reason: StreamCompletedReason::Error,
                    assistant_content: None,
                    tool_calls: None,
                    cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
                }));
            }
        });

        // Update session state.
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.state = SessionState::Streaming;
        }

        self.tasks.insert(session_id, handle);
    }

    /// Handles stream completion events to clean up session state.
    ///
    /// Removes the session from tracking for [`Finished`] and [`Error`] reasons.
    /// For [`ToolUse`], the session stays tracked until cancellation or the next
    /// stream starts - the session actor handles the continuation.
    fn handle_stream_completed(&mut self, payload: &StreamCompleted) {
        if !self.sessions.contains_key(&payload.session_id) {
            return;
        }

        match payload.reason {
            StreamCompletedReason::ToolUse => {
                // Session stays tracked - the continuation is handled by the
                // session actor when ToolBatchCompleted arrives. The next
                // start_stream call will reset this session.
                tracing::trace!(
                    session_id = ?payload.session_id,
                    reason = "ToolUse",
                    "handle_stream_completed - keeping session for continuation"
                );
            }
            StreamCompletedReason::Error | StreamCompletedReason::Finished => {
                tracing::trace!(
                    session_id = ?payload.session_id,
                    reason = ?payload.reason,
                    "handle_stream_completed - removing session"
                );
                self.sessions.remove(&payload.session_id);
            }
            StreamCompletedReason::Canceled => {
                // Already cleaned up by cancel_stream.
            }
        }
    }

    /// Cancels the active stream for a session and emits a completion event.
    fn cancel_stream(&mut self, session_id: &SessionId, ctx: &ActorContext) {
        // If there's an active session, cancel any pending tool batches.
        if self.sessions.contains_key(session_id)
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
        // Only emit StreamCompleted if there was actually an active session
        // to cancel. Avoids pushing a spurious "Cancelled" error entry when
        // the user presses ESC with nothing streaming.
        if had_session {
            let _ = ctx.send_event(Event::StreamCompleted(StreamCompleted {
                session_id: session_id.clone(),
                reason: StreamCompletedReason::Canceled,
                assistant_content: None,
                tool_calls: None,
                cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
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
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;

    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::provider_infra::LlmServiceFactoryService;
    use jinn_provider::FakeLlmServiceFactory;

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
            .insert(session_id.clone(), SessionData::new());

        // When handling StreamCompleted with Error reason.
        let payload = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Error,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.handle_stream_completed(&payload);

        // Then the session is removed from the sessions map.
        assert!(actor.sessions.is_empty());
    }

    // --- Phase 2: Additional LlmActor tests ---

    #[test]
    fn handle_stream_completed_finished_reason_removes_session() {
        // Given an LLM actor with a streaming session.
        let mut actor = test_llm_actor();
        let session_id = SessionId::new();
        actor
            .sessions
            .insert(session_id.clone(), SessionData::new());

        // When handling StreamCompleted with Finished reason.
        let payload = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("hello".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.handle_stream_completed(&payload);

        // Then the session is removed.
        assert!(
            actor.sessions.is_empty(),
            "Finished should remove the session"
        );
    }

    #[test]
    fn handle_stream_completed_tool_use_keeps_session() {
        // Given an LLM actor with a streaming session.
        let mut actor = test_llm_actor();
        let session_id = SessionId::new();
        actor
            .sessions
            .insert(session_id.clone(), SessionData::new());

        // When handling StreamCompleted with ToolUse reason.
        let payload = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::ToolUse,
            assistant_content: Some("thinking...".to_owned()),
            tool_calls: Some(vec![]),
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.handle_stream_completed(&payload);

        // Then the session is kept for continuation.
        assert!(
            actor.sessions.contains_key(&session_id),
            "ToolUse should keep the session for continuation"
        );
    }

    #[test]
    fn handle_stream_completed_unknown_session_is_noop() {
        // Given an LLM actor with NO sessions.
        let mut actor = test_llm_actor();
        let session_id = SessionId::new();

        // When handling StreamCompleted for an unknown session.
        let payload = StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Error,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
        };
        actor.handle_stream_completed(&payload);

        // Then nothing happens - no panic.
        assert!(actor.sessions.is_empty());
    }

    #[tokio::test]
    async fn cancel_stream_removes_session_and_task() {
        // Given an LLM actor with a session and a spawned task.
        use crate::common::actor::{ActorContext, RecordingSink};
        let sink = Arc::new(RecordingSink::new());
        let ctx = ActorContext::new("test-llm", sink.clone());

        let mut actor = test_llm_actor();
        let session_id = SessionId::new();
        actor.sessions.insert(session_id.clone(), SessionData::new());
        // Insert a dummy task that will be aborted.
        let handle = tokio::spawn(async { std::future::pending::<()>().await });
        actor.tasks.insert(session_id.clone(), handle);

        // When cancelling the stream.
        actor.cancel_stream(&session_id, &ctx);

        // Then the session and task are removed.
        assert!(!actor.sessions.contains_key(&session_id));
        assert!(!actor.tasks.contains_key(&session_id));

        // And a StreamCompleted(Canceled) event was emitted.
        let events = sink.events();
        let found = events.iter().any(|e| {
            if let Event::StreamCompleted(sc) = e {
                sc.reason == StreamCompletedReason::Canceled
                    && sc.session_id == session_id
            } else {
                false
            }
        });
        assert!(found, "should emit StreamCompleted with Canceled reason");
    }

    #[test]
    fn cancel_stream_without_session_emits_nothing() {
        // Given an LLM actor with no sessions.
        use crate::common::actor::{ActorContext, RecordingSink};
        let sink = Arc::new(RecordingSink::new());
        let ctx = ActorContext::new("test-llm", sink.clone());

        let mut actor = test_llm_actor();
        let session_id = SessionId::new();

        // When cancelling a stream for a session that doesn't exist.
        actor.cancel_stream(&session_id, &ctx);

        // Then no StreamCompleted event is emitted.
        let events = sink.events();
        assert!(
            events.is_empty(),
            "should not emit StreamCompleted for non-existent session"
        );
    }

    #[tokio::test]
    async fn cancel_all_aborts_all_tasks() {
        // Given an LLM actor with multiple spawned tasks.
        let mut actor = test_llm_actor();
        let sid1 = SessionId::new();
        let sid2 = SessionId::new();
        let h1 = tokio::spawn(async { std::future::pending::<()>().await });
        let h2 = tokio::spawn(async { std::future::pending::<()>().await });
        actor.tasks.insert(sid1, h1);
        actor.tasks.insert(sid2, h2);

        // When calling cancel_all.
        actor.cancel_all();

        // Then the tasks are aborted (JoinHandle.is_finished).
        // Yield to let abort propagate.
        tokio::task::yield_now().await;
        for handle in actor.tasks.values() {
            assert!(handle.is_finished(), "task should be aborted after cancel_all");
        }
    }

    #[tokio::test]
    async fn on_shutdown_cancels_all_tasks() {
        use crate::common::actor::{ActorContext, RecordingSink};
        let sink = Arc::new(RecordingSink::new());
        let ctx = ActorContext::new("test-llm", sink.clone());

        let mut actor = test_llm_actor();
        let sid = SessionId::new();
        let handle = tokio::spawn(async { std::future::pending::<()>().await });
        actor.tasks.insert(sid, handle);

        // When on_shutdown is called.
        actor.on_shutdown(&ctx).await;

        // Then the task is aborted.
        tokio::task::yield_now().await;
        for handle in actor.tasks.values() {
            assert!(handle.is_finished(), "task should be aborted after on_shutdown");
        }
    }

    #[tokio::test]
    async fn start_stream_emits_stream_completed_with_tokens() {
        // Given an LLM actor with a fake factory that returns tokens.
        use crate::common::actor::{ActorContext, RecordingSink};
        let sink = Arc::new(RecordingSink::new());
        let ctx = ActorContext::new("test-llm", sink.clone());

        let factory = LlmServiceFactoryService::new(Arc::new(
            FakeLlmServiceFactory::new(vec!["Hello".to_owned(), " World".to_owned()]),
        ));
        let mut actor = LlmActor {
            factory,
            services: None,
            state: State::new(AppState::default()),
            tasks: HashMap::new(),
            sessions: HashMap::new(),
        };

        let session_id = SessionId::new();
        let payload = SendToLlmProvider {
            session_id: session_id.clone(),
            messages: vec![],
            tool_definitions: vec![],
            provider_id: None,
            estimated_tokens: 0,
        };

        // When starting a stream.
        actor.start_stream(&payload, &ctx);

        // Then wait for the stream to complete.
        let mut stream_events = vec![];
        for _ in 0..100 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            stream_events = sink.events();
            if stream_events.iter().any(|e| {
                matches!(e, Event::StreamCompleted(sc) if sc.reason == StreamCompletedReason::Finished)
            }) {
                break;
            }
        }

        // And StreamToken events were emitted with sequential indices.
        let token_events: Vec<&StreamToken> = stream_events
            .iter()
            .filter_map(|e| match e {
                Event::StreamToken(st) => Some(st),
                _ => None,
            })
            .collect();
        assert_eq!(token_events.len(), 2, "should have 2 token events");
        assert_eq!(token_events[0].index, 0, "first token index should be 0");
        assert_eq!(token_events[1].index, 1, "second token index should be 1");

        // And StreamCompleted was emitted.
        let completed = stream_events.iter().find_map(|e| match e {
            Event::StreamCompleted(sc) if sc.reason == StreamCompletedReason::Finished => Some(sc.clone()),
            _ => None,
        });
        assert!(completed.is_some(), "should emit StreamCompleted(Finished)");
        let completed = completed.unwrap();
        assert_eq!(completed.assistant_content.as_deref(), Some("Hello World"));
    }

    #[tokio::test]
    async fn start_stream_aborts_existing_stream_for_same_session() {
        // Given an LLM actor with an existing stream for a session.
        use crate::common::actor::{ActorContext, RecordingSink};
        let sink = Arc::new(RecordingSink::new());
        let ctx = ActorContext::new("test-llm", sink.clone());

        let factory = LlmServiceFactoryService::new(Arc::new(
            FakeLlmServiceFactory::new(vec!["First".to_owned()]),
        ));
        let mut actor = LlmActor {
            factory,
            services: None,
            state: State::new(AppState::default()),
            tasks: HashMap::new(),
            sessions: HashMap::new(),
        };

        let session_id = SessionId::new();
        let payload = SendToLlmProvider {
            session_id: session_id.clone(),
            messages: vec![],
            tool_definitions: vec![],
            provider_id: None,
            estimated_tokens: 0,
        };

        // Start first stream and save the handle.
        actor.start_stream(&payload, &ctx);
        let first_handle = actor.tasks.remove(&session_id);
        assert!(first_handle.is_some());
        // Re-insert for the second start_stream to find and abort.
        actor.tasks.insert(session_id.clone(), first_handle.unwrap());

        // When starting a second stream for the same session.
        actor.start_stream(&payload, &ctx);

        // Then a new task exists (the old one was aborted internally).
        assert!(
            actor.tasks.contains_key(&session_id),
            "second stream should create a new task"
        );
    }

    #[tokio::test]
    async fn start_stream_sets_session_to_streaming() {
        // Given an LLM actor.
        use crate::common::actor::{ActorContext, RecordingSink};
        let sink = Arc::new(RecordingSink::new());
        let ctx = ActorContext::new("test-llm", sink.clone());

        let factory = LlmServiceFactoryService::new(Arc::new(
            FakeLlmServiceFactory::new(vec!["Hi".to_owned()]),
        ));
        let mut actor = LlmActor {
            factory,
            services: None,
            state: State::new(AppState::default()),
            tasks: HashMap::new(),
            sessions: HashMap::new(),
        };

        let session_id = SessionId::new();
        let payload = SendToLlmProvider {
            session_id: session_id.clone(),
            messages: vec![],
            tool_definitions: vec![],
            provider_id: None,
            estimated_tokens: 0,
        };

        // When starting a stream.
        actor.start_stream(&payload, &ctx);

        // Then the session state is Streaming.
        let session_data = actor.sessions.get(&session_id);
        assert!(session_data.is_some(), "session should be tracked");
        assert_eq!(
            session_data.unwrap().state,
            SessionState::Streaming,
            "session should be in Streaming state"
        );
    }

    #[tokio::test]
    async fn handle_command_dispatches_send_to_llm() {
        // Given an LLM actor.
        use crate::common::actor::{ActorContext, RecordingSink};
        let sink = Arc::new(RecordingSink::new());
        let ctx = ActorContext::new("test-llm", sink.clone());

        let factory = LlmServiceFactoryService::new(Arc::new(
            FakeLlmServiceFactory::new(vec!["response".to_owned()]),
        ));
        let mut actor = LlmActor {
            factory,
            services: None,
            state: State::new(AppState::default()),
            tasks: HashMap::new(),
            sessions: HashMap::new(),
        };

        let session_id = SessionId::new();
        let command = Command::SendToLlmProvider(SendToLlmProvider {
            session_id: session_id.clone(),
            messages: vec![],
            tool_definitions: vec![],
            provider_id: None,
            estimated_tokens: 0,
        });

        // When dispatching via handle_command.
        actor.handle_command(&command, &ctx);

        // Then a task is spawned for the session.
        assert!(actor.tasks.contains_key(&session_id));
    }

    #[tokio::test]
    async fn handle_command_dispatches_cancel() {
        // Given an LLM actor with a streaming session.
        use crate::common::actor::{ActorContext, RecordingSink};
        let sink = Arc::new(RecordingSink::new());
        let ctx = ActorContext::new("test-llm", sink.clone());

        let mut actor = test_llm_actor();
        let session_id = SessionId::new();
        actor.sessions.insert(session_id.clone(), SessionData::new());
        let handle = tokio::spawn(async { std::future::pending::<()>().await });
        actor.tasks.insert(session_id.clone(), handle);

        let command = Command::CancelStream(CancelStream {
            session_id: session_id.clone(),
        });

        // When dispatching via handle_command.
        actor.handle_command(&command, &ctx);

        // Then the task and session are removed.
        assert!(!actor.tasks.contains_key(&session_id));
        assert!(!actor.sessions.contains_key(&session_id));
    }
}
