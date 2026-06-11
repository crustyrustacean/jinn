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

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::services::bus_service::BusService;
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
use crate::protocol::{ChatEntry, SessionId};
use error_stack::Report;
use futures::StreamExt as _;

use jinn_provider::{LlmService, LlmServiceError, OnRetry, RetryingLlmService};
use kameo::actor::{ActorRef, WeakActorRef};
use kameo::error::ActorStopReason;
use kameo::message::{Context as MsgContext, Message};
use kameo::Actor;
use session::SessionData;

/// OnRetry callback that pushes a system chat entry to notify the user.
struct PushEntryOnRetry {
    bus: BusService,
    session_id: SessionId,
}

impl PushEntryOnRetry {
    fn new(bus: BusService, session_id: SessionId) -> Self {
        Self { bus, session_id }
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
        let bus = self.bus.clone();
        let session_id = self.session_id.clone();
        tokio::spawn(async move {
            bus.publish(PushChatEntry {
                session_id,
                entry: ChatEntry::system(message),
            })
            .await;
        });
    }
}

/// LLM streaming actor.
///
/// Holds a reference to the LLM service factory and tracks active
/// streaming tasks and per-session state.
pub struct LlmActor {
    /// Factory for creating LLM service instances.
    factory: LlmServiceFactoryService,
    /// Shared dependencies (services, bus).
    deps: ActorDeps,
    /// Shared application state (for reading tool definitions).
    _state: State,
    /// Active stream tasks, keyed by session ID.
    tasks: HashMap<SessionId, tokio::task::JoinHandle<()>>,
    /// Per-session state.
    sessions: HashMap<SessionId, SessionData>,
}

/// Dependencies for [`LlmActor`].
pub struct LlmActorDeps {
    /// Factory for creating LLM service instances.
    pub factory: LlmServiceFactoryService,
    /// Shared dependencies (services, bus).
    pub deps: ActorDeps,
    /// Shared application state (for reading tool definitions).
    pub state: State,
}

impl Actor for LlmActor {
    type Args = LlmActorDeps;
    type Error = std::convert::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        let deps = &args.deps;
        deps.subscribe(actor_ref.clone().recipient::<SendToLlmProvider>())
            .await;
        deps.subscribe(actor_ref.clone().recipient::<CancelStream>()).await;
        deps.subscribe(actor_ref.recipient::<StreamCompleted>())
            .await;

        Ok(Self {
            factory: args.factory,
            deps: args.deps,
            _state: args.state,
            tasks: HashMap::new(),
            sessions: HashMap::new(),
        })
    }

    async fn on_stop(
        &mut self,
        _actor_ref: WeakActorRef<Self>,
        _reason: ActorStopReason,
    ) -> Result<(), Self::Error> {
        self.cancel_all();
        Ok(())
    }
}

impl BusPublish for LlmActor {
    fn bus(&self) -> &BusService {
        &self.deps.services.bus
    }
}

impl Message<SendToLlmProvider> for LlmActor {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: SendToLlmProvider,
        _ctx: &mut MsgContext<Self, Self::Reply>,
    ) {
        self.start_stream(msg);
    }
}

impl Message<CancelStream> for LlmActor {
    type Reply = ();

    async fn handle(&mut self, msg: CancelStream, _ctx: &mut MsgContext<Self, Self::Reply>) {
        self.cancel_stream(&msg.session_id).await;
    }
}

impl Message<StreamCompleted> for LlmActor {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: StreamCompleted,
        _ctx: &mut MsgContext<Self, Self::Reply>,
    ) {
        self.handle_stream_completed(&msg);
    }
}

impl LlmActor {
    /// Starts an LLM streaming response for a session, aborting any existing stream.
    #[expect(
        clippy::too_many_lines,
        reason = "stream handling is inherently linear; splitting would obscure the flow"
    )]
    fn start_stream(&mut self, payload: SendToLlmProvider) {
        let retry_config = self
            .deps
            .services
            .user_preferences_storage
            .read()
            .request_retry
            .to_retry_config();

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
        let factory = if let Some(pid) = payload.provider_id.clone() {
            let id = crate::feat::provider_infra::ProviderId::new(pid.clone());
            let api_keys = self.deps.services.api_keys.read();
            match self
                .deps
                .services
                .provider_registry
                .create_factory(&id, &api_keys)
            {
                Ok(f) => {
                    tracing::debug!(provider_id = %pid, "created per-request LLM factory");
                    LlmServiceFactoryService::new(Arc::from(f))
                }
                Err(e) => {
                    tracing::error!(err = ?e, provider_id = %pid, "failed to create per-request factory");
                    let bus = self.deps.services.bus.clone();
                    let sid = session_id.clone();
                    tokio::spawn(async move {
                        bus.publish(PushChatEntry {
                            session_id: sid.clone(),
                            entry: ChatEntry::error(format!(
                                "LLM factory creation failed for {pid}: {e:?}"
                            )),
                        })
                        .await;
                        bus.publish(StreamCompleted {
                            session_id: sid,
                            reason: StreamCompletedReason::Error,
                            assistant_content: None,
                            tool_calls: None,
                            cost: None,
                            provider_completion_tokens: None,
                            thinking_content: None,
                        })
                        .await;
                    });
                    return;
                }
            }
        } else {
            self.factory.clone()
        };
        let model_id = payload
            .provider_id
            .as_deref()
            .map(std::borrow::ToOwned::to_owned)
            .unwrap_or_default();
        let bus = self.deps.services.bus.clone();
        let sid = session_id.clone();

        let handle = tokio::spawn(async move {
            let service: Box<dyn LlmService> = match factory.create() {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(err = ?e, "failed to create LLM service");
                    bus.publish(PushChatEntry {
                        session_id: sid.clone(),
                        entry: ChatEntry::error(format!("LLM service creation failed: {e:?}")),
                    })
                    .await;
                    bus.publish(StreamCompleted {
                        session_id: sid,
                        reason: StreamCompletedReason::Error,
                        assistant_content: None,
                        tool_calls: None,
                        cost: None,
                        provider_completion_tokens: None,
                        thinking_content: None,
                    })
                    .await;
                    return;
                }
            };

            let service = RetryingLlmService::new(
                service,
                retry_config,
                Box::new(PushEntryOnRetry::new(bus.clone(), sid.clone())),
            );

            let stream = match service.chat_stream_with_tools(messages, tools).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(err = ?e, "failed to start LLM stream");
                    bus.publish(PushChatEntry {
                        session_id: sid.clone(),
                        entry: ChatEntry::error(format!("LLM stream error: {e:?}")),
                    })
                    .await;
                    bus.publish(StreamCompleted {
                        session_id: sid,
                        reason: StreamCompletedReason::Error,
                        assistant_content: None,
                        tool_calls: None,
                        cost: None,
                        provider_completion_tokens: None,
                        thinking_content: None,
                    })
                    .await;
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
                                token_preview = %token.get(..token.len().min(50)).unwrap_or_default(),
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
                                bus.publish(StreamToken {
                                    session_id: sid.clone(),
                                    index: token_index,
                                    token: parsed.reasoning_text,
                                    is_thinking: true,
                                })
                                .await;
                                token_index += 1;
                            }
                            if !parsed.normal_text.is_empty() {
                                bus.publish(StreamToken {
                                    session_id: sid.clone(),
                                    index: token_index,
                                    token: parsed.normal_text,
                                    is_thinking: false,
                                })
                                .await;
                                token_index += 1;
                            }
                        }
                        StreamEvent::Reasoning(token) => {
                            tracing::info!(
                                session_id = ?sid,
                                token_len = token.len(),
                                token_preview = %token.get(..token.len().min(50)).unwrap_or_default(),
                                "LLM ACTOR StreamEvent::Reasoning"
                            );
                            accumulated_thinking.push_str(&token);
                            bus.publish(StreamToken {
                                session_id: sid.clone(),
                                index: token_index,
                                token,
                                is_thinking: true,
                            })
                            .await;
                            token_index += 1;
                        }
                        StreamEvent::ToolUseStart {
                            index,
                            id,
                            name,
                        } => {
                            bus.publish(ToolUseStarted {
                                session_id: sid.clone(),
                                index,
                                id,
                                name,
                            })
                            .await;
                        }
                        StreamEvent::ToolUseInputDelta {
                            index,
                            partial_json,
                        } => {
                            bus.publish(ToolCallStreaming {
                                session_id: sid.clone(),
                                index,
                                partial_json,
                            })
                            .await;
                        }
                        StreamEvent::ToolUseComplete {
                            tool_call, ..
                        } => {
                            accumulated_tool_calls.push(tool_call.clone());
                            bus.publish(ToolCallReceived {
                                session_id: sid.clone(),
                                tool_call,
                            })
                            .await;
                        }
                        StreamEvent::Done {
                            stop_reason,
                            usage,
                        } => {
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
                                bus.publish(ExecuteToolBatch {
                                    session_id: sid.clone(),
                                    tool_calls: accumulated_tool_calls.clone(),
                                })
                                .await;

                                // Emit StreamCompleted with ToolUse reason so the session
                                // actor can finalize output tokens. The continuation is
                                // handled by the session actor via context assembly when
                                // ToolBatchCompleted arrives.
                                bus.publish(StreamCompleted {
                                    session_id: sid.clone(),
                                    reason: StreamCompletedReason::ToolUse,
                                    assistant_content: Some(accumulated_text.clone()),
                                    tool_calls: Some(accumulated_tool_calls.clone()),
                                    cost,
                                    provider_completion_tokens,
                                    thinking_content,
                                })
                                .await;
                            } else {
                                // Normal end_turn - emit StreamCompleted.
                                bus.publish(StreamCompleted {
                                    session_id: sid.clone(),
                                    reason: StreamCompletedReason::Finished,
                                    assistant_content: Some(accumulated_text.clone()),
                                    tool_calls: None,
                                    cost,
                                    provider_completion_tokens,
                                    thinking_content,
                                })
                                .await;
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
                            bus.publish(PushChatEntry {
                                session_id: sid.clone(),
                                entry: ChatEntry::error(format!(
                                    "LLM error ({error_type}): {message}"
                                )),
                            })
                            .await;
                            bus.publish(StreamCompleted {
                                session_id: sid.clone(),
                                reason: StreamCompletedReason::Error,
                                assistant_content: None,
                                tool_calls: None,
                                cost: None,
                                provider_completion_tokens: None,
                                thinking_content: None,
                            })
                            .await;
                            stream_ended_normally = true;
                            break;
                        }
                    },
                    Err(e) => {
                        tracing::error!(err = ?e, "LLM stream error");
                        bus.publish(PushChatEntry {
                            session_id: sid.clone(),
                            entry: ChatEntry::error(format!("LLM stream error: {e:?}")),
                        })
                        .await;
                        bus.publish(StreamCompleted {
                            session_id: sid.clone(),
                            reason: StreamCompletedReason::Error,
                            assistant_content: None,
                            tool_calls: None,
                            cost: None,
                            provider_completion_tokens: None,
                            thinking_content: None,
                        })
                        .await;
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
                bus.publish(PushChatEntry {
                    session_id: sid.clone(),
                    entry: ChatEntry::error(
                        "LLM stream ended unexpectedly. The connection may have been interrupted."
                            .to_owned(),
                    ),
                })
                .await;
                bus.publish(StreamCompleted {
                    session_id: sid.clone(),
                    reason: StreamCompletedReason::Error,
                    assistant_content: None,
                    tool_calls: None,
                    cost: None,
                    provider_completion_tokens: None,
                    thinking_content: None,
                })
                .await;
            }
        });

        // Update session state.
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.begin_streaming();
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
    async fn cancel_stream(&mut self, session_id: &SessionId) {
        // If there's an active session, cancel any pending tool batches.
        if self.sessions.contains_key(session_id) {
            self.publish(CancelToolBatch {
                session_id: session_id.clone(),
            })
            .await;
        }

        if let Some(handle) = self.tasks.remove(session_id) {
            handle.abort();
        }
        let had_session = self.sessions.remove(session_id).is_some();
        // Only emit StreamCompleted if there was actually an active session
        // to cancel. Avoids pushing a spurious "Cancelled" error entry when
        // the user presses ESC with nothing streaming.
        if had_session {
            self.publish(StreamCompleted {
                session_id: session_id.clone(),
                reason: StreamCompletedReason::Canceled,
                assistant_content: None,
                tool_calls: None,
                cost: None,
                provider_completion_tokens: None,
                thinking_content: None,
            })
            .await;
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
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::session::SessionState;
    use super::*;

    use crate::common::bus::test_harness::{TestHarness, await_recorded};
    use crate::common::bus::BusMessage;
    use crate::feat::provider::protocol::event::StreamToken;
    use jinn_provider::FakeLlmServiceFactory;

    impl BusMessage for CancelStream {}
    impl BusMessage for StreamCompleted {}
    impl BusMessage for StreamToken {}
    async fn test_llm_actor() -> LlmActor {
        let factory = LlmServiceFactoryService::new(Arc::new(FakeLlmServiceFactory::new(vec![])));
        LlmActor {
            factory,
            deps: crate::common::actor_deps::ActorDeps {
                services: crate::common::services::Services::new_fake().await,
            },
            _state: State::new(crate::common::app_state::AppState::default()),
            tasks: HashMap::new(),
            sessions: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn handle_stream_completed_error_reason_removes_session() {
        // Given an LLM actor with a streaming session.
        let mut actor = test_llm_actor().await;
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

    #[tokio::test]
    async fn handle_stream_completed_finished_reason_removes_session() {
        // Given an LLM actor with a streaming session.
        let mut actor = test_llm_actor().await;
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

    #[tokio::test]
    async fn handle_stream_completed_tool_use_keeps_session() {
        // Given an LLM actor with a streaming session.
        let mut actor = test_llm_actor().await;
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

    #[tokio::test]
    async fn handle_stream_completed_unknown_session_is_noop() {
        // Given an LLM actor with NO sessions.
        let mut actor = test_llm_actor().await;
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
        let mut actor = test_llm_actor().await;
        let session_id = SessionId::new();
        actor
            .sessions
            .insert(session_id.clone(), SessionData::new());
        // Insert a dummy task that will be aborted.
        let handle = tokio::spawn(async { std::future::pending::<()>().await });
        actor.tasks.insert(session_id.clone(), handle);

        // When cancelling the stream.
        actor.cancel_stream(&session_id).await;

        // Then the session and task are removed.
        assert!(!actor.sessions.contains_key(&session_id));
        assert!(!actor.tasks.contains_key(&session_id));
    }

    #[tokio::test]
    async fn cancel_stream_without_session_emits_nothing() {
        // Given a test harness with the LLM actor and a recorder.
        let harness = TestHarness::new().await;
        let factory = LlmServiceFactoryService::new(Arc::new(FakeLlmServiceFactory::new(vec![])));
        let _actor = harness
            .spawn_actor::<LlmActor>(LlmActorDeps {
                factory,
                deps: harness.actor_deps().await,
                state: State::new(crate::common::app_state::AppState::default()),
            })
            .await;
        let recorder = harness.spawn_recorder::<StreamCompleted>().await;

        // When cancelling a stream for a session that doesn't exist.
        harness
            .publish(CancelStream {
                session_id: SessionId::new(),
            })
            .await;

        // Then no StreamCompleted event is emitted.
        let recorded = await_recorded(&recorder, 0, std::time::Duration::from_millis(100)).await;
        assert!(
            recorded.is_empty(),
            "should not emit StreamCompleted for non-existent session"
        );
    }

    #[tokio::test]
    async fn cancel_all_aborts_all_tasks() {
        // Given an LLM actor with multiple spawned tasks.
        let mut actor = test_llm_actor().await;
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
            assert!(
                handle.is_finished(),
                "task should be aborted after cancel_all"
            );
        }
    }

    #[tokio::test]
    async fn on_stop_cancels_all_tasks() {
        let mut actor = test_llm_actor().await;
        let sid = SessionId::new();
        let handle = tokio::spawn(async { std::future::pending::<()>().await });
        actor.tasks.insert(sid, handle);

        // When on_stop is called.
        actor.cancel_all();

        // Then the task is aborted.
        tokio::task::yield_now().await;
        for handle in actor.tasks.values() {
            assert!(
                handle.is_finished(),
                "task should be aborted after on_stop"
            );
        }
    }

    #[tokio::test]
    async fn start_stream_emits_stream_completed_with_tokens() {
        // Given a test harness with an LLM actor.
        let harness = TestHarness::new().await;
        let factory = LlmServiceFactoryService::new(Arc::new(FakeLlmServiceFactory::new(vec![
            "Hello".to_owned(),
            " World".to_owned(),
        ])));
        let _actor = harness
            .spawn_actor::<LlmActor>(LlmActorDeps {
                factory,
                deps: harness.actor_deps().await,
                state: State::new(crate::common::app_state::AppState::default()),
            })
            .await;

        let recorder_tokens = harness.spawn_recorder::<StreamToken>().await;
        let recorder_completed = harness.spawn_recorder::<StreamCompleted>().await;

        let session_id = SessionId::new();
        let payload = SendToLlmProvider {
            session_id: session_id.clone(),
            messages: vec![],
            tool_definitions: vec![],
            provider_id: None,
            estimated_tokens: 0,
        };

        // When starting a stream.
        harness.publish(payload).await;

        // Then StreamCompleted was emitted.
        let completed = await_recorded(&recorder_completed, 1, std::time::Duration::from_secs(5))
            .await;
        let finished = completed
            .iter()
            .find(|sc| sc.reason == StreamCompletedReason::Finished);
        assert!(finished.is_some(), "should emit StreamCompleted(Finished)");
        let finished = finished.unwrap();
        assert_eq!(finished.assistant_content.as_deref(), Some("Hello World"));

        // And StreamToken events were emitted with sequential indices.
        let token_events = await_recorded(&recorder_tokens, 2, std::time::Duration::from_secs(2))
            .await;
        assert_eq!(token_events.len(), 2, "should have 2 token events");
        assert_eq!(token_events[0].index, 0, "first token index should be 0");
        assert_eq!(token_events[1].index, 1, "second token index should be 1");
    }

    #[tokio::test]
    async fn start_stream_aborts_existing_stream_for_same_session() {
        // Given an LLM actor with an existing stream for a session.
        let mut actor = test_llm_actor().await;

        let session_id = SessionId::new();
        let payload = SendToLlmProvider {
            session_id: session_id.clone(),
            messages: vec![],
            tool_definitions: vec![],
            provider_id: None,
            estimated_tokens: 0,
        };

        // Start first stream and save the handle.
        actor.start_stream(payload.clone());
        let first_handle = actor.tasks.remove(&session_id);
        assert!(first_handle.is_some());
        // Re-insert for the second start_stream to find and abort.
        actor
            .tasks
            .insert(session_id.clone(), first_handle.unwrap());

        // When starting a second stream for the same session.
        actor.start_stream(payload);

        // Then a new task exists (the old one was aborted internally).
        assert!(
            actor.tasks.contains_key(&session_id),
            "second stream should create a new task"
        );
    }

    #[tokio::test]
    async fn start_stream_sets_session_to_streaming() {
        // Given an LLM actor.
        let mut actor = test_llm_actor().await;

        let session_id = SessionId::new();
        let payload = SendToLlmProvider {
            session_id: session_id.clone(),
            messages: vec![],
            tool_definitions: vec![],
            provider_id: None,
            estimated_tokens: 0,
        };

        // When starting a stream.
        actor.start_stream(payload);

        // Then the session state is Streaming.
        let session_data = actor.sessions.get(&session_id);
        assert!(session_data.is_some(), "session should be tracked");
        assert_eq!(
            *session_data.unwrap().state(),
            SessionState::Streaming,
            "session should be in Streaming state"
        );
    }

    #[tokio::test]
    async fn handle_send_to_llm_via_bus() {
        // Given a test harness with an LLM actor.
        let harness = TestHarness::new().await;
        let factory = LlmServiceFactoryService::new(Arc::new(FakeLlmServiceFactory::new(vec![
            "response".to_owned(),
        ])));
        let _actor = harness
            .spawn_actor::<LlmActor>(LlmActorDeps {
                factory,
                deps: harness.actor_deps().await,
                state: State::new(crate::common::app_state::AppState::default()),
            })
            .await;
        let recorder = harness.spawn_recorder::<StreamCompleted>().await;

        let session_id = SessionId::new();
        let payload = SendToLlmProvider {
            session_id: session_id.clone(),
            messages: vec![],
            tool_definitions: vec![],
            provider_id: None,
            estimated_tokens: 0,
        };

        // When sending via bus.
        harness.publish(payload).await;

        // Then the stream completes.
        let completed = await_recorded(&recorder, 1, std::time::Duration::from_secs(5)).await;
        let found = completed
            .iter()
            .any(|sc| sc.reason == StreamCompletedReason::Finished);
        assert!(found, "should emit StreamCompleted(Finished)");
    }

    #[tokio::test]
    async fn handle_cancel_stream_with_no_active_stream_is_noop() {
        // Given a test harness with an LLM actor.
        let harness = TestHarness::new().await;
        let factory = LlmServiceFactoryService::new(Arc::new(FakeLlmServiceFactory::new(vec![
            "response".to_owned(),
        ])));
        let _actor = harness
            .spawn_actor::<LlmActor>(LlmActorDeps {
                factory,
                deps: harness.actor_deps().await,
                state: State::new(crate::common::app_state::AppState::default()),
            })
            .await;

        let recorder = harness.spawn_recorder::<StreamCompleted>().await;

        // When sending CancelStream with no active stream.
        let session_id = SessionId::new();
        harness
            .publish(CancelStream {
                session_id: session_id.clone(),
            })
            .await;

        // Then no StreamCompleted is emitted (nothing to cancel).
        let completed = await_recorded(&recorder, 1, std::time::Duration::from_millis(100)).await;
        assert!(completed.is_empty(), "CancelStream with no active stream should be a no-op");
    }
}
