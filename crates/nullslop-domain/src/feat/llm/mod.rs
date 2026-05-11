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

use crate::common::actor::{
    Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink, SystemMessage,
};
use crate::common::actor_host::{ActorSpawnResult, spawn_actor};
use crate::feat::provider_infra::LlmServiceFactoryService;
use crate::feat::provider_infra::StreamEvent;
use crate::feat::chat_input::protocol::command::PushChatEntry;
use crate::feat::provider::llm_message::LlmMessage;
use crate::feat::provider::protocol::command::{CancelStream, SendToLlmProvider};
use crate::feat::provider::protocol::event::{
    StreamCompleted, StreamCompletedReason, StreamToken,
};
use crate::feat::tools::protocol::command::{ExecuteToolBatch, PushToolResult};
use crate::feat::tools::protocol::event::{
    ToolBatchCompleted, ToolCallReceived, ToolCallStreaming, ToolUseStarted, ToolsRegistered,
};
use crate::feat::tools::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::protocol::{ChatEntry, Command, Event, SessionId};
use futures::StreamExt as _;

use session::{SessionData, SessionState};

/// Direct message type for the LLM actor.
///
/// Currently unused — the actor responds to bus commands and events.
/// Reserved for future intra-actor communication.
pub enum LlmDirectMsg {}

/// Spawns the LLM streaming actor on the given tokio runtime handle.
///
/// Creates the actor's channel, context, and run loop, returning the
/// [`ActorRef`] for sending direct messages and the [`ActorSpawnResult`]
/// for routing integration.
///
/// # Errors
///
/// Returns an error if the actor fails to activate.
pub fn spawn(
    llm_service: crate::feat::provider_infra::LlmServiceFactoryService,
    sink: Arc<dyn MessageSink>,
    handle: &tokio::runtime::Handle,
) -> (ActorRef<LlmDirectMsg>, ActorSpawnResult) {
    let (tx, rx) = kanal::unbounded::<ActorEnvelope<LlmDirectMsg>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new("llm-streaming", sink);
    ctx.set_description("LLM streaming with tool support");
    ctx.set_data(llm_service);
    let actor = LlmActor::activate(&mut ctx);
    let result = spawn_actor("llm-streaming", actor, &actor_ref, rx, ctx, handle);
    (actor_ref, result)
}

/// LLM streaming actor with tool support.
///
/// Holds a reference to the LLM service factory and tracks active
/// streaming tasks and per-session state.
pub struct LlmActor {
    /// Factory for creating LLM service instances.
    factory: LlmServiceFactoryService,
    /// Active stream tasks, keyed by session ID.
    tasks: HashMap<SessionId, tokio::task::JoinHandle<()>>,
    /// Per-session state.
    sessions: HashMap<SessionId, SessionData>,
    /// Accumulated tool definitions from [`ToolsRegistered`] events.
    tool_definitions: HashMap<String, ToolDefinition>,
}

impl Actor for LlmActor {
    type Message = LlmDirectMsg;

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

        Self {
            factory,
            tasks: HashMap::new(),
            sessions: HashMap::new(),
            tool_definitions: HashMap::new(),
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<LlmDirectMsg>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(command) => self.handle_command(&command, ctx),
            ActorEnvelope::Event(event) => self.handle_event(&event, ctx),
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                self.cancel_all();
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::System(SystemMessage::ApplicationReady) => {
                ctx.announce_started();
            }
            ActorEnvelope::Direct(_) | ActorEnvelope::Shutdown => {}
        }
    }

    async fn shutdown(self) {
        self.cancel_all();
    }
}

impl LlmActor {
    /// Dispatches incoming commands to the appropriate handler.
    fn handle_command(&mut self, command: &Command, ctx: &ActorContext) {
        match command {
            Command::SendToLlmProvider { payload } => {
                self.start_stream(payload.session_id.clone(), payload.messages.clone(), ctx);
            }
            Command::CancelStream { payload } => {
                self.cancel_stream(&payload.session_id, ctx);
            }
            _ => {}
        }
    }

    /// Dispatches incoming events to the appropriate handler.
    fn handle_event(&mut self, event: &Event, ctx: &ActorContext) {
        match event {
            Event::ToolsRegistered { payload } => {
                self.handle_tools_registered(&payload.definitions);
            }
            Event::ToolBatchCompleted { payload } => {
                self.handle_tool_batch_completed(payload.session_id.clone(), &payload.results, ctx);
            }
            Event::StreamCompleted { payload } => {
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
        ctx: &ActorContext,
    ) {
        // Collect current tool definitions.
        let tools: Vec<ToolDefinition> = self.tool_definitions.values().cloned().collect();

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
        let session = SessionData::new(messages);
        self.sessions.insert(session_id.clone(), session);

        let factory = self.factory.clone();
        let sink = ctx.sink();
        let sid = session_id.clone();

        let handle = tokio::spawn(async move {
            let service = match factory.create() {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(err = ?e, "failed to create LLM service");
                    let _ = sink.send_command(Command::PushChatEntry {
                        payload: PushChatEntry {
                            session_id: sid.clone(),
                            entry: ChatEntry::system("LLM service creation failed"),
                        },
                    });
                    let _ = sink.send_event(Event::StreamCompleted {
                        payload: StreamCompleted {
                            session_id: sid,
                            reason: StreamCompletedReason::Finished,
                            assistant_content: None,
                            tool_calls: None,
                        },
                    });
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
                    let _ = sink.send_command(Command::PushChatEntry {
                        payload: PushChatEntry {
                            session_id: sid.clone(),
                            entry: ChatEntry::system(format!("LLM stream error: {e:?}")),
                        },
                    });
                    let _ = sink.send_event(Event::StreamCompleted {
                        payload: StreamCompleted {
                            session_id: sid,
                            reason: StreamCompletedReason::Finished,
                            assistant_content: None,
                            tool_calls: None,
                        },
                    });
                    return;
                }
            };

            // Accumulate text and tool calls from the stream.
            let mut accumulated_text = String::new();
            let mut accumulated_tool_calls: Vec<ToolCall> = Vec::new();
            let mut token_index = 0usize;

            let mut stream = std::pin::pin!(stream);
            while let Some(result) = stream.next().await {
                match result {
                    Ok(event) => match event {
                        StreamEvent::Text(token) => {
                            accumulated_text.push_str(&token);
                            let _ = sink.send_event(Event::StreamToken {
                                payload: StreamToken {
                                    session_id: sid.clone(),
                                    index: token_index,
                                    token,
                                },
                            });
                            token_index += 1;
                        }
                        StreamEvent::ToolUseStart { index, id, name } => {
                            let _ = sink.send_event(Event::ToolUseStarted {
                                payload: ToolUseStarted {
                                    session_id: sid.clone(),
                                    index,
                                    id,
                                    name,
                                },
                            });
                        }
                        StreamEvent::ToolUseInputDelta {
                            index,
                            partial_json,
                        } => {
                            let _ = sink.send_event(Event::ToolCallStreaming {
                                payload: ToolCallStreaming {
                                    session_id: sid.clone(),
                                    index,
                                    partial_json,
                                },
                            });
                        }
                        StreamEvent::ToolUseComplete { tool_call, .. } => {
                            accumulated_tool_calls.push(tool_call.clone());
                            let _ = sink.send_event(Event::ToolCallReceived {
                                payload: ToolCallReceived {
                                    session_id: sid.clone(),
                                    tool_call,
                                },
                            });
                        }
                        StreamEvent::Done { stop_reason } => {
                            tracing::trace!(
                                session_id = ?sid,
                                stop_reason = %stop_reason,
                                tool_call_count = accumulated_tool_calls.len(),
                                "stream Done"
                            );
                            if stop_reason == "tool_use" {
                                // Emit ExecuteToolBatch for the orchestrator.
                                let _ = sink.send_command(Command::ExecuteToolBatch {
                                    payload: ExecuteToolBatch {
                                        session_id: sid.clone(),
                                        tool_calls: accumulated_tool_calls.clone(),
                                    },
                                });

                                // Emit StreamCompleted with ToolUse reason so the actor
                                // can transition state.
                                let _ = sink.send_event(Event::StreamCompleted {
                                    payload: StreamCompleted {
                                        session_id: sid.clone(),
                                        reason: StreamCompletedReason::ToolUse,
                                        assistant_content: Some(accumulated_text.clone()),
                                        tool_calls: Some(accumulated_tool_calls.clone()),
                                    },
                                });
                            } else {
                                // Normal end_turn — emit StreamCompleted.
                                let _ = sink.send_event(Event::StreamCompleted {
                                    payload: StreamCompleted {
                                        session_id: sid.clone(),
                                        reason: StreamCompletedReason::Finished,
                                        assistant_content: Some(accumulated_text.clone()),
                                        tool_calls: None,
                                    },
                                });
                            }
                        }
                    },
                    Err(e) => {
                        tracing::error!(err = ?e, "LLM stream error");
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
            StreamCompletedReason::Finished => {
                // Defensive guard: if the session is awaiting tool results, a
                // duplicate Done from the provider should not remove the session.
                if session.state == SessionState::AwaitingToolResults {
                    tracing::warn!(
                        session_id = ?payload.session_id,
                        state = ?session.state,
                        "received StreamCompleted(Finished) while awaiting tool results — ignoring duplicate Done"
                    );
                    return;
                }
                // Clean up the completed session.
                tracing::trace!(
                    session_id = ?payload.session_id,
                    reason = "Finished",
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

        // Emit PushToolResult for each result.
        for result in results {
            let _ = ctx.send_command(Command::PushToolResult {
                payload: PushToolResult {
                    session_id: session_id.clone(),
                    result: result.clone(),
                },
            });
        }

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
        let messages = std::mem::take(&mut session.messages);
        self.start_stream(session_id, messages, ctx);
    }

    /// Caches tool definitions from a [`ToolsRegistered`] event.
    fn handle_tools_registered(&mut self, definitions: &[ToolDefinition]) {
        for def in definitions {
            self.tool_definitions.insert(def.name.clone(), def.clone());
        }
    }

    /// Cancels the active stream for a session and emits a completion event.
    fn cancel_stream(&mut self, session_id: &SessionId, ctx: &ActorContext) {
        if let Some(handle) = self.tasks.remove(session_id) {
            handle.abort();
        }
        self.sessions.remove(session_id);
        let _ = ctx.send_event(Event::StreamCompleted {
            payload: StreamCompleted {
                session_id: session_id.clone(),
                reason: StreamCompletedReason::Canceled,
                assistant_content: None,
                tool_calls: None,
            },
        });
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
    use std::sync::Arc;

    use crate::common::actor::RecordingSink;
    use crate::feat::provider_infra::FakeLlmServiceFactory;
    use crate::protocol::EventMsg as _;
    use crate::feat::tools::protocol::event::ToolsRegistered;
    use crate::feat::tools::tool_types::ToolDefinition;

    use super::*;

    /// Creates a test context backed by a recording sink.
    fn test_context(sink: &Arc<RecordingSink>) -> ActorContext {
        ActorContext::new("test-llm", sink.clone())
    }

    /// Creates an actor with a fake factory producing the given tokens (text only).
    fn actor_with_tokens(
        _sink: &Arc<RecordingSink>,
        ctx: &mut ActorContext,
        tokens: Vec<String>,
    ) -> LlmActor {
        let factory = FakeLlmServiceFactory::new(tokens);
        let factory_service =
            crate::feat::provider_infra::LlmServiceFactoryService::new(Arc::new(factory));
        ctx.set_data(factory_service);
        LlmActor::activate(ctx)
    }

    /// Creates an actor with a fake factory producing text tokens and tool calls.
    fn actor_with_tool_calls(
        _sink: &Arc<RecordingSink>,
        ctx: &mut ActorContext,
        tokens: Vec<String>,
        tool_calls: Vec<ToolCall>,
    ) -> LlmActor {
        let factory = FakeLlmServiceFactory::with_tool_calls(tokens, tool_calls);
        let factory_service =
            crate::feat::provider_infra::LlmServiceFactoryService::new(Arc::new(factory));
        ctx.set_data(factory_service);
        LlmActor::activate(ctx)
    }

    /// Extracts `StreamCompleted` events from a list of events.
    fn find_stream_completed(events: &[Event]) -> Vec<&StreamCompleted> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::StreamCompleted { payload } => Some(payload),
                _ => None,
            })
            .collect()
    }

    /// Extracts `StreamToken` events.
    fn find_stream_tokens(events: &[Event]) -> Vec<&StreamToken> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::StreamToken { payload } => Some(payload),
                _ => None,
            })
            .collect()
    }

    // --- Activation tests ---

    #[rstest::rstest]
    fn activate_subscribes_to_commands_and_events() {
        // Given a fresh actor context.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);

        // When activating the actor.
        let _actor = actor_with_tokens(&sink, &mut ctx, vec![]);

        // Then the context accumulated subscriptions.
        let (events, _commands) = ctx.take_registrations();
        assert!(events.contains(&ToolBatchCompleted::TYPE_NAME.to_owned()));
        assert!(events.contains(&ToolsRegistered::TYPE_NAME.to_owned()));
        assert!(events.contains(&StreamCompleted::TYPE_NAME.to_owned()));
    }

    // --- Text-only streaming tests ---

    #[rstest::rstest]
    #[tokio::test]
    async fn text_stream_emits_tokens() {
        // Given an actor with text tokens.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let mut actor = actor_with_tokens(
            &sink,
            &mut ctx,
            vec!["Hello".to_owned(), " world".to_owned()],
        );
        sink.clear();

        let session_id = SessionId::new();

        // When sending SendToLlmProvider.
        let cmd = Command::SendToLlmProvider {
            payload: SendToLlmProvider {
                session_id: session_id.clone(),
                messages: vec![LlmMessage::User {
                    content: "hi".to_owned(),
                }],
                provider_id: None,
            },
        };
        actor.handle_command(&cmd, &ctx);

        // Wait for the stream task to complete.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Then StreamToken events were emitted.
        let events = sink.take_events();
        let tokens = find_stream_tokens(&events);
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].token, "Hello");
        assert_eq!(tokens[1].token, " world");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn text_stream_emits_completed_with_finished() {
        // Given an actor with text tokens.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let mut actor = actor_with_tokens(
            &sink,
            &mut ctx,
            vec!["Hello".to_owned(), " world".to_owned()],
        );
        sink.clear();

        let session_id = SessionId::new();

        // When sending SendToLlmProvider.
        let cmd = Command::SendToLlmProvider {
            payload: SendToLlmProvider {
                session_id: session_id.clone(),
                messages: vec![LlmMessage::User {
                    content: "hi".to_owned(),
                }],
                provider_id: None,
            },
        };
        actor.handle_command(&cmd, &ctx);

        // Wait for the stream task to complete.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Then a StreamCompleted event was emitted with Finished reason.
        let events = sink.take_events();
        let completed = find_stream_completed(&events);
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].reason, StreamCompletedReason::Finished);
        assert_eq!(
            completed[0].assistant_content,
            Some("Hello world".to_owned())
        );
        assert_eq!(completed[0].tool_calls, None);
    }

    // --- Tool use streaming tests ---

    #[rstest::rstest]
    #[tokio::test]
    async fn tool_stream_emits_tool_use_events() {
        // Given an actor configured with tool calls.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let tool_call = ToolCall {
            id: "call_1".to_owned(),
            name: "echo".to_owned(),
            arguments: r#"{"input":"hi"}"#.to_owned(),
        };
        let mut actor = actor_with_tool_calls(
            &sink,
            &mut ctx,
            vec!["Let me check".to_owned()],
            vec![tool_call.clone()],
        );
        sink.clear();

        let session_id = SessionId::new();

        // When sending SendToLlmProvider.
        let cmd = Command::SendToLlmProvider {
            payload: SendToLlmProvider {
                session_id: session_id.clone(),
                messages: vec![LlmMessage::User {
                    content: "hi".to_owned(),
                }],
                provider_id: None,
            },
        };
        actor.handle_command(&cmd, &ctx);

        // Wait for the stream task to complete.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Then ToolUseStarted, ToolCallStreaming, ToolCallReceived events were emitted.
        let events = sink.events();
        let has_tool_use_started = events.iter().any(|e| {
            matches!(
                e,
                Event::ToolUseStarted { payload }
                if payload.id == "call_1" && payload.name == "echo"
            )
        });
        let has_tool_call_streaming = events
            .iter()
            .any(|e| matches!(e, Event::ToolCallStreaming { .. }));
        let has_tool_call_received = events.iter().any(|e| {
            matches!(
                e,
                Event::ToolCallReceived { payload }
                if payload.tool_call == tool_call
            )
        });
        assert!(has_tool_use_started, "expected ToolUseStarted event");
        assert!(has_tool_call_streaming, "expected ToolCallStreaming event");
        assert!(has_tool_call_received, "expected ToolCallReceived event");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn tool_stream_emits_execute_batch_command() {
        // Given an actor configured with tool calls.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let tool_call = ToolCall {
            id: "call_1".to_owned(),
            name: "echo".to_owned(),
            arguments: r#"{"input":"hi"}"#.to_owned(),
        };
        let mut actor = actor_with_tool_calls(
            &sink,
            &mut ctx,
            vec!["Let me check".to_owned()],
            vec![tool_call.clone()],
        );
        sink.clear();

        let session_id = SessionId::new();

        // When sending SendToLlmProvider.
        let cmd = Command::SendToLlmProvider {
            payload: SendToLlmProvider {
                session_id: session_id.clone(),
                messages: vec![LlmMessage::User {
                    content: "hi".to_owned(),
                }],
                provider_id: None,
            },
        };
        actor.handle_command(&cmd, &ctx);

        // Wait for the stream task to complete.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Then an ExecuteToolBatch command was emitted.
        let commands = sink.commands();
        let has_execute_batch = commands.iter().any(|c| {
            matches!(
                c,
                Command::ExecuteToolBatch { payload }
                if payload.tool_calls.len() == 1 && payload.tool_calls[0] == tool_call
            )
        });
        assert!(has_execute_batch, "expected ExecuteToolBatch command");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn tool_stream_emits_stream_completed_with_tool_use() {
        // Given an actor configured with tool calls.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let tool_call = ToolCall {
            id: "call_1".to_owned(),
            name: "echo".to_owned(),
            arguments: r#"{"input":"hi"}"#.to_owned(),
        };
        let mut actor = actor_with_tool_calls(
            &sink,
            &mut ctx,
            vec!["Let me check".to_owned()],
            vec![tool_call.clone()],
        );
        sink.clear();

        let session_id = SessionId::new();

        // When sending SendToLlmProvider.
        let cmd = Command::SendToLlmProvider {
            payload: SendToLlmProvider {
                session_id: session_id.clone(),
                messages: vec![LlmMessage::User {
                    content: "hi".to_owned(),
                }],
                provider_id: None,
            },
        };
        actor.handle_command(&cmd, &ctx);

        // Wait for the stream task to complete.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Then a StreamCompleted event with ToolUse reason.
        let events = sink.events();
        let completed = find_stream_completed(&events);
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].reason, StreamCompletedReason::ToolUse);
        assert_eq!(
            completed[0].assistant_content,
            Some("Let me check".to_owned())
        );
        assert_eq!(completed[0].tool_calls, Some(vec![tool_call]));
    }

    // --- Tool batch completed → new stream tests ---

    #[rstest::rstest]
    #[tokio::test]
    async fn tool_results_received_sets_awaiting_state() {
        // Given an actor configured with tool calls.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);

        let tool_call = ToolCall {
            id: "call_1".to_owned(),
            name: "echo".to_owned(),
            arguments: r#"{"input":"hi"}"#.to_owned(),
        };
        let factory = FakeLlmServiceFactory::with_tool_calls(
            vec!["Let me check".to_owned()],
            vec![tool_call.clone()],
        );
        let factory_service =
            crate::feat::provider_infra::LlmServiceFactoryService::new(Arc::new(factory));
        ctx.set_data(factory_service);
        let mut actor = LlmActor::activate(&mut ctx);
        sink.clear();

        let session_id = SessionId::new();

        // When sending SendToLlmProvider and routing stream events back.
        let cmd = Command::SendToLlmProvider {
            payload: SendToLlmProvider {
                session_id: session_id.clone(),
                messages: vec![LlmMessage::User {
                    content: "hi".to_owned(),
                }],
                provider_id: None,
            },
        };
        actor.handle_command(&cmd, &ctx);

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let events_from_stream = sink.take_events();
        for event in events_from_stream {
            actor.handle_event(&event, &ctx);
        }

        // Then the session is in AwaitingToolResults state.
        let session = actor
            .sessions
            .get(&session_id)
            .expect("session should exist");
        assert_eq!(session.state, SessionState::AwaitingToolResults);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn tool_batch_completed_starts_new_stream() {
        // Given an actor in AwaitingToolResults state after a tool_use stream.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);

        let tool_call = ToolCall {
            id: "call_1".to_owned(),
            name: "echo".to_owned(),
            arguments: r#"{"input":"hi"}"#.to_owned(),
        };
        let factory = FakeLlmServiceFactory::with_tool_calls(
            vec!["Let me check".to_owned()],
            vec![tool_call.clone()],
        );
        let factory_service =
            crate::feat::provider_infra::LlmServiceFactoryService::new(Arc::new(factory));
        ctx.set_data(factory_service);
        let mut actor = LlmActor::activate(&mut ctx);
        sink.clear();

        let session_id = SessionId::new();

        let cmd = Command::SendToLlmProvider {
            payload: SendToLlmProvider {
                session_id: session_id.clone(),
                messages: vec![LlmMessage::User {
                    content: "hi".to_owned(),
                }],
                provider_id: None,
            },
        };
        actor.handle_command(&cmd, &ctx);

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let events_from_stream = sink.take_events();
        for event in events_from_stream {
            actor.handle_event(&event, &ctx);
        }
        sink.clear();

        // When receiving ToolBatchCompleted.
        let tool_result = ToolResult {
            tool_call_id: "call_1".to_owned(),
            name: "echo".to_owned(),
            content: "hi".to_owned(),
            success: true,
        };
        let batch_event = Event::ToolBatchCompleted {
            payload: ToolBatchCompleted {
                session_id: session_id.clone(),
                results: vec![tool_result],
            },
        };
        actor.handle_event(&batch_event, &ctx);

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Then new stream tokens were emitted (from the second stream call).
        let events = sink.events();
        let tokens = find_stream_tokens(&events);
        assert!(
            !tokens.is_empty(),
            "expected StreamToken events from new stream"
        );

        // And a PushToolResult command was emitted.
        let commands = sink.commands();
        let has_push_tool_result = commands
            .iter()
            .any(|c| matches!(c, Command::PushToolResult { .. }));
        assert!(has_push_tool_result, "expected PushToolResult command");
    }

    // --- Cancel tests ---

    #[rstest::rstest]
    #[tokio::test]
    async fn cancel_stream_emits_canceled_event() {
        // Given an actor with a stream in progress.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let mut actor = actor_with_tokens(&sink, &mut ctx, vec!["Hello".to_owned()]);
        sink.clear();

        let session_id = SessionId::new();

        let cmd = Command::SendToLlmProvider {
            payload: SendToLlmProvider {
                session_id: session_id.clone(),
                messages: vec![],
                provider_id: None,
            },
        };
        actor.handle_command(&cmd, &ctx);

        // When cancelling the stream.
        let cancel_cmd = Command::CancelStream {
            payload: CancelStream {
                session_id: session_id.clone(),
            },
        };
        actor.handle_command(&cancel_cmd, &ctx);

        // Then a StreamCompleted event with Canceled reason was emitted.
        let events = sink.events();
        let completed = find_stream_completed(&events);
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].reason, StreamCompletedReason::Canceled);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn cancel_stream_removes_task() {
        // Given an actor with a stream in progress.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let mut actor = actor_with_tokens(&sink, &mut ctx, vec!["Hello".to_owned()]);
        sink.clear();

        let session_id = SessionId::new();

        let cmd = Command::SendToLlmProvider {
            payload: SendToLlmProvider {
                session_id: session_id.clone(),
                messages: vec![],
                provider_id: None,
            },
        };
        actor.handle_command(&cmd, &ctx);

        // When cancelling the stream.
        let cancel_cmd = Command::CancelStream {
            payload: CancelStream {
                session_id: session_id.clone(),
            },
        };
        actor.handle_command(&cancel_cmd, &ctx);

        // Then the task was removed.
        assert!(!actor.tasks.contains_key(&session_id));
    }

    // --- ToolsRegistered event tests ---

    #[rstest::rstest]
    fn tools_registered_updates_definitions() {
        // Given an activated actor.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let mut actor = actor_with_tokens(&sink, &mut ctx, vec![]);
        sink.clear();

        let definition = ToolDefinition {
            name: "web_search".to_owned(),
            description: "Search the web".to_owned(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        };

        // When receiving a ToolsRegistered event.
        let event = Event::ToolsRegistered {
            payload: ToolsRegistered {
                provider: "web-actor".to_owned(),
                definitions: vec![definition.clone()],
            },
        };
        actor.handle_event(&event, &ctx);

        // Then the tool definition is cached.
        assert!(actor.tool_definitions.contains_key("web_search"));
        assert_eq!(actor.tool_definitions.get("web_search"), Some(&definition));
    }

    // --- Session state tests ---

    #[rstest::rstest]
    #[tokio::test]
    async fn session_transitions_to_streaming_on_send() {
        // Given an activated actor.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let mut actor = actor_with_tokens(&sink, &mut ctx, vec![]);
        sink.clear();

        let session_id = SessionId::new();

        // When sending SendToLlmProvider.
        let cmd = Command::SendToLlmProvider {
            payload: SendToLlmProvider {
                session_id: session_id.clone(),
                messages: vec![],
                provider_id: None,
            },
        };
        actor.handle_command(&cmd, &ctx);

        // Then the session state is Streaming.
        let session = actor
            .sessions
            .get(&session_id)
            .expect("session should exist");
        assert_eq!(session.state, SessionState::Streaming);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn stream_completed_transitions_to_awaiting_tool_results() {
        // Given an actor with a tool_use stream that completed.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let tool_call = ToolCall {
            id: "call_1".to_owned(),
            name: "echo".to_owned(),
            arguments: r#"{"input":"hi"}"#.to_owned(),
        };
        let mut actor = actor_with_tool_calls(
            &sink,
            &mut ctx,
            vec!["Let me check".to_owned()],
            vec![tool_call.clone()],
        );
        sink.clear();

        let session_id = SessionId::new();

        let cmd = Command::SendToLlmProvider {
            payload: SendToLlmProvider {
                session_id: session_id.clone(),
                messages: vec![],
                provider_id: None,
            },
        };
        actor.handle_command(&cmd, &ctx);

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // When processing the StreamCompleted(ToolUse) event from the stream task.
        let events_from_stream = sink.take_events();
        for event in events_from_stream {
            actor.handle_event(&event, &ctx);
        }

        // Then the session transitions to AwaitingToolResults.
        let session = actor
            .sessions
            .get(&session_id)
            .expect("session should exist");
        assert_eq!(session.state, SessionState::AwaitingToolResults);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn stream_completed_stores_accumulated_data() {
        // Given an actor with a tool_use stream that completed.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let tool_call = ToolCall {
            id: "call_1".to_owned(),
            name: "echo".to_owned(),
            arguments: r#"{"input":"hi"}"#.to_owned(),
        };
        let mut actor = actor_with_tool_calls(
            &sink,
            &mut ctx,
            vec!["Let me check".to_owned()],
            vec![tool_call.clone()],
        );
        sink.clear();

        let session_id = SessionId::new();

        let cmd = Command::SendToLlmProvider {
            payload: SendToLlmProvider {
                session_id: session_id.clone(),
                messages: vec![],
                provider_id: None,
            },
        };
        actor.handle_command(&cmd, &ctx);

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // When processing the StreamCompleted(ToolUse) event from the stream task.
        let events_from_stream = sink.take_events();
        for event in events_from_stream {
            actor.handle_event(&event, &ctx);
        }

        // Then the accumulated data was stored in the session.
        let session = actor
            .sessions
            .get(&session_id)
            .expect("session should exist");
        assert_eq!(session.accumulated_text, "Let me check");
        assert_eq!(session.accumulated_tool_calls, vec![tool_call]);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn stream_completed_finished_removes_session() {
        // Given an actor with a text-only stream that completed.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let mut actor = actor_with_tokens(&sink, &mut ctx, vec!["Hello".to_owned()]);
        sink.clear();

        let session_id = SessionId::new();

        let cmd = Command::SendToLlmProvider {
            payload: SendToLlmProvider {
                session_id: session_id.clone(),
                messages: vec![],
                provider_id: None,
            },
        };
        actor.handle_command(&cmd, &ctx);

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // When processing the StreamCompleted(Finished) event.
        let events_from_stream = sink.take_events();
        for event in events_from_stream {
            actor.handle_event(&event, &ctx);
        }

        // Then the session was removed (cleaned up).
        assert!(
            !actor.sessions.contains_key(&session_id),
            "session should be removed after Finished"
        );
    }

    // --- Defensive guard tests ---

    #[rstest::rstest]
    #[tokio::test]
    async fn duplicate_finished_while_awaiting_tool_results_is_ignored() {
        // Given an actor in AwaitingToolResults state after a tool_use stream.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(&sink);
        let tool_call = ToolCall {
            id: "call_1".to_owned(),
            name: "echo".to_owned(),
            arguments: r#"{"input":"hi"}"#.to_owned(),
        };
        let mut actor = actor_with_tool_calls(
            &sink,
            &mut ctx,
            vec!["Let me check".to_owned()],
            vec![tool_call.clone()],
        );
        sink.clear();

        let session_id = SessionId::new();

        let cmd = Command::SendToLlmProvider {
            payload: SendToLlmProvider {
                session_id: session_id.clone(),
                messages: vec![],
                provider_id: None,
            },
        };
        actor.handle_command(&cmd, &ctx);

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let events_from_stream = sink.take_events();
        for event in events_from_stream {
            actor.handle_event(&event, &ctx);
        }

        // When receiving a duplicate StreamCompleted(Finished) — simulates OpenRouter bug.
        let duplicate_finished = Event::StreamCompleted {
            payload: StreamCompleted {
                session_id: session_id.clone(),
                reason: StreamCompletedReason::Finished,
                assistant_content: None,
                tool_calls: None,
            },
        };
        actor.handle_event(&duplicate_finished, &ctx);

        // Then the session still exists and is still AwaitingToolResults.
        let session = actor
            .sessions
            .get(&session_id)
            .expect("session should still exist after duplicate Finished");
        assert_eq!(
            session.state,
            SessionState::AwaitingToolResults,
            "session should remain in AwaitingToolResults"
        );
    }
}
