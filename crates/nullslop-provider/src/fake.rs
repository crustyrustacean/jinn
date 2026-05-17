//! Fake LLM service for testing.
//!
//! Supports both plain text streaming and tool-call streaming.
//! Use [`FakeLlmServiceFactory::with_tool_calls`] to simulate tool responses.
//! Use [`FakeLlmServiceFactory::with_tool_loop`] to simulate a multi-turn
//! tool loop where the first call returns `tool_use` and subsequent calls
//! return `end_turn`.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::llm_message::LlmMessage;
use crate::tool_types::ToolCall;
use error_stack::Report;
use futures::stream;

use crate::service::{ChatStream, LlmService, LlmServiceError, LlmServiceFactory, ToolStream};
use crate::stream_event::StopReason;
use crate::stream_event::StreamEvent;

/// Special prompt that triggers multi-turn tool loop behavior.
///
/// When the last user message contains this string, the fake service
/// returns a `tool_use` response on the first call and a text-only response
/// on subsequent calls.
pub const TOOL_LOOP_TRIGGER: &str = "__tool_loop_test__";

/// Factory that creates fake LLM service instances.
///
/// Each service yields the tokens the factory was configured with.
/// Optionally emits tool call events before the text tokens.
/// Use this in tests to avoid hitting real LLM backends.
#[derive(Debug, Clone)]
pub struct FakeLlmServiceFactory {
    /// Tokens each created service will yield.
    tokens: Vec<String>,
    /// Tool calls to emit during streaming.
    tool_calls: Vec<ToolCall>,
    /// Shared call counter for multi-turn tool loop simulation.
    ///
    /// When set, the first call with the trigger prompt returns `tool_use`,
    /// and subsequent calls return `end_turn` text.
    tool_loop_call_count: Option<Arc<AtomicUsize>>,
    /// Tool calls to use on the first call of a tool loop.
    tool_loop_first_tool_calls: Vec<ToolCall>,
    /// Text tokens to use on subsequent calls of a tool loop.
    tool_loop_subsequent_tokens: Vec<String>,
    /// Messages received by all services created from this factory.
    received_calls: Arc<Mutex<Vec<Vec<LlmMessage>>>>,
}

impl FakeLlmServiceFactory {
    /// Create a factory whose services yield the given tokens (text only).
    #[must_use]
    pub fn new(tokens: Vec<String>) -> Self {
        Self {
            tokens,
            tool_calls: vec![],
            tool_loop_call_count: None,
            tool_loop_first_tool_calls: vec![],
            tool_loop_subsequent_tokens: vec![],
            received_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Create a factory whose services yield text tokens and tool call events.
    ///
    /// The stream emits: text tokens → tool call events → Done.
    /// The stop reason is `"tool_use"` when tool calls are configured,
    /// `"end_turn"` otherwise.
    #[must_use]
    pub fn with_tool_calls(tokens: Vec<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            tokens,
            tool_calls,
            tool_loop_call_count: None,
            tool_loop_first_tool_calls: vec![],
            tool_loop_subsequent_tokens: vec![],
            received_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Create a factory that simulates a multi-turn tool loop.
    ///
    /// When the last user message contains [`TOOL_LOOP_TRIGGER`], the fake
    /// returns a `tool_use` response (with the given tool calls and tokens) on
    /// the first call, and a text-only response (with the subsequent tokens)
    /// on the second call. This simulates the LLM calling a tool, receiving
    /// results, and then producing a final text response.
    ///
    /// When the last user message does not contain the trigger, behaves like
    /// [`FakeLlmServiceFactory::new`] with the `tokens` parameter.
    #[must_use]
    pub fn with_tool_loop(
        tokens: Vec<String>,
        first_tool_calls: Vec<ToolCall>,
        subsequent_tokens: Vec<String>,
    ) -> Self {
        Self {
            tokens,
            tool_calls: vec![],
            tool_loop_call_count: Some(Arc::new(AtomicUsize::new(0))),
            tool_loop_first_tool_calls: first_tool_calls,
            tool_loop_subsequent_tokens: subsequent_tokens,
            received_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Returns the number of tool loop calls made so far.
    ///
    /// Only meaningful when created with [`Self::with_tool_loop`].
    #[must_use]
    pub fn tool_loop_call_count(&self) -> usize {
        self.tool_loop_call_count
            .as_ref()
            .map_or(0, |c| c.load(Ordering::SeqCst))
    }

    /// Returns a copy of all messages received by services created from this factory.
    ///
    /// # Panics
    ///
    /// Panics if the mutex is poisoned.
    #[must_use]
    pub fn received_calls(&self) -> Vec<Vec<LlmMessage>> {
        self.received_calls.lock().expect("lock").clone()
    }

    /// Clears all recorded calls.
    ///
    /// # Panics
    ///
    /// Panics if the mutex is poisoned.
    pub fn clear_calls(&self) {
        self.received_calls.lock().expect("lock").clear();
    }
}

impl LlmServiceFactory for FakeLlmServiceFactory {
    fn create(&self) -> Result<Box<dyn LlmService>, Report<LlmServiceError>> {
        Ok(Box::new(FakeLlmService {
            tokens: self.tokens.clone(),
            tool_calls: self.tool_calls.clone(),
            tool_loop_call_count: self.tool_loop_call_count.clone(),
            tool_loop_first_tool_calls: self.tool_loop_first_tool_calls.clone(),
            tool_loop_subsequent_tokens: self.tool_loop_subsequent_tokens.clone(),
            received_calls: self.received_calls.clone(),
        }))
    }

    fn name(&self) -> &'static str {
        "FakeLlm"
    }
}

/// A fake LLM service that yields preconfigured tokens and tool calls.
struct FakeLlmService {
    /// Tokens to yield during streaming.
    tokens: Vec<String>,
    /// Tool calls to emit during tool streaming.
    tool_calls: Vec<ToolCall>,
    /// Shared call counter for multi-turn tool loop simulation.
    tool_loop_call_count: Option<Arc<AtomicUsize>>,
    /// Tool calls for the first call of a tool loop.
    tool_loop_first_tool_calls: Vec<ToolCall>,
    /// Text tokens for subsequent calls of a tool loop.
    tool_loop_subsequent_tokens: Vec<String>,
    /// Shared call recording with the parent factory.
    received_calls: Arc<Mutex<Vec<Vec<LlmMessage>>>>,
}

impl FakeLlmService {
    /// Extracts the content of the last user message, if any.
    fn last_user_content(messages: &[LlmMessage]) -> Option<&str> {
        messages.iter().rev().find_map(|msg| match msg {
            LlmMessage::User { content } => Some(content.as_str()),
            _ => None,
        })
    }

    /// Returns true if the messages contain the tool loop trigger.
    fn is_tool_loop_trigger(messages: &[LlmMessage]) -> bool {
        Self::last_user_content(messages).is_some_and(|c| c.contains(TOOL_LOOP_TRIGGER))
    }

    /// Builds a `tool_use` stream for the first call of a tool loop.
    fn build_tool_loop_first_stream(&self) -> Vec<Result<StreamEvent, Report<LlmServiceError>>> {
        let mut events: Vec<Result<StreamEvent, Report<LlmServiceError>>> = Vec::new();

        for token in &self.tokens {
            events.push(Ok(StreamEvent::Text(token.clone())));
        }

        for (index, tc) in self.tool_loop_first_tool_calls.iter().enumerate() {
            events.push(Ok(StreamEvent::ToolUseStart {
                index,
                id: tc.id.clone(),
                name: tc.name.clone(),
            }));
            events.push(Ok(StreamEvent::ToolUseInputDelta {
                index,
                partial_json: tc.arguments.clone(),
            }));
            events.push(Ok(StreamEvent::ToolUseComplete {
                index,
                tool_call: tc.clone(),
            }));
        }

        events.push(Ok(StreamEvent::Done {
            stop_reason: StopReason::ToolUse,
        }));

        events
    }

    /// Builds a text-only stream for subsequent calls of a tool loop.
    fn build_tool_loop_subsequent_stream(
        &self,
    ) -> Vec<Result<StreamEvent, Report<LlmServiceError>>> {
        let mut events: Vec<Result<StreamEvent, Report<LlmServiceError>>> = Vec::new();

        for token in &self.tool_loop_subsequent_tokens {
            events.push(Ok(StreamEvent::Text(token.clone())));
        }

        events.push(Ok(StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        }));

        events
    }
}

#[async_trait::async_trait]
impl LlmService for FakeLlmService {
    async fn chat_stream(
        &self,
        messages: Vec<LlmMessage>,
    ) -> Result<ChatStream, Report<LlmServiceError>> {
        // Record the messages for test observability.
        self.received_calls.lock().expect("lock").push(messages);

        let tokens = self.tokens.clone();
        let stream: ChatStream = Box::pin(stream::iter(tokens.into_iter().map(Ok)));
        Ok(stream)
    }

    async fn chat_stream_with_tools(
        &self,
        messages: Vec<LlmMessage>,
        _tools: Vec<crate::tool_types::ToolDefinition>,
    ) -> Result<ToolStream, Report<LlmServiceError>> {
        // Record the messages for test observability.
        self.received_calls
            .lock()
            .expect("lock")
            .push(messages.clone());

        // Check for multi-turn tool loop trigger.
        if let Some(ref counter) = self.tool_loop_call_count
            && Self::is_tool_loop_trigger(&messages)
        {
            let call_num = counter.fetch_add(1, Ordering::SeqCst);
            if call_num == 0 {
                return Ok(Box::pin(stream::iter(self.build_tool_loop_first_stream())));
            }
            return Ok(Box::pin(stream::iter(
                self.build_tool_loop_subsequent_stream(),
            )));
        }

        let mut events: Vec<Result<StreamEvent, Report<LlmServiceError>>> = Vec::new();

        // Emit text tokens.
        for token in &self.tokens {
            events.push(Ok(StreamEvent::Text(token.clone())));
        }

        if self.tool_calls.is_empty() {
            // No tool calls — just text and Done(end_turn).
            events.push(Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            }));
        } else {
            // Emit tool call events.
            for (index, tc) in self.tool_calls.iter().enumerate() {
                events.push(Ok(StreamEvent::ToolUseStart {
                    index,
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                }));
                events.push(Ok(StreamEvent::ToolUseInputDelta {
                    index,
                    partial_json: tc.arguments.clone(),
                }));
                events.push(Ok(StreamEvent::ToolUseComplete {
                    index,
                    tool_call: tc.clone(),
                }));
            }

            // Terminal event.
            events.push(Ok(StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
            }));
        }

        Ok(Box::pin(stream::iter(events)))
    }
}
