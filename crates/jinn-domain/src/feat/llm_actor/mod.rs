//! LLM streaming actor.
//!
//! Subscribes to [`SendToLlmProvider`] and [`CancelStream`] commands, and
//! [`StreamCompleted`] events. On send, creates an
//! LLM service via the factory and streams tokens and tool call events back
//! as bus commands. When the LLM requests tool use, emits [`ExecuteToolBatch`]
//! - the session actor handles the continuation via context assembly.

use serde::{Deserialize, Serialize};

/// Default retry configuration values.
const DEFAULT_RETRY_MAX_RETRIES: u32 = 5;
const DEFAULT_RETRY_BASE_DELAY_SECS: u64 = 2;
const DEFAULT_RETRY_MAX_DELAY_SECS: u64 = 60;

/// Retry configuration for LLM provider requests.
///
/// Serialized as `[request_retry]` in `jinn.toml`.
/// Controls exponential backoff behavior for transient errors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestRetryConfig {
    /// Maximum number of retry attempts. Default: 5.
    #[serde(default = "default_retry_max_retries")]
    pub max_retries: u32,
    /// Base delay in seconds for exponential backoff. Default: 2.
    #[serde(default = "default_retry_base_delay_secs")]
    pub base_delay_secs: u64,
    /// Maximum delay cap in seconds. Default: 60.
    /// Overridden by provider-supplied Retry-After / error body hints.
    #[serde(default = "default_retry_max_delay_secs")]
    pub max_delay_secs: u64,
}

fn default_retry_max_retries() -> u32 {
    DEFAULT_RETRY_MAX_RETRIES
}
fn default_retry_base_delay_secs() -> u64 {
    DEFAULT_RETRY_BASE_DELAY_SECS
}
fn default_retry_max_delay_secs() -> u64 {
    DEFAULT_RETRY_MAX_DELAY_SECS
}

impl Default for RequestRetryConfig {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_RETRY_MAX_RETRIES,
            base_delay_secs: DEFAULT_RETRY_BASE_DELAY_SECS,
            max_delay_secs: DEFAULT_RETRY_MAX_DELAY_SECS,
        }
    }
}

impl RequestRetryConfig {
    /// Convert to the provider-crate [`jinn_provider::RetryConfig`].
    #[must_use]
    pub fn to_retry_config(&self) -> jinn_provider::RetryConfig {
        jinn_provider::RetryConfig {
            max_retries: self.max_retries,
            base_delay: std::time::Duration::from_secs(self.base_delay_secs),
            max_delay: std::time::Duration::from_secs(self.max_delay_secs),
        }
    }
}

mod session;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::services::bus_service::BusService;
use crate::common::state::State;
use crate::feat::chat_input::protocol::command::PushChatEntry;
use crate::feat::provider::protocol::command::{
    CancelStream, ResetStreamForRetry, SendToLlmProvider,
};
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
use jiff::Timestamp;

use jinn_provider::{
    LlmMessage, LlmService, LlmServiceError, OnRetry, RetryingLlmService, ToolDefinition,
};
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

/// Emits a [`PushChatEntry`] error and [`StreamCompleted`] error event.
///
/// Used at every point where an LLM operation fails and the session needs
/// to be notified of the error terminal state.
async fn emit_stream_error(
    bus: &BusService,
    session_id: &SessionId,
    message: String,
    dispatched_at: Timestamp,
) {
    bus.publish(PushChatEntry {
        session_id: session_id.clone(),
        entry: ChatEntry::error(message),
    })
    .await;
    bus.publish(StreamCompleted {
        model_used: None,
        session_id: session_id.clone(),
        reason: StreamCompletedReason::Error,
        assistant_content: None,
        tool_calls: None,
        cost: None,
        provider_completion_tokens: None,
        thinking_content: None,
        dispatched_at,
    })
    .await;
}

/// Exponential backoff with full jitter for mid-stream stall retries.
///
/// Mirrors `RetryingLlmService::compute_delay` but without the
/// retryable-error / Retry-After-hint logic — a stall is always retryable
/// and has no provider hint.
fn compute_stall_backoff(config: &RequestRetryConfig, attempt: u32) -> Duration {
    let base_secs = config.base_delay_secs as f64;
    let exponential = base_secs * 2_f64.powi(i32::try_from(attempt).unwrap_or(i32::MAX));
    let capped = exponential.min(config.max_delay_secs as f64);
    if capped <= 0.0 {
        return Duration::ZERO;
    }
    let final_delay = rand::random_range(0.0..capped);
    Duration::from_secs_f64(final_delay)
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

impl BusPublish for LlmActor {
    fn bus(&self) -> &BusService {
        &self.deps.services.bus
    }
}

/// Dependencies for [`LlmActor`].
#[derive(Clone)]
pub struct LlmActorDeps {
    /// Factory for creating LLM service instances.
    pub factory: LlmServiceFactoryService,
    /// Shared dependencies (services, bus).
    pub deps: ActorDeps,
    /// Shared application state (for reading tool definitions).
    pub state: State,
}

impl kameo::Actor for LlmActor {
    type Args = LlmActorDeps;
    type Error = std::convert::Infallible;

    async fn on_start(
        args: Self::Args,
        actor_ref: kameo::actor::ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        let bus = &args.deps.services.bus;
        bus.subscribe::<SendToLlmProvider, _>(&actor_ref).await;
        bus.subscribe::<CancelStream, _>(&actor_ref).await;
        bus.subscribe::<StreamCompleted, _>(&actor_ref).await;

        Ok(Self {
            factory: args.factory,
            deps: args.deps,
            _state: args.state,
            tasks: HashMap::new(),
            sessions: HashMap::new(),
        })
    }
}

impl kameo::message::Message<SendToLlmProvider> for LlmActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: SendToLlmProvider,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.start_stream(&msg).await;
    }
}

impl kameo::message::Message<CancelStream> for LlmActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: CancelStream,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.cancel_stream(&msg.session_id).await;
    }
}

impl kameo::message::Message<StreamCompleted> for LlmActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: StreamCompleted,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_stream_completed(&msg);
    }
}

/// Outcome of consuming an LLM stream.
///
/// `process_stream_events` distinguishes a normal termination from an
/// idle stall so the caller (the stall-retry loop in `start_stream`) can
/// decide whether to retry.
enum StreamOutcome {
    /// The stream terminated normally via a `Done`/`Error` event or stream end.
    /// `StreamCompleted` has already been published on the bus.
    Completed,
    /// No event arrived for longer than the idle timeout. `StreamCompleted` has
    /// NOT been published — the caller owns the retry decision.
    Stalled,
}

/// Processes events from an LLM stream, emitting token/tool events via the sink.
///
/// Returns [`StreamOutcome::Completed`] if the stream ended with a terminal event
/// (Done or Error), or [`StreamOutcome::Stalled`] if no event arrived for
/// `idle_timeout` (the caller then decides whether to retry).
async fn process_stream_events(
    mut stream: jinn_provider::ToolStream,
    bus: &BusService,
    sid: &SessionId,
    model_id: &str,
    dispatched_at: jiff::Timestamp,
    idle_timeout: std::time::Duration,
) -> StreamOutcome {
    let mut accum = StreamAccumulator::new(model_id);

    loop {
        // Reset the idle timer on every iteration: each event — token, reasoning,
        // or tool-call delta — resets it. Only a true provider/connection-level
        // stall (zero events) trips the timeout. Long reasoning blocks that keep
        // producing tokens are never falsely tripped.
        match tokio::time::timeout(idle_timeout, stream.next()).await {
            Ok(Some(Ok(event))) => match event {
                StreamEvent::Text(token) => {
                    handle_text_event(bus, sid, dispatched_at, &mut accum, token).await;
                }
                StreamEvent::Reasoning(token) => {
                    handle_reasoning_event(bus, sid, dispatched_at, &mut accum, token).await;
                }
                StreamEvent::ToolUseStart { index, id, name } => {
                    bus.publish(ToolUseStarted {
                        session_id: sid.clone(),
                        index,
                        id,
                        name,
                        dispatched_at,
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
                StreamEvent::ToolUseComplete { tool_call, .. } => {
                    accum.tool_calls.push(tool_call.clone());
                    bus.publish(ToolCallReceived {
                        session_id: sid.clone(),
                        tool_call,
                        dispatched_at,
                    })
                    .await;
                }
                StreamEvent::Citations(citations) => {
                    accum.citations.extend(citations);
                }
                StreamEvent::Done { stop_reason, usage } => {
                    handle_done_event(bus, sid, &mut accum, stop_reason, usage, dispatched_at)
                        .await;
                    return StreamOutcome::Completed;
                }
                StreamEvent::Error { message, .. } => {
                    tracing::error!(
                        session_id = ?sid,
                        error = %message,
                        "LLM stream error event"
                    );
                    emit_stream_error(
                        bus,
                        sid,
                        format!("LLM stream error: {message}"),
                        dispatched_at,
                    )
                    .await;
                    return StreamOutcome::Completed;
                }
            },
            Ok(Some(Err(e))) => {
                emit_stream_error(bus, sid, format!("LLM stream error: {e:?}"), dispatched_at)
                    .await;
                return StreamOutcome::Completed;
            }
            Ok(None) => {
                // Stream ended without a terminal event (Done/Error).
                tracing::error!(
                    session_id = ?sid,
                    "LLM stream ended without a terminal event (Done/Error)"
                );
                emit_stream_error(
                    bus,
                    sid,
                    "LLM stream ended unexpectedly. The connection may have been interrupted."
                        .to_owned(),
                    dispatched_at,
                )
                .await;
                return StreamOutcome::Completed;
            }
            Err(_elapsed) => {
                // Idle timeout fired: no event for `idle_timeout`. Hand control back
                // to the caller's stall-retry loop. Do NOT publish `StreamCompleted`.
                tracing::warn!(
                    session_id = ?sid,
                    idle_timeout_secs = idle_timeout.as_secs(),
                    "LLM stream stalled — no event within idle timeout"
                );
                return StreamOutcome::Stalled;
            }
        }
    }
}

/// Accumulates streamed text, reasoning, and tool calls during an LLM stream.
struct StreamAccumulator {
    text: String,
    thinking: String,
    tool_calls: Vec<ToolCall>,
    citations: Vec<jinn_provider::UrlCitation>,
    token_index: usize,
    model_id: String,
    parser: Box<dyn reasoning_parser::ReasoningParser>,
}

impl StreamAccumulator {
    fn new(model_id: &str) -> Self {
        let parser = reasoning_parser::ParserFactory::new().create(model_id);
        Self {
            text: String::new(),
            thinking: String::new(),
            tool_calls: Vec::new(),
            citations: Vec::new(),
            token_index: 0,
            model_id: model_id.to_owned(),
            parser,
        }
    }

    /// Publishes a reasoning fragment to the bus and advances the token index.
    async fn publish_thinking(
        &mut self,
        bus: &BusService,
        sid: &SessionId,
        token: String,
        dispatched_at: jiff::Timestamp,
    ) {
        self.thinking.push_str(&token);
        bus.publish(StreamToken {
            session_id: sid.clone(),
            index: self.token_index,
            token,
            is_thinking: true,
            dispatched_at,
        })
        .await;
        self.token_index += 1;
    }

    /// Publishes a normal-text fragment to the bus and advances the token index.
    async fn publish_text(
        &mut self,
        bus: &BusService,
        sid: &SessionId,
        token: String,
        dispatched_at: jiff::Timestamp,
    ) {
        bus.publish(StreamToken {
            session_id: sid.clone(),
            index: self.token_index,
            token,
            is_thinking: false,
            dispatched_at,
        })
        .await;
        self.token_index += 1;
    }

    /// Takes the accumulated thinking text, returning `None` if empty.
    fn take_thinking(&mut self) -> Option<String> {
        if self.thinking.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.thinking))
        }
    }
}

/// Builds the terminal `StreamCompleted` payload shared by ToolUse and Finished.
async fn publish_stream_completed(
    bus: &BusService,
    sid: &SessionId,
    accum: &mut StreamAccumulator,
    reason: StreamCompletedReason,
    tool_calls: Option<Vec<ToolCall>>,
    cost: Option<f64>,
    provider_completion_tokens: Option<u64>,
    dispatched_at: jiff::Timestamp,
) {
    bus.publish(StreamCompleted {
        model_used: Some(accum.model_id.clone()),
        session_id: sid.clone(),
        reason,
        thinking_content: accum.take_thinking(),
        assistant_content: Some(std::mem::take(&mut accum.text)),
        tool_calls,
        cost,
        provider_completion_tokens,
        dispatched_at,
    })
    .await;
}

/// Handles a `StreamEvent::Text`: parses reasoning/normal split, publishes both.
async fn handle_text_event(
    bus: &BusService,
    sid: &SessionId,
    dispatched_at: jiff::Timestamp,
    accum: &mut StreamAccumulator,
    token: String,
) {
    tracing::info!(
        session_id = ?sid,
        token_len = token.len(),
        token_preview = %token.get(..token.len().min(50)).unwrap_or_default(),
        "LLM ACTOR StreamEvent::Text"
    );
    accum.text.push_str(&token);
    let parsed = match accum.parser.parse_reasoning_streaming_incremental(&token) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(err = ?e, "reasoning parser error, treating as normal text");
            reasoning_parser::ParserResult::normal(token.clone())
        }
    };
    if !parsed.reasoning_text.is_empty() {
        accum
            .publish_thinking(bus, sid, parsed.reasoning_text, dispatched_at)
            .await;
    }
    if !parsed.normal_text.is_empty() {
        accum
            .publish_text(bus, sid, parsed.normal_text, dispatched_at)
            .await;
    }
}

/// Handles a `StreamEvent::Reasoning`: accumulates and publishes thinking text.
async fn handle_reasoning_event(
    bus: &BusService,
    sid: &SessionId,
    dispatched_at: jiff::Timestamp,
    accum: &mut StreamAccumulator,
    token: String,
) {
    tracing::info!(
        session_id = ?sid,
        token_len = token.len(),
        token_preview = %token.get(..token.len().min(50)).unwrap_or_default(),
        "LLM ACTOR StreamEvent::Reasoning"
    );
    accum.publish_thinking(bus, sid, token, dispatched_at).await;
}

/// Handles `StreamEvent::Done`: routes tool-use vs finished and publishes
/// `ExecuteToolBatch` + `StreamCompleted`.
async fn handle_done_event(
    bus: &BusService,
    sid: &SessionId,
    accum: &mut StreamAccumulator,
    stop_reason: StopReason,
    usage: Option<jinn_provider::StreamUsage>,
    dispatched_at: jiff::Timestamp,
) {
    tracing::trace!(
        session_id = ?sid,
        stop_reason = %stop_reason,
        tool_call_count = accum.tool_calls.len(),
        "stream Done"
    );
    if !accum.citations.is_empty() {
        let citations = std::mem::take(&mut accum.citations);
        bus.publish(crate::feat::session::protocol::citations_received::CitationsReceived {
            session_id: sid.clone(),
            citations,
        })
        .await;
    }
    let cost = usage.as_ref().and_then(|u| u.cost);
    let provider_completion_tokens = usage.as_ref().and_then(|u| u.completion_tokens);
    if stop_reason == StopReason::ToolUse {
        let tool_calls = std::mem::take(&mut accum.tool_calls);
        bus.publish(ExecuteToolBatch {
            session_id: sid.clone(),
            tool_calls: tool_calls.clone(),
            dispatched_at,
        })
        .await;
        publish_stream_completed(
            bus,
            sid,
            accum,
            StreamCompletedReason::ToolUse,
            Some(tool_calls),
            cost,
            provider_completion_tokens,
            dispatched_at,
        )
        .await;
    } else {
        publish_stream_completed(
            bus,
            sid,
            accum,
            StreamCompletedReason::Finished,
            None,
            cost,
            provider_completion_tokens,
            dispatched_at,
        )
        .await;
    }
}

impl LlmActor {
    /// Dispatches incoming commands to the appropriate handler.
    /// Starts an LLM streaming response for a session, aborting any existing stream.
    async fn start_stream(&mut self, payload: &SendToLlmProvider) {
        let prefs = self.deps.services.user_preferences_storage.read();
        let retry_config = prefs.request_retry.clone();
        let idle_timeout_secs = prefs.stream_idle_timeout_secs;

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

        // Track the session and store the resolved model.
        let model_used = payload.model_used.clone();
        self.sessions.insert(session_id.clone(), SessionData::new());
        if let Some(data) = self.sessions.get_mut(&session_id) {
            data.set_model_used(model_used);
        }

        // Resolve the factory: per-request if provider_id is set, global fallback otherwise.
        let factory = match self.resolve_factory(payload) {
            Ok(f) => f,
            Err(msg) => {
                emit_stream_error(
                    &self.deps.services.bus,
                    &session_id,
                    msg,
                    payload.dispatched_at,
                )
                .await;
                return;
            }
        };
        let model_id = payload
            .provider_id
            .as_deref()
            .map(std::borrow::ToOwned::to_owned)
            .unwrap_or_default();
        let bus = self.deps.services.bus.clone();
        let sid = session_id.clone();
        let dispatched_at = payload.dispatched_at;

        let handle = tokio::spawn(run_stream_with_stall_retry(
            factory,
            bus,
            sid,
            model_id,
            messages,
            tools,
            dispatched_at,
            retry_config,
            idle_timeout_secs,
        ));

        // Update session state.
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.begin_streaming(dispatched_at);
        }

        self.tasks.insert(session_id, handle);
    }

    /// Resolves the LLM factory for a request: per-request when `provider_id` is set,
    /// the global factory otherwise. Returns an error message on failure.
    fn resolve_factory(
        &self,
        payload: &SendToLlmProvider,
    ) -> Result<LlmServiceFactoryService, String> {
        if let Some(pid) = payload.provider_id.clone() {
            let id = crate::feat::provider_infra::ProviderId::new(pid.clone());
            let api_keys = self.deps.services.api_keys.read();
            match self.deps.services.provider_registry.create_factory(
                &id,
                &api_keys,
                payload.reasoning_effort,
            ) {
                Ok(f) => {
                    tracing::debug!(provider_id = %pid, "created per-request LLM factory");
                    Ok(LlmServiceFactoryService::new(Arc::from(f)))
                }
                Err(e) => {
                    tracing::error!(err = ?e, provider_id = %pid, "failed to create per-request factory");
                    Err(format!("LLM factory creation failed for {pid}: {e:?}"))
                }
            }
        } else {
            Ok(self.factory.clone())
        }
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
        let dispatched_at = self
            .sessions
            .get(session_id)
            .and_then(session::SessionData::dispatched_at);
        let had_session = self.sessions.remove(session_id).is_some();
        // Only emit StreamCompleted if there was actually an active session
        // to cancel. Avoids pushing a spurious "Cancelled" error entry when
        // the user presses ESC with nothing streaming.
        if had_session {
            self.publish(StreamCompleted {
                model_used: None,
                session_id: session_id.clone(),
                reason: StreamCompletedReason::Canceled,
                assistant_content: None,
                tool_calls: None,
                cost: None,
                provider_completion_tokens: None,
                thinking_content: None,
                dispatched_at: dispatched_at.unwrap_or_else(Timestamp::now),
            })
            .await;
        }
    }
}

/// Drives the streaming conversation with stall detection and bounded auto-retry.
///
/// A stall (no stream events for `idle_timeout_secs`) is treated like a hard
/// server error: partial streaming entries are discarded, a system entry is pushed,
/// and the request is retried with backoff up to `request_retry.max_retries` times.
async fn run_stream_with_stall_retry(
    factory: LlmServiceFactoryService,
    bus: BusService,
    sid: SessionId,
    model_id: String,
    messages: Vec<LlmMessage>,
    tools: Vec<ToolDefinition>,
    dispatched_at: jiff::Timestamp,
    retry_config: RequestRetryConfig,
    idle_timeout_secs: u64,
) {
    let idle_timeout = Duration::from_secs(idle_timeout_secs);
    let max_stall_retries = retry_config.max_retries;
    let mut stall_attempt: u32 = 0;

    loop {
        let service = match build_streaming_service(&factory, &retry_config, &bus, &sid) {
            Ok(s) => s,
            Err(message) => {
                emit_stream_error(&bus, &sid, message, dispatched_at).await;
                return;
            }
        };

        let stream = match service
            .chat_stream_with_tools(messages.clone(), tools.clone())
            .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(err = ?e, "failed to start LLM stream");
                emit_stream_error(
                    &bus,
                    &sid,
                    format!("LLM stream error: {e:?}"),
                    dispatched_at,
                )
                .await;
                return;
            }
        };

        let outcome =
            process_stream_events(stream, &bus, &sid, &model_id, dispatched_at, idle_timeout).await;

        match outcome {
            StreamOutcome::Completed => return,
            StreamOutcome::Stalled => {
                stall_attempt += 1;
                if stall_attempt > max_stall_retries {
                    tracing::error!(
                        session_id = ?sid,
                        stall_attempt,
                        max_stall_retries,
                        "LLM stream stalled; stall-retry budget exhausted"
                    );
                    emit_stream_error(
                        &bus,
                        &sid,
                        format!("LLM stream stalled (no activity for {idle_timeout_secs}s)"),
                        dispatched_at,
                    )
                    .await;
                    return;
                }

                if handle_stall_retry(&bus, &sid, &retry_config, stall_attempt, max_stall_retries)
                    .await
                {
                    continue;
                }
                return;
            }
        }
    }
}

/// Constructs a fresh retrying service for one streaming attempt.
fn build_streaming_service(
    factory: &LlmServiceFactoryService,
    retry_config: &RequestRetryConfig,
    bus: &BusService,
    sid: &SessionId,
) -> Result<RetryingLlmService, String> {
    let service: Box<dyn LlmService> = factory.create().map_err(|e| {
        tracing::error!(err = ?e, "failed to create LLM service");
        format!("LLM service creation failed: {e:?}")
    })?;
    Ok(RetryingLlmService::new(
        service,
        retry_config.to_retry_config(),
        Box::new(PushEntryOnRetry::new(bus.clone(), sid.clone())),
    ))
}

/// Handles a stall by discarding partial output and scheduling a retry.
///
/// Returns `true` when the caller should retry, `false` if recovery is impossible
/// (should not happen given the caller checks the budget, but kept defensive).
async fn handle_stall_retry(
    bus: &BusService,
    sid: &SessionId,
    retry_config: &RequestRetryConfig,
    stall_attempt: u32,
    max_stall_retries: u32,
) -> bool {
    let delay = compute_stall_backoff(retry_config, stall_attempt.saturating_sub(1));
    tracing::warn!(
        session_id = ?sid,
        stall_attempt,
        max_stall_retries,
        backoff_secs = delay.as_secs(),
        "LLM stream stalled; resetting and retrying"
    );

    // Discard partial streaming entries so the retry starts clean.
    bus.publish(ResetStreamForRetry {
        session_id: sid.clone(),
    })
    .await;
    // Notify the user the turn is recovering.
    bus.publish(PushChatEntry {
        session_id: sid.clone(),
        entry: ChatEntry::system(format!(
            "LLM stream stalled, retrying in {}s (attempt {stall_attempt}/{max_stall_retries})",
            delay.as_secs()
        )),
    })
    .await;
    tokio::time::sleep(delay).await;
    true
}

#[cfg(test)]
mod test_fakes {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::items_after_statements,
        clippy::unused_async,
        reason = "test code"
    )]
    use super::*;
    use jinn_provider::{ChatStream, LlmServiceFactory, ToolStream};

    /// An LLM service whose stream never yields — used to test cancel paths
    /// while a stream is genuinely in flight.
    #[derive(Debug)]
    struct HangingLlmService;

    #[async_trait::async_trait]
    impl LlmService for HangingLlmService {
        fn name(&self) -> &'static str {
            "HangingLlm"
        }
        async fn chat_stream(
            &self,
            _messages: Vec<jinn_provider::LlmMessage>,
        ) -> Result<ChatStream, Report<LlmServiceError>> {
            Ok(Box::pin(futures::stream::pending()))
        }
        async fn chat_stream_with_tools(
            &self,
            _messages: Vec<jinn_provider::LlmMessage>,
            _tools: Vec<jinn_provider::ToolDefinition>,
        ) -> Result<ToolStream, Report<LlmServiceError>> {
            Ok(Box::pin(futures::stream::pending()))
        }
    }

    /// Factory that produces [`HangingLlmService`] instances.
    #[derive(Debug)]
    pub(super) struct HangingLlmFactory;

    impl LlmServiceFactory for HangingLlmFactory {
        fn create(&self) -> Result<Box<dyn LlmService>, Report<LlmServiceError>> {
            Ok(Box::new(HangingLlmService))
        }
        fn name(&self) -> &'static str {
            "HangingLlm"
        }
    }

    /// A factory whose stream creation fails immediately — used to verify the
    /// error path publishes a chat entry and stream completion.
    #[derive(Debug)]
    pub(super) struct ErroringLlmFactory;

    impl LlmServiceFactory for ErroringLlmFactory {
        fn create(&self) -> Result<Box<dyn LlmService>, Report<LlmServiceError>> {
            Ok(Box::new(ErroringLlmService))
        }
        fn name(&self) -> &'static str {
            "ErroringLlm"
        }
    }

    /// A service whose stream immediately yields an error.
    #[derive(Debug)]
    struct ErroringLlmService;

    #[async_trait::async_trait]
    impl LlmService for ErroringLlmService {
        fn name(&self) -> &'static str {
            "ErroringLlm"
        }
        async fn chat_stream(
            &self,
            _messages: Vec<jinn_provider::LlmMessage>,
        ) -> Result<ChatStream, Report<LlmServiceError>> {
            Err(Report::new(LlmServiceError::Provider))
        }
        async fn chat_stream_with_tools(
            &self,
            _messages: Vec<jinn_provider::LlmMessage>,
            _tools: Vec<jinn_provider::ToolDefinition>,
        ) -> Result<ToolStream, Report<LlmServiceError>> {
            Err(Report::new(LlmServiceError::Provider))
        }
    }
    /// A factory whose first `chat_stream_with_tools` call returns a
    /// stream that never yields (simulating a provider stall), and whose
    /// subsequent calls return a normal text+Done stream. Used to exercise
    /// the stall-retry loop in `start_stream`.
    #[derive(Debug)]
    pub(super) struct StallThenCompleteLlmFactory {
        call_count: Arc<std::sync::atomic::AtomicU32>,
        tokens: Vec<String>,
    }

    impl StallThenCompleteLlmFactory {
        pub(super) fn new(tokens: Vec<String>) -> Self {
            Self {
                call_count: Arc::new(std::sync::atomic::AtomicU32::new(0)),
                tokens,
            }
        }
    }

    impl LlmServiceFactory for StallThenCompleteLlmFactory {
        fn create(&self) -> Result<Box<dyn LlmService>, Report<LlmServiceError>> {
            Ok(Box::new(StallThenCompleteLlmService {
                call_count: self.call_count.clone(),
                tokens: self.tokens.clone(),
            }))
        }
        fn name(&self) -> &'static str {
            "StallThenComplete"
        }
    }

    #[derive(Debug)]
    struct StallThenCompleteLlmService {
        call_count: Arc<std::sync::atomic::AtomicU32>,
        tokens: Vec<String>,
    }

    #[async_trait::async_trait]
    impl LlmService for StallThenCompleteLlmService {
        fn name(&self) -> &'static str {
            "StallThenComplete"
        }
        async fn chat_stream(
            &self,
            _messages: Vec<jinn_provider::LlmMessage>,
        ) -> Result<ChatStream, Report<LlmServiceError>> {
            // Unused by the stall-retry path; emit tokens for safety.
            let tokens = self.tokens.clone();
            Ok(Box::pin(futures::stream::iter(tokens.into_iter().map(Ok))))
        }
        async fn chat_stream_with_tools(
            &self,
            _messages: Vec<jinn_provider::LlmMessage>,
            _tools: Vec<jinn_provider::ToolDefinition>,
        ) -> Result<ToolStream, Report<LlmServiceError>> {
            use std::sync::atomic::Ordering;
            let n = self.call_count.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                // First attempt: never yields — simulates a provider stall.
                return Ok(Box::pin(futures::stream::pending()));
            }
            // Subsequent attempts: emit tokens then Done(EndTurn).
            use jinn_provider::{StopReason, StreamEvent};
            let mut events: Vec<Result<StreamEvent, Report<LlmServiceError>>> = self
                .tokens
                .iter()
                .cloned()
                .map(StreamEvent::Text)
                .map(Ok)
                .collect();
            events.push(Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: None,
            }));
            Ok(Box::pin(futures::stream::iter(events)))
        }
    }

    /// A factory that simulates independent retry budgets: the inner service
    /// returns a sequence of `Retryable` connection errors (consumed by
    /// `RetryingLlmService`'s per-instance connection counter), then a stalled
    /// stream (consumed by the outer stall counter), then a normal completion.
    /// Used to prove the stall counter and connection-retry counter are
    /// independent budgets.
    #[derive(Debug)]
    pub(super) struct RetryableThenStallThenCompleteLlmFactory {
        call_count: Arc<std::sync::atomic::AtomicU32>,
        retryable_failures: u32,
        stall_calls: u32,
        tokens: Vec<String>,
    }

    impl RetryableThenStallThenCompleteLlmFactory {
        /// `retryable_failures` connection errors, then `stall_calls` stalled
        /// streams, then a normal completion.
        pub(super) fn new(retryable_failures: u32, stall_calls: u32, tokens: Vec<String>) -> Self {
            Self {
                call_count: Arc::new(std::sync::atomic::AtomicU32::new(0)),
                retryable_failures,
                stall_calls,
                tokens,
            }
        }
    }

    impl LlmServiceFactory for RetryableThenStallThenCompleteLlmFactory {
        fn create(&self) -> Result<Box<dyn LlmService>, Report<LlmServiceError>> {
            Ok(Box::new(RetryableThenStallThenCompleteLlmService {
                call_count: self.call_count.clone(),
                retryable_failures: self.retryable_failures,
                stall_calls: self.stall_calls,
                tokens: self.tokens.clone(),
            }))
        }
        fn name(&self) -> &'static str {
            "RetryableThenStallThenComplete"
        }
    }

    #[derive(Debug)]
    struct RetryableThenStallThenCompleteLlmService {
        call_count: Arc<std::sync::atomic::AtomicU32>,
        retryable_failures: u32,
        stall_calls: u32,
        tokens: Vec<String>,
    }

    #[async_trait::async_trait]
    impl LlmService for RetryableThenStallThenCompleteLlmService {
        fn name(&self) -> &'static str {
            "RetryableThenStallThenComplete"
        }
        async fn chat_stream(
            &self,
            _messages: Vec<jinn_provider::LlmMessage>,
        ) -> Result<ChatStream, Report<LlmServiceError>> {
            let tokens = self.tokens.clone();
            Ok(Box::pin(futures::stream::iter(tokens.into_iter().map(Ok))))
        }
        async fn chat_stream_with_tools(
            &self,
            _messages: Vec<jinn_provider::LlmMessage>,
            _tools: Vec<jinn_provider::ToolDefinition>,
        ) -> Result<ToolStream, Report<LlmServiceError>> {
            use std::sync::atomic::Ordering;
            let n = self.call_count.fetch_add(1, Ordering::SeqCst);
            if n < self.retryable_failures {
                // Connection-level retryable failure (consumed by
                // RetryingLlmService's per-instance counter).
                return Err(Report::new(LlmServiceError::Retryable));
            }
            let stall_end = self.retryable_failures + self.stall_calls;
            if n < stall_end {
                // Mid-stream stall (consumed by the outer stall counter).
                return Ok(Box::pin(futures::stream::pending()));
            }
            // Normal completion.
            use jinn_provider::{StopReason, StreamEvent};
            let mut events: Vec<Result<StreamEvent, Report<LlmServiceError>>> = self
                .tokens
                .iter()
                .cloned()
                .map(StreamEvent::Text)
                .map(Ok)
                .collect();
            events.push(Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: None,
            }));
            Ok(Box::pin(futures::stream::iter(events)))
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
        clippy::unused_async,
        reason = "test code"
    )]
    use super::session::SessionState;
    use super::test_fakes::{ErroringLlmFactory, HangingLlmFactory};
    use super::*;

    use crate::common::bus::test_harness::{TestHarness, await_recorded};
    use crate::feat::provider::protocol::event::StreamToken;
    use jinn_provider::FakeLlmServiceFactory;

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
            model_used: None,
            session_id,
            reason: StreamCompletedReason::Error,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
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
            model_used: None,
            session_id,
            reason: StreamCompletedReason::Finished,
            assistant_content: Some("hello".to_owned()),
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
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
            model_used: None,
            session_id: session_id.clone(),
            reason: StreamCompletedReason::ToolUse,
            assistant_content: Some("thinking...".to_owned()),
            tool_calls: Some(vec![]),
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
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
            model_used: None,
            session_id,
            reason: StreamCompletedReason::Error,
            assistant_content: None,
            tool_calls: None,
            cost: None,
            provider_completion_tokens: None,
            thinking_content: None,
            dispatched_at: jiff::Timestamp::now(),
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
        let messages = await_recorded(&recorder, 0, std::time::Duration::from_millis(100)).await;
        assert!(
            messages.is_empty(),
            "should not emit StreamCompleted for non-existent session"
        );
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
            model_used: None,
            reasoning_effort: None,
            session_id: session_id.clone(),
            messages: vec![],
            tool_definitions: vec![],
            provider_id: None,
            estimated_tokens: 0,
            dispatched_at: jiff::Timestamp::now(),
        };

        // When starting a stream.
        harness.publish(payload).await;

        // Then StreamCompleted was emitted.
        let completed =
            await_recorded(&recorder_completed, 1, std::time::Duration::from_secs(5)).await;
        let finished = completed
            .iter()
            .find(|sc| sc.reason == StreamCompletedReason::Finished);
        assert!(finished.is_some(), "should emit StreamCompleted(Finished)");
        let finished = finished.unwrap();
        assert_eq!(finished.assistant_content.as_deref(), Some("Hello World"));

        // And StreamToken events were emitted with sequential indices.
        let token_events =
            await_recorded(&recorder_tokens, 2, std::time::Duration::from_secs(2)).await;
        assert_eq!(token_events.len(), 2, "should have 2 token events");
        assert_eq!(token_events[0].index, 0, "first token index should be 0");
        assert_eq!(token_events[1].index, 1, "second token index should be 1");
    }

    #[tokio::test]
    async fn stalled_stream_is_retried_and_eventually_completes() {
        // Given a factory whose first call stalls and second completes.
        let harness = TestHarness::new().await;
        // Build deps once so the prefs we set reach the spawned actor.
        let deps = harness.actor_deps().await;
        {
            let mut prefs = deps.services.user_preferences_storage.read();
            prefs.stream_idle_timeout_secs = 1;
            prefs.request_retry.max_retries = 3;
            prefs.request_retry.base_delay_secs = 0;
            prefs.request_retry.max_delay_secs = 0;
            deps.services
                .user_preferences_storage
                .save(&prefs)
                .expect("save prefs");
        }
        let factory = LlmServiceFactoryService::new(Arc::new(
            super::test_fakes::StallThenCompleteLlmFactory::new(vec!["ok".to_owned()]),
        ));
        let _actor = harness
            .spawn_actor::<LlmActor>(LlmActorDeps {
                factory,
                deps,
                state: State::new(crate::common::app_state::AppState::default()),
            })
            .await;
        let recorder = harness.spawn_recorder::<StreamCompleted>().await;

        let session_id = SessionId::new();
        let payload = SendToLlmProvider {
            model_used: None,
            reasoning_effort: None,
            session_id: session_id.clone(),
            messages: vec![],
            tool_definitions: vec![],
            provider_id: None,
            estimated_tokens: 0,
            dispatched_at: jiff::Timestamp::now(),
        };

        // When the stream stalls and then retries.
        harness.publish(payload).await;

        // Then StreamCompleted(Finished) is eventually emitted despite the stall.
        let completed = await_recorded(&recorder, 1, std::time::Duration::from_secs(15)).await;
        let found = completed
            .iter()
            .any(|sc| sc.reason == StreamCompletedReason::Finished);
        assert!(
            found,
            "stalled stream should be retried and complete normally"
        );
    }

    #[tokio::test]
    async fn stalled_stream_gives_up_after_max_retries_and_errors() {
        // Given a factory that always stalls.
        let harness = TestHarness::new().await;
        let deps = harness.actor_deps().await;
        {
            let mut prefs = deps.services.user_preferences_storage.read();
            prefs.stream_idle_timeout_secs = 1;
            prefs.request_retry.max_retries = 1;
            prefs.request_retry.base_delay_secs = 0;
            prefs.request_retry.max_delay_secs = 0;
            deps.services
                .user_preferences_storage
                .save(&prefs)
                .expect("save prefs");
        }
        let factory = LlmServiceFactoryService::new(Arc::new(HangingLlmFactory));
        let _actor = harness
            .spawn_actor::<LlmActor>(LlmActorDeps {
                factory,
                deps,
                state: State::new(crate::common::app_state::AppState::default()),
            })
            .await;
        let recorder = harness.spawn_recorder::<StreamCompleted>().await;

        let session_id = SessionId::new();
        let payload = SendToLlmProvider {
            model_used: None,
            reasoning_effort: None,
            session_id: session_id.clone(),
            messages: vec![],
            tool_definitions: vec![],
            provider_id: None,
            estimated_tokens: 0,
            dispatched_at: jiff::Timestamp::now(),
        };

        // When the stream stalls and retries are exhausted.
        harness.publish(payload).await;

        // Then StreamCompleted(Error) is emitted (no infinite hang).
        let completed = await_recorded(&recorder, 1, std::time::Duration::from_secs(15)).await;
        let found = completed
            .iter()
            .any(|sc| sc.reason == StreamCompletedReason::Error);
        assert!(
            found,
            "perpetually stalled stream should emit StreamCompleted(Error) after retries exhaust"
        );
    }

    #[tokio::test]
    async fn stall_retry_publishes_system_chat_entry() {
        // Given a factory whose first call stalls and second completes.
        let harness = TestHarness::new().await;
        let deps = harness.actor_deps().await;
        {
            let mut prefs = deps.services.user_preferences_storage.read();
            prefs.stream_idle_timeout_secs = 1;
            prefs.request_retry.max_retries = 3;
            prefs.request_retry.base_delay_secs = 0;
            prefs.request_retry.max_delay_secs = 0;
            deps.services
                .user_preferences_storage
                .save(&prefs)
                .expect("save prefs");
        }
        let factory = LlmServiceFactoryService::new(Arc::new(
            super::test_fakes::StallThenCompleteLlmFactory::new(vec!["ok".to_owned()]),
        ));
        let _actor = harness
            .spawn_actor::<LlmActor>(LlmActorDeps {
                factory,
                deps,
                state: State::new(crate::common::app_state::AppState::default()),
            })
            .await;
        let entry_recorder = harness.spawn_recorder::<PushChatEntry>().await;
        let completed_recorder = harness.spawn_recorder::<StreamCompleted>().await;

        let session_id = SessionId::new();
        let payload = SendToLlmProvider {
            model_used: None,
            reasoning_effort: None,
            session_id: session_id.clone(),
            messages: vec![],
            tool_definitions: vec![],
            provider_id: None,
            estimated_tokens: 0,
            dispatched_at: jiff::Timestamp::now(),
        };

        // When the stream stalls and retries.
        harness.publish(payload).await;

        // Then the turn eventually completes.
        let completed =
            await_recorded(&completed_recorder, 1, std::time::Duration::from_secs(15)).await;
        assert!(
            completed
                .iter()
                .any(|sc| sc.reason == StreamCompletedReason::Finished),
            "stalled stream should complete after retry"
        );

        // And a System chat entry announcing the retry was published.
        let entries = await_recorded(&entry_recorder, 1, std::time::Duration::from_secs(15)).await;
        let found_retry_notice = entries.iter().any(|pce| {
            matches!(
                &pce.entry.kind,
                crate::protocol::ChatEntryKind::System(text) if text.contains("retrying"),
            )
        });
        assert!(
            found_retry_notice,
            "a System entry containing 'retrying' should be published on stall retry"
        );
    }

    #[tokio::test]
    async fn separate_stall_counter_does_not_share_budget_with_request_retries() {
        // Given a factory that first returns two Retryable connection errors
        // (consumed by RetryingLlmService's per-instance connection budget),
        // then one stalled stream (consumed by the outer stall budget), then a
        // normal completion. With max_retries=2, a SHARED budget would be
        // exhausted by the two connection failures and the stall would error
        // out instead of completing. A SEPARATE stall budget lets the turn
        // reach Finished.
        let harness = TestHarness::new().await;
        let deps = harness.actor_deps().await;
        {
            let mut prefs = deps.services.user_preferences_storage.read();
            prefs.stream_idle_timeout_secs = 1;
            // Two connection retries AND two stall retries share the same
            // config value, but operate on independent counters.
            prefs.request_retry.max_retries = 2;
            prefs.request_retry.base_delay_secs = 0;
            prefs.request_retry.max_delay_secs = 0;
            deps.services
                .user_preferences_storage
                .save(&prefs)
                .expect("save prefs");
        }
        let factory = LlmServiceFactoryService::new(Arc::new(
            super::test_fakes::RetryableThenStallThenCompleteLlmFactory::new(
                2, // two connection-retryable failures first
                1, // then one stalled stream
                vec!["ok".to_owned()],
            ),
        ));
        let _actor = harness
            .spawn_actor::<LlmActor>(LlmActorDeps {
                factory,
                deps,
                state: State::new(crate::common::app_state::AppState::default()),
            })
            .await;
        let completed_recorder = harness.spawn_recorder::<StreamCompleted>().await;

        let session_id = SessionId::new();
        let payload = SendToLlmProvider {
            model_used: None,
            reasoning_effort: None,
            session_id: session_id.clone(),
            messages: vec![],
            tool_definitions: vec![],
            provider_id: None,
            estimated_tokens: 0,
            dispatched_at: jiff::Timestamp::now(),
        };

        // When the inner service fails with two Retryable errors (consuming the
        // connection-retry budget on the first RetryingLlmService instance),
        // then stalls once (consuming the stall budget), then completes.
        harness.publish(payload).await;

        // Then the turn reaches Finished: the two connection retries did NOT
        // deplete the stall budget, so the stall retry still proceeds and
        // completes. A shared budget would have terminated with Error after
        // the stall (2 connection + 1 stall > 2 total).
        let completed =
            await_recorded(&completed_recorder, 1, std::time::Duration::from_secs(15)).await;
        assert!(
            completed
                .iter()
                .any(|sc| sc.reason == StreamCompletedReason::Finished),
            "independent stall counter must allow completion after connection retries"
        );
    }
    #[tokio::test]
    async fn start_stream_aborts_existing_stream_for_same_session() {
        // Given an LLM actor with an existing stream for a session.
        let mut actor = test_llm_actor().await;

        let session_id = SessionId::new();
        let payload = SendToLlmProvider {
            model_used: None,
            reasoning_effort: None,
            session_id: session_id.clone(),
            messages: vec![],
            tool_definitions: vec![],
            provider_id: None,
            estimated_tokens: 0,
            dispatched_at: jiff::Timestamp::now(),
        };

        // Start first stream and save the handle.
        actor.start_stream(&payload.clone()).await;
        let first_handle = actor.tasks.remove(&session_id);
        assert!(first_handle.is_some());
        // Re-insert for the second start_stream to find and abort.
        actor
            .tasks
            .insert(session_id.clone(), first_handle.unwrap());

        // When starting a second stream for the same session.
        actor.start_stream(&payload).await;
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
            model_used: None,
            reasoning_effort: None,
            session_id: session_id.clone(),
            messages: vec![],
            tool_definitions: vec![],
            provider_id: None,
            estimated_tokens: 0,
            dispatched_at: jiff::Timestamp::now(),
        };

        // When starting a stream.
        actor.start_stream(&payload).await;

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
            model_used: None,
            reasoning_effort: None,
            session_id: session_id.clone(),
            messages: vec![],
            tool_definitions: vec![],
            provider_id: None,
            estimated_tokens: 0,
            dispatched_at: jiff::Timestamp::now(),
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
        assert!(
            completed.is_empty(),
            "CancelStream with no active stream should be a no-op"
        );
    }

    #[tokio::test]
    async fn cancel_stream_via_bus_emits_completion() {
        // Given a test harness with an LLM actor using a never-completing fake.
        // This ensures the stream is actively streaming when we cancel.
        let harness = TestHarness::new().await;
        let factory = LlmServiceFactoryService::new(Arc::new(HangingLlmFactory));
        let _actor = harness
            .spawn_actor::<LlmActor>(LlmActorDeps {
                factory,
                deps: harness.actor_deps().await,
                state: State::new(crate::common::app_state::AppState::default()),
            })
            .await;
        let recorder = harness.spawn_recorder::<StreamCompleted>().await;

        let session_id = SessionId::new();
        harness
            .publish(SendToLlmProvider {
                model_used: None,
                reasoning_effort: None,
                session_id: session_id.clone(),
                messages: vec![],
                tool_definitions: vec![],
                provider_id: None,
                estimated_tokens: 0,
                dispatched_at: jiff::Timestamp::now(),
            })
            .await;
        // Give the stream a moment to start.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // When sending CancelStream via bus.
        harness
            .publish(CancelStream {
                session_id: session_id.clone(),
            })
            .await;

        // Then StreamCompleted(Canceled) is emitted.
        let completed = await_recorded(&recorder, 1, std::time::Duration::from_secs(5)).await;
        let found = completed
            .iter()
            .any(|sc| sc.reason == StreamCompletedReason::Canceled);
        assert!(found, "should emit StreamCompleted(Canceled)");
    }

    #[tokio::test]
    async fn stream_error_emits_chat_entry_and_completion_via_bus() {
        // Given a test harness with an LLM actor using an immediately-erroring fake.
        // The stream errors synchronously on creation, exercising emit_stream_error.
        let harness = TestHarness::new().await;
        let factory = LlmServiceFactoryService::new(Arc::new(ErroringLlmFactory));
        let _actor = harness
            .spawn_actor::<LlmActor>(LlmActorDeps {
                factory,
                deps: harness.actor_deps().await,
                state: State::new(crate::common::app_state::AppState::default()),
            })
            .await;
        let entry_recorder = harness.spawn_recorder::<PushChatEntry>().await;
        let completed_recorder = harness.spawn_recorder::<StreamCompleted>().await;

        // When sending SendToLlmProvider, which errors during stream creation.
        let session_id = SessionId::new();
        harness
            .publish(SendToLlmProvider {
                model_used: None,
                reasoning_effort: None,
                session_id: session_id.clone(),
                messages: vec![],
                tool_definitions: vec![],
                provider_id: None,
                estimated_tokens: 0,
                dispatched_at: jiff::Timestamp::now(),
            })
            .await;

        // Then an error PushChatEntry is emitted (from emit_stream_error).
        let entries = await_recorded(&entry_recorder, 1, std::time::Duration::from_secs(5)).await;
        assert!(
            !entries.is_empty(),
            "stream error should publish an error chat entry"
        );

        // And a StreamCompleted(Error) is emitted, so the session isn't stuck streaming.
        let completed =
            await_recorded(&completed_recorder, 1, std::time::Duration::from_secs(5)).await;
        let found = completed
            .iter()
            .any(|sc| sc.reason == StreamCompletedReason::Error);
        assert!(found, "stream error should emit StreamCompleted(Error)");
    }

    #[rstest::rstest]
    fn to_retry_config_uses_actual_values_not_defaults() {
        // If to_retry_config returned Default::default(), all durations would be zero.
        let config = RequestRetryConfig {
            max_retries: 3,
            base_delay_secs: 5,
            max_delay_secs: 120,
        };

        let retry = config.to_retry_config();

        assert_eq!(retry.max_retries, 3);
        assert_eq!(retry.base_delay, std::time::Duration::from_secs(5));
        assert_eq!(retry.max_delay, std::time::Duration::from_mins(2));
    }

    // ------------------------------------------------------------------
    // Phase 1: Stream idle-stall detection + auto-retry tests
    // ------------------------------------------------------------------

    /// Builds a [`jinn_provider::ToolStream`] from a scripted event sequence,
    /// optionally inserting an idle gap (a pending future) at a chosen point.
    fn scripted_stream(events: Vec<jinn_provider::StreamEvent>) -> jinn_provider::ToolStream {
        let events: Vec<Result<jinn_provider::StreamEvent, Report<LlmServiceError>>> =
            events.into_iter().map(Ok).collect();
        Box::pin(futures::stream::iter(events))
    }

    /// A stream that yields `head` then never produces another event.
    async fn stalled_stream_after(
        head: Vec<jinn_provider::StreamEvent>,
    ) -> jinn_provider::ToolStream {
        let head_events: Vec<Result<jinn_provider::StreamEvent, Report<LlmServiceError>>> =
            head.into_iter().map(Ok).collect();
        let stalled: futures::stream::Pending<
            Result<jinn_provider::StreamEvent, Report<LlmServiceError>>,
        > = futures::stream::pending();
        Box::pin(futures::stream::iter(head_events).chain(stalled))
    }

    /// Builds a stream that yields `events` in order, sleeping `gap` after each one.
    /// Used to verify the idle timer resets on every event.
    fn gapped_stream(
        events: Vec<jinn_provider::StreamEvent>,
        gap: std::time::Duration,
    ) -> jinn_provider::ToolStream {
        use futures::stream;
        use std::sync::Arc;
        use std::sync::Mutex as StdMutex;
        let events = Arc::new(StdMutex::new(events));
        let stream = stream::unfold((events, gap), move |(events, gap)| async move {
            let next = events.lock().expect("lock").first().cloned();
            match next {
                Some(event) => {
                    events.lock().expect("lock").remove(0);
                    tokio::time::sleep(gap).await;
                    Some((Ok(event), (events, gap)))
                }
                None => None,
            }
        });
        Box::pin(stream)
    }
    #[tokio::test]
    async fn process_stream_events_returns_stalled_when_no_event_for_idle_timeout() {
        // Given a harness bus and a stream that never yields.
        let harness = TestHarness::new().await;
        let stream: jinn_provider::ToolStream = Box::pin(futures::stream::pending::<
            Result<jinn_provider::StreamEvent, Report<LlmServiceError>>,
        >());
        let sid = SessionId::new();

        // When processing with a short idle timeout.
        let outcome = process_stream_events(
            stream,
            &harness.bus(),
            &sid,
            "test-model",
            jiff::Timestamp::now(),
            std::time::Duration::from_millis(50),
        )
        .await;

        // Then the outcome is Stalled (not Completed) and it returns quickly.
        assert!(
            matches!(outcome, StreamOutcome::Stalled),
            "idle stream should stall"
        );
    }

    #[tokio::test]
    async fn process_stream_events_completes_on_done_event() {
        // Given a stream that yields a token then Done.
        use jinn_provider::{StopReason, StreamEvent};
        let harness = TestHarness::new().await;
        let stream = scripted_stream(vec![
            StreamEvent::Text("hi".to_owned()),
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: None,
            },
        ]);
        let sid = SessionId::new();

        // When processing with a generous idle timeout.
        let outcome = process_stream_events(
            stream,
            &harness.bus(),
            &sid,
            "test-model",
            jiff::Timestamp::now(),
            std::time::Duration::from_secs(5),
        )
        .await;

        // Then the outcome is Completed.
        assert!(
            matches!(outcome, StreamOutcome::Completed),
            "Done should complete"
        );
    }

    #[tokio::test]
    async fn process_stream_events_publishes_citations_on_done_when_accumulated() {
        // Given a stream carrying url_citation annotations then Done.
        use jinn_provider::{StopReason, StreamEvent, UrlCitation};
        let harness = TestHarness::new().await;
        let recorder = harness
            .spawn_recorder::<crate::feat::session::protocol::citations_received::CitationsReceived>()
            .await;
        let stream = scripted_stream(vec![
            StreamEvent::Citations(vec![UrlCitation {
                url: "https://example.com/a".to_owned(),
                title: "Source A".to_owned(),
                content: None,
                start_index: None,
                end_index: None,
            }]),
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: None,
            },
        ]);
        let sid = SessionId::new();

        // When processing the stream to completion.
        let outcome = process_stream_events(
            stream,
            &harness.bus(),
            &sid,
            "test-model",
            jiff::Timestamp::now(),
            std::time::Duration::from_secs(5),
        )
        .await;

        // Then the outcome is Completed.
        assert!(
            matches!(outcome, StreamOutcome::Completed),
            "Done should complete"
        );

        // And exactly one CitationsReceived was published on the bus.
        let recorded = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
        assert_eq!(recorded.len(), 1, "one CitationsReceived published");
        assert_eq!(recorded[0].citations.len(), 1);
        assert_eq!(recorded[0].citations[0].url, "https://example.com/a");
        assert_eq!(recorded[0].session_id, sid);
    }

    #[tokio::test]
    async fn process_stream_events_resets_idle_timeout_on_each_text_token() {
        // Given a stream that yields tokens with sub-threshold gaps then Done.
        // The gaps (40ms) are below the idle timeout (100ms), so the timer resets.
        use jinn_provider::{StopReason, StreamEvent};
        let harness = TestHarness::new().await;
        let events = vec![
            StreamEvent::Text("a".to_owned()),
            StreamEvent::Text("b".to_owned()),
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: None,
            },
        ];
        let stream = gapped_stream(events, std::time::Duration::from_millis(40));
        let sid = SessionId::new();

        // When processing with a 100ms idle timeout.
        let outcome = process_stream_events(
            stream,
            &harness.bus(),
            &sid,
            "test-model",
            jiff::Timestamp::now(),
            std::time::Duration::from_millis(100),
        )
        .await;

        // Then the outcome is Completed (the 40ms gaps never trip the 100ms timer).
        assert!(
            matches!(outcome, StreamOutcome::Completed),
            "slow-but-active stream should complete, not stall"
        );
    }

    #[tokio::test]
    async fn process_stream_events_resets_idle_timeout_on_reasoning_tokens() {
        // Given a stream that yields reasoning deltas with sub-threshold gaps then Done.
        use jinn_provider::{StopReason, StreamEvent};
        let harness = TestHarness::new().await;
        let events = vec![
            StreamEvent::Reasoning("thinking".to_owned()),
            StreamEvent::Reasoning("more".to_owned()),
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: None,
            },
        ];
        let stream = gapped_stream(events, std::time::Duration::from_millis(40));
        let sid = SessionId::new();

        // When processing with a 100ms idle timeout.
        let outcome = process_stream_events(
            stream,
            &harness.bus(),
            &sid,
            "test-model",
            jiff::Timestamp::now(),
            std::time::Duration::from_millis(100),
        )
        .await;

        // Then the outcome is Completed (reasoning tokens reset the timer).
        assert!(
            matches!(outcome, StreamOutcome::Completed),
            "reasoning tokens should reset the idle timer"
        );
    }

    #[tokio::test]
    async fn process_stream_events_stalls_after_partial_tokens_then_idle() {
        // Given a stream that yields one token then goes idle forever.
        use jinn_provider::StreamEvent;
        let harness = TestHarness::new().await;
        let stream = stalled_stream_after(vec![StreamEvent::Text("partial".to_owned())]).await;
        let sid = SessionId::new();

        // When processing with a short idle timeout.
        let outcome = process_stream_events(
            stream,
            &harness.bus(),
            &sid,
            "test-model",
            jiff::Timestamp::now(),
            std::time::Duration::from_millis(50),
        )
        .await;

        // Then the outcome is Stalled (the token was consumed, but no Done followed).
        assert!(
            matches!(outcome, StreamOutcome::Stalled),
            "stream that goes idle after partial tokens should stall"
        );
    }

    #[rstest::rstest]
    fn compute_stall_backoff_is_capped_at_max_delay(#[values(1, 2, 3, 5, 10)] attempt: u32) {
        // Given a config with a small max delay cap.
        let config = RequestRetryConfig {
            max_retries: 5,
            base_delay_secs: 2,
            max_delay_secs: 4,
        };

        // When computing the backoff for this attempt.
        let delay = compute_stall_backoff(&config, attempt);

        // Then the delay never exceeds the max cap.
        assert!(
            delay <= std::time::Duration::from_secs(4),
            "backoff for attempt {attempt} ({delay:?}) must not exceed max_delay"
        );
    }

    #[tokio::test]
    async fn compute_stall_backoff_uses_base_delay_for_first_attempt_upper_bound() {
        // Given a config with a known base delay and large max.
        let config = RequestRetryConfig {
            max_retries: 5,
            base_delay_secs: 2,
            max_delay_secs: 60,
        };

        // When computing the backoff for attempt 1 (exponential = 2^1 = 2s * base 2s = 4s).
        // The full-jitter result is in [0, 4s].
        let delay = compute_stall_backoff(&config, 1);

        // Then the delay is within [0, 4s].
        assert!(
            delay <= std::time::Duration::from_secs(4),
            "attempt-1 backoff must be bounded by base*2^1"
        );
    }
}
