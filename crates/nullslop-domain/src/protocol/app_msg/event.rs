//! Event types for the component event pipeline.
//!
//! The [`Event`] enum is the unified type the host broadcasts to
//! inform internal handlers and actors about state changes and input.
//!
//! Individual event structs live in domain modules (`chat_input`, `system`,
//! `custom`, `actor`). Consumers import structs directly from those modules —
//! this facade only re-exports infrastructure types.
//!
//! # When adding a new event
//!
//! Every new event struct **must** be added as a variant on the [`Event`] enum
//! below. Creating the struct alone is not enough — the bus broadcasts based on
//! enum variants, so a missing variant means the event is invisible to the system.

use serde::{Deserialize, Serialize};

// Internal imports for enum definition, type_name(), and tests.
use crate::common::actor::protocol::event::{ActorShutdownCompleted, ActorStarted, ActorStarting};
use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
use crate::feat::context::protocol::event::{
    PromptAssembled, PromptStrategySwitched, StrategyStateUpdated,
};
// Re-export infrastructure types only. Domain structs are imported from their modules.
pub use crate::common::actor::event_msg::EventMsg;
use crate::feat::provider::protocol::event::{
    ModelsRefreshed, PromptTemplatesLoaded, ProviderSwitched, StreamCompleted, StreamToken,
};
use crate::feat::session::protocol::session_save_requested::SessionSaveRequested;
use crate::feat::skills::skills_scan_actor::SkillsLoaded;
use crate::feat::tools_actor::protocol::event::{
    ToolBatchCompleted, ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted,
    ToolUseStarted, ToolsRegistered,
};
use crate::protocol::system::{KeyDown, KeyUp, ModeChanged};

/// Every event the host can broadcast.
///
/// Actors subscribe to relevant variants; the host also
/// uses them internally to drive UI updates.
///
/// **When adding a new event struct**, you must add a corresponding variant to
/// this enum. An event struct defined in a domain module without an enum variant
/// here will not be broadcast by the bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Event {
    /// A key was pressed down.
    #[serde(rename = "key_down")]
    KeyDown {
        /// Which key was pressed.
        #[serde(flatten)]
        payload: KeyDown,
    },
    /// A key was released.
    #[serde(rename = "key_up")]
    KeyUp {
        /// Which key was released.
        #[serde(flatten)]
        payload: KeyUp,
    },
    /// A chat entry was added to the conversation history.
    #[serde(rename = "chat_entry_submitted")]
    ChatEntrySubmitted {
        /// The chat entry that was added.
        #[serde(flatten)]
        payload: ChatEntrySubmitted,
    },
    /// The application mode changed.
    #[serde(rename = "mode_changed")]
    ModeChanged {
        /// The previous and new mode.
        #[serde(flatten)]
        payload: ModeChanged,
    },
    /// An actor is starting up.
    #[serde(rename = "actor_starting")]
    ActorStarting {
        /// Which actor is starting.
        #[serde(flatten)]
        payload: ActorStarting,
    },
    /// An actor has finished starting up.
    #[serde(rename = "actor_started")]
    ActorStarted {
        /// Which actor finished starting.
        #[serde(flatten)]
        payload: ActorStarted,
    },
    /// An actor has completed shutdown.
    #[serde(rename = "actor_shutdown_completed")]
    ActorShutdownCompleted {
        /// Which actor finished shutting down.
        #[serde(flatten)]
        payload: ActorShutdownCompleted,
    },
    /// A streaming LLM response completed.
    #[serde(rename = "stream_completed")]
    StreamCompleted {
        /// The session whose stream completed.
        #[serde(flatten)]
        payload: StreamCompleted,
    },
    /// A single token from a streaming LLM response.
    #[serde(rename = "stream_token")]
    StreamToken {
        /// The stream token.
        #[serde(flatten)]
        payload: StreamToken,
    },
    /// A tool call has started in the LLM stream.
    #[serde(rename = "tool_use_started")]
    ToolUseStarted {
        /// The tool use started payload.
        #[serde(flatten)]
        payload: ToolUseStarted,
    },
    /// A complete tool call was received from the LLM stream.
    #[serde(rename = "tool_call_received")]
    ToolCallReceived {
        /// The received tool call payload.
        #[serde(flatten)]
        payload: ToolCallReceived,
    },
    /// Streaming update for a tool call's arguments being assembled.
    #[serde(rename = "tool_call_streaming")]
    ToolCallStreaming {
        /// The streaming tool call payload.
        #[serde(flatten)]
        payload: ToolCallStreaming,
    },
    /// The active provider was switched.
    #[serde(rename = "provider_switched")]
    ProviderSwitched {
        /// The provider switch confirmation.
        #[serde(flatten)]
        payload: ProviderSwitched,
    },
    /// Models refresh completed.
    #[serde(rename = "models_refreshed")]
    ModelsRefreshed {
        /// Refresh results per provider.
        #[serde(flatten)]
        payload: ModelsRefreshed,
    },
    /// Prompt templates loaded after a rescan.
    #[serde(rename = "prompt_templates_loaded")]
    PromptTemplatesLoaded {
        /// The loaded templates and optional error.
        #[serde(flatten)]
        payload: PromptTemplatesLoaded,
    },
    /// All tool calls in a batch have completed execution.
    #[serde(rename = "tool_batch_completed")]
    ToolBatchCompleted {
        /// The batch completion payload.
        #[serde(flatten)]
        payload: ToolBatchCompleted,
    },
    /// A single tool execution completed.
    #[serde(rename = "tool_execution_completed")]
    ToolExecutionCompleted {
        /// The execution completion payload.
        #[serde(flatten)]
        payload: ToolExecutionCompleted,
    },
    /// Tools were registered by an actor.
    #[serde(rename = "tools_registered")]
    ToolsRegistered {
        /// The registration confirmation payload.
        #[serde(flatten)]
        payload: ToolsRegistered,
    },
    /// A prompt has been assembled and is ready to send.
    #[serde(rename = "prompt_assembled")]
    PromptAssembled {
        /// The assembled prompt payload.
        #[serde(flatten)]
        payload: PromptAssembled,
    },
    /// A session's prompt assembly strategy has been switched.
    #[serde(rename = "prompt_strategy_switched")]
    PromptStrategySwitched {
        /// The strategy switch payload.
        #[serde(flatten)]
        payload: PromptStrategySwitched,
    },
    /// A strategy's session state has changed and should be persisted.
    #[serde(rename = "strategy_state_updated")]
    StrategyStateUpdated {
        /// The state update payload.
        #[serde(flatten)]
        payload: StrategyStateUpdated,
    },
    /// Session data should be persisted to disk.
    #[serde(rename = "session_save_requested")]
    SessionSaveRequested {
        /// The session save request payload.
        #[serde(flatten)]
        payload: SessionSaveRequested,
    },
    /// Agent skills have been scanned and loaded.
    #[serde(rename = "skills_loaded")]
    SkillsLoaded {
        /// The loaded skills and optional error.
        #[serde(flatten)]
        payload: SkillsLoaded,
    },
    /// Personas have been scanned and loaded from disk.
    #[serde(rename = "personas_loaded")]
    PersonasLoaded {
        /// The loaded personas and optional error.
        #[serde(flatten)]
        payload: crate::feat::context::protocol::event::PersonasLoaded,
    },
}

impl Event {
    /// Returns the subscription-relevant type name for event routing.
    #[must_use]
    pub fn type_name(&self) -> Option<&str> {
        match self {
            Self::ChatEntrySubmitted { .. } => Some(ChatEntrySubmitted::TYPE_NAME),
            Self::ActorStarting { .. } => Some(ActorStarting::TYPE_NAME),
            Self::ActorStarted { .. } => Some(ActorStarted::TYPE_NAME),
            Self::ActorShutdownCompleted { .. } => Some(ActorShutdownCompleted::TYPE_NAME),
            Self::KeyDown { .. } => Some(KeyDown::TYPE_NAME),
            Self::KeyUp { .. } => Some(KeyUp::TYPE_NAME),
            Self::ModeChanged { .. } => Some(ModeChanged::TYPE_NAME),
            Self::StreamCompleted { .. } => Some(StreamCompleted::TYPE_NAME),
            Self::StreamToken { .. } => Some(StreamToken::TYPE_NAME),
            Self::ToolUseStarted { .. } => Some(ToolUseStarted::TYPE_NAME),
            Self::ToolCallReceived { .. } => Some(ToolCallReceived::TYPE_NAME),
            Self::ToolCallStreaming { .. } => Some(ToolCallStreaming::TYPE_NAME),
            Self::ProviderSwitched { .. } => Some(ProviderSwitched::TYPE_NAME),
            Self::ModelsRefreshed { .. } => Some(ModelsRefreshed::TYPE_NAME),
            Self::PromptTemplatesLoaded { .. } => Some(PromptTemplatesLoaded::TYPE_NAME),
            Self::ToolBatchCompleted { .. } => Some(ToolBatchCompleted::TYPE_NAME),
            Self::ToolExecutionCompleted { .. } => Some(ToolExecutionCompleted::TYPE_NAME),
            Self::ToolsRegistered { .. } => Some(ToolsRegistered::TYPE_NAME),
            Self::PromptAssembled { .. } => Some(PromptAssembled::TYPE_NAME),
            Self::PromptStrategySwitched { .. } => Some(PromptStrategySwitched::TYPE_NAME),
            Self::StrategyStateUpdated { .. } => Some(StrategyStateUpdated::TYPE_NAME),

            Self::SessionSaveRequested { .. } => Some(SessionSaveRequested::TYPE_NAME),
            Self::SkillsLoaded { .. } => Some(SkillsLoaded::TYPE_NAME),
            Self::PersonasLoaded { .. } => Some(
                crate::feat::context::protocol::event::PersonasLoaded::TYPE_NAME,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feat::provider::protocol::event::StreamCompletedReason;
    use crate::feat::session::protocol::session_save_requested::SessionSaveRequested;
    use crate::protocol::{ChatEntry, Key, KeyEvent, Mode, Modifiers, SessionId};

    #[rstest::rstest]
    fn event_chat_entry_submitted_preserves_entry() {
        // Given a ChatEntrySubmitted event with a user entry.
        let entry = ChatEntry::user("hello");
        let event = Event::ChatEntrySubmitted {
            payload: ChatEntrySubmitted {
                session_id: SessionId::new(),
                entry,
            },
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&event).expect("serialize");
        let back: Event = serde_json::from_str(&json).expect("deserialize");

        // Then entry text is preserved.
        match back {
            Event::ChatEntrySubmitted { payload } => {
                assert_eq!(
                    payload.entry.kind,
                    crate::ChatEntryKind::User("hello".to_owned())
                );
            }
            other => panic!("expected ChatEntrySubmitted, got {other:?}"),
        }
    }

    #[rstest::rstest]
    #[case::key_down(Event::KeyDown { payload: KeyDown { key: KeyEvent { key: Key::Char('a'), modifiers: Modifiers::none() } } })]
    #[case::key_up(Event::KeyUp { payload: KeyUp { key: KeyEvent { key: Key::Enter, modifiers: Modifiers::none() } } })]
    #[case::chat_submitted(Event::ChatEntrySubmitted { payload: ChatEntrySubmitted { session_id: SessionId::new(), entry: ChatEntry::user("test") } })]
    #[case::mode_changed(Event::ModeChanged { payload: ModeChanged { from: Mode::Normal, to: Mode::Input } })]
    #[case::actor_starting(Event::ActorStarting { payload: ActorStarting { name: "actor-a".into(), description: None } })]
    #[case::actor_started(Event::ActorStarted { payload: ActorStarted { name: "actor-a".into(), description: None } })]
    #[case::actor_shutdown_completed(Event::ActorShutdownCompleted { payload: ActorShutdownCompleted { name: "actor-a".into() } })]
    #[case::stream_completed(Event::StreamCompleted { payload: StreamCompleted { session_id: SessionId::new(), reason: StreamCompletedReason::Finished, assistant_content: None, tool_calls: None } })]
    #[case::stream_token(Event::StreamToken { payload: StreamToken { session_id: SessionId::new(), index: 0, token: "hello".into() } })]
    #[case::tool_use_started(Event::ToolUseStarted { payload: ToolUseStarted { session_id: SessionId::new(), index: 0, id: "call_1".into(), name: "echo".into() } })]
    #[case::tool_call_received(Event::ToolCallReceived { payload: ToolCallReceived { session_id: SessionId::new(), tool_call: crate::ToolCall { id: "call_1".into(), name: "echo".into(), arguments: "{}".into() } } })]
    #[case::tool_call_streaming(Event::ToolCallStreaming { payload: ToolCallStreaming { session_id: SessionId::new(), index: 0, partial_json: "{\"a\":".into() } })]
    #[case::provider_switched(Event::ProviderSwitched { payload: ProviderSwitched { provider_name: "Ollama".into() } })]
    #[case::models_refreshed(Event::ModelsRefreshed { payload: ModelsRefreshed { results: std::collections::HashMap::new(), errors: std::collections::HashMap::new() } })]
    #[case::prompt_templates_loaded(Event::PromptTemplatesLoaded { payload: PromptTemplatesLoaded { templates: vec![], error: None } })]
    #[case::tool_batch_completed(Event::ToolBatchCompleted { payload: ToolBatchCompleted { session_id: SessionId::new(), results: vec![crate::ToolResult { tool_call_id: "call_1".into(), name: "echo".into(), content: "hi".into(), success: true }] } })]
    #[case::tool_execution_completed(Event::ToolExecutionCompleted { payload: ToolExecutionCompleted { session_id: SessionId::new(), result: crate::ToolResult { tool_call_id: "call_1".into(), name: "echo".into(), content: "hi".into(), success: true } } })]
    #[case::tools_registered(Event::ToolsRegistered { payload: ToolsRegistered { provider: "echo-actor".into(), definitions: vec![crate::ToolDefinition { name: "echo".into(), description: "echoes".into(), parameters: serde_json::json!({}) }] } })]
    #[case::prompt_assembled(Event::PromptAssembled { payload: PromptAssembled { session_id: SessionId::new(), system_prompt: None, messages: vec![] } })]
    #[case::prompt_strategy_switched(Event::PromptStrategySwitched { payload: PromptStrategySwitched { session_id: SessionId::new(), strategy_id: crate::PromptStrategyId::sliding_window() } })]
    #[case::strategy_state_updated(Event::StrategyStateUpdated { payload: StrategyStateUpdated { session_id: SessionId::new(), strategy_id: crate::PromptStrategyId::compaction(), blob: serde_json::json!({"compaction_count": 0}) } })]
    #[case::session_save_requested(Event::SessionSaveRequested { payload: SessionSaveRequested {
        session_id: SessionId::new(),
        title: "Test".to_owned(),
        history: vec![ChatEntry::user("hello")],
        active_strategy: crate::PromptStrategyId::passthrough(),
        blobs: std::collections::HashMap::new(),
    } })]
    #[case::skills_loaded(Event::SkillsLoaded { payload: crate::feat::skills::skills_scan_actor::SkillsLoaded {
        skills: vec![],
        error: None,
    } })]
    #[case::personas_loaded(Event::PersonasLoaded { payload: crate::feat::context::protocol::event::PersonasLoaded {
        personas: vec![],
        error: None,
    } })]
    fn event_roundtrip_all_variants(#[case] event: Event) {
        // Given an event variant.
        let json = serde_json::to_string(&event).expect("serialize");

        // When deserialized.
        let back: Event = serde_json::from_str(&json).expect("deserialize");

        // Then it matches the original when re-serialized.
        let back_json = serde_json::to_string(&back).expect("re-serialize");
        assert_eq!(json, back_json);
    }

    /// Checks that `Event::type_name()` delegates to the correct payload `TYPE_NAME`
    /// for all event variants that have a meaningful `type_name`.
    #[rstest::rstest]
    #[case::chat_submitted(
        Event::ChatEntrySubmitted { payload: ChatEntrySubmitted { session_id: SessionId::new(), entry: ChatEntry::user("test") } },
        ChatEntrySubmitted::TYPE_NAME
    )]
    #[case::actor_starting(
        Event::ActorStarting { payload: ActorStarting { name: "actor-a".into(), description: None } },
        ActorStarting::TYPE_NAME
    )]
    #[case::actor_started(
        Event::ActorStarted { payload: ActorStarted { name: "actor-a".into(), description: None } },
        ActorStarted::TYPE_NAME
    )]
    #[case::actor_shutdown_completed(
        Event::ActorShutdownCompleted { payload: ActorShutdownCompleted { name: "actor-a".into() } },
        ActorShutdownCompleted::TYPE_NAME
    )]
    #[case::key_down(
        Event::KeyDown { payload: KeyDown { key: KeyEvent { key: Key::Enter, modifiers: Modifiers::none() } } },
        KeyDown::TYPE_NAME
    )]
    #[case::key_up(
        Event::KeyUp { payload: KeyUp { key: KeyEvent { key: Key::Char('a'), modifiers: Modifiers::none() } } },
        KeyUp::TYPE_NAME
    )]
    #[case::mode_changed(
        Event::ModeChanged { payload: ModeChanged { from: Mode::Normal, to: Mode::Input } },
        ModeChanged::TYPE_NAME
    )]
    #[case::prompt_templates_loaded(
        Event::PromptTemplatesLoaded { payload: PromptTemplatesLoaded { templates: vec![], error: None } },
        PromptTemplatesLoaded::TYPE_NAME
    )]
    fn event_type_name_returns_payload_type_name(#[case] event: Event, #[case] expected: &str) {
        // Given an Event variant with a payload.
        // When calling type_name().
        // Then it returns Some of the payload's TYPE_NAME.
        assert_eq!(event.type_name(), Some(expected));
    }
}
