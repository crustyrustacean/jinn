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
use crate::common::actor::protocol::event::{
    ActorShutdownCompleted, ActorStarted, ActorStarting, AllActorsSpawned,
};
use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
use crate::feat::context::protocol::event::{
    PromptAssembled, PromptStrategySwitched, StrategyStateUpdated,
};
// Re-export infrastructure types only. Domain structs are imported from their modules.
pub use crate::common::actor::event_msg::EventMsg;
use crate::feat::plugin_actor::protocol::event::PluginEvent;
use crate::feat::preferences_actor::protocol::event::PreferencesUpdated;
use crate::feat::provider::protocol::event::{
    ModelsRefreshed, PromptTemplatesLoaded, ProviderSwitched, StreamCompleted, StreamToken,
};
use crate::feat::skills::skills_scan_actor::SkillsLoaded;
use crate::feat::tools_actor::protocol::event::{
    ToolBatchCompleted, ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted,
    ToolUseStarted, ToolsRegistered,
};
use crate::init::EnvironmentLoaded;
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
pub enum Event {
    /// A key was pressed down.
    KeyDown(KeyDown),
    /// A key was released.
    KeyUp(KeyUp),
    /// A chat entry was added to the conversation history.
    ChatEntrySubmitted(ChatEntrySubmitted),
    /// The application mode changed.
    ModeChanged(ModeChanged),
    /// An actor is starting up.
    ActorStarting(ActorStarting),
    /// An actor has finished starting up.
    ActorStarted(ActorStarted),
    /// An actor has completed shutdown.
    ActorShutdownCompleted(ActorShutdownCompleted),
    /// All actors have been spawned.
    AllActorsSpawned(AllActorsSpawned),
    /// A streaming LLM response completed.
    StreamCompleted(StreamCompleted),
    /// A single token from a streaming LLM response.
    StreamToken(StreamToken),
    /// A tool call has started in the LLM stream.
    ToolUseStarted(ToolUseStarted),
    /// A complete tool call was received from the LLM stream.
    ToolCallReceived(ToolCallReceived),
    /// Streaming update for a tool call's arguments being assembled.
    ToolCallStreaming(ToolCallStreaming),
    /// The active provider was switched.
    ProviderSwitched(ProviderSwitched),
    /// Models refresh completed.
    ModelsRefreshed(ModelsRefreshed),
    /// Prompt templates loaded after a rescan.
    PromptTemplatesLoaded(PromptTemplatesLoaded),
    /// All tool calls in a batch have completed execution.
    ToolBatchCompleted(ToolBatchCompleted),
    /// A single tool execution completed.
    ToolExecutionCompleted(ToolExecutionCompleted),
    /// Tools were registered by an actor.
    ToolsRegistered(ToolsRegistered),
    /// A prompt has been assembled and is ready to send.
    PromptAssembled(PromptAssembled),
    /// A session's prompt assembly strategy has been switched.
    PromptStrategySwitched(PromptStrategySwitched),
    /// A strategy's session state has changed and should be persisted.
    StrategyStateUpdated(StrategyStateUpdated),
    /// Agent skills have been scanned and loaded.
    SkillsLoaded(SkillsLoaded),
    /// Personas have been scanned and loaded from disk.
    PersonasLoaded(crate::feat::context::protocol::event::PersonasLoaded),
    /// Environment variables and API keys have been loaded.
    EnvironmentLoaded(EnvironmentLoaded),
    /// User preferences have been updated and persisted.
    PreferencesUpdated(PreferencesUpdated),
    /// An event emitted by a plugin.
    Plugin(PluginEvent),
    /// The active session changed (tab switch or session load).
    ActiveSessionChanged(crate::protocol::system::ActiveSessionChanged),
}

impl Event {
    /// Returns the subscription-relevant type name for event routing.
    #[must_use]
    pub fn type_name(&self) -> Option<&str> {
        match self {
            Self::ChatEntrySubmitted(..) => Some(ChatEntrySubmitted::TYPE_NAME),
            Self::ActorStarting(..) => Some(ActorStarting::TYPE_NAME),
            Self::ActorStarted(..) => Some(ActorStarted::TYPE_NAME),
            Self::ActorShutdownCompleted(..) => Some(ActorShutdownCompleted::TYPE_NAME),
            Self::AllActorsSpawned(..) => Some(AllActorsSpawned::TYPE_NAME),
            Self::KeyDown(..) => Some(KeyDown::TYPE_NAME),
            Self::KeyUp(..) => Some(KeyUp::TYPE_NAME),
            Self::ModeChanged(..) => Some(ModeChanged::TYPE_NAME),
            Self::StreamCompleted(..) => Some(StreamCompleted::TYPE_NAME),
            Self::StreamToken(..) => Some(StreamToken::TYPE_NAME),
            Self::ToolUseStarted(..) => Some(ToolUseStarted::TYPE_NAME),
            Self::ToolCallReceived(..) => Some(ToolCallReceived::TYPE_NAME),
            Self::ToolCallStreaming(..) => Some(ToolCallStreaming::TYPE_NAME),
            Self::ProviderSwitched(..) => Some(ProviderSwitched::TYPE_NAME),
            Self::ModelsRefreshed(..) => Some(ModelsRefreshed::TYPE_NAME),
            Self::PromptTemplatesLoaded(..) => Some(PromptTemplatesLoaded::TYPE_NAME),
            Self::ToolBatchCompleted(..) => Some(ToolBatchCompleted::TYPE_NAME),
            Self::ToolExecutionCompleted(..) => Some(ToolExecutionCompleted::TYPE_NAME),
            Self::ToolsRegistered(..) => Some(ToolsRegistered::TYPE_NAME),
            Self::PromptAssembled(..) => Some(PromptAssembled::TYPE_NAME),
            Self::PromptStrategySwitched(..) => Some(PromptStrategySwitched::TYPE_NAME),
            Self::StrategyStateUpdated(..) => Some(StrategyStateUpdated::TYPE_NAME),
            Self::SkillsLoaded(..) => Some(SkillsLoaded::TYPE_NAME),
            Self::PersonasLoaded(..) => {
                Some(crate::feat::context::protocol::event::PersonasLoaded::TYPE_NAME)
            }
            Self::EnvironmentLoaded(..) => Some(EnvironmentLoaded::TYPE_NAME),
            Self::PreferencesUpdated(..) => Some(PreferencesUpdated::TYPE_NAME),
            Self::Plugin(..) => Some(PluginEvent::TYPE_NAME),
            Self::ActiveSessionChanged(..) => {
                Some(crate::protocol::system::ActiveSessionChanged::TYPE_NAME)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ChatEntry, Key, KeyEvent, Mode, Modifiers, SessionId};

    #[rstest::rstest]
    fn event_chat_entry_submitted_preserves_entry() {
        // Given a ChatEntrySubmitted event with a user entry.
        let entry = ChatEntry::user("hello");
        let event = Event::ChatEntrySubmitted(ChatEntrySubmitted {
            session_id: SessionId::new(),
            entry,
        });

        // When serialized and deserialized.
        let json = serde_json::to_string(&event).expect("serialize");
        let back: Event = serde_json::from_str(&json).expect("deserialize");

        // Then entry text is preserved.
        match back {
            Event::ChatEntrySubmitted(payload) => {
                assert_eq!(
                    payload.entry.kind,
                    crate::ChatEntryKind::User("hello".to_owned())
                );
            }
            other => panic!("expected ChatEntrySubmitted, got {other:?}"),
        }
    }

    /// Checks that `Event::type_name()` delegates to the correct payload `TYPE_NAME`
    /// for all event variants that have a meaningful `type_name`.
    #[rstest::rstest]
    #[case::chat_submitted(
        Event::ChatEntrySubmitted(ChatEntrySubmitted { session_id: SessionId::new(), entry: ChatEntry::user("test") }),
        ChatEntrySubmitted::TYPE_NAME
    )]
    #[case::actor_starting(
        Event::ActorStarting(ActorStarting { name: "actor-a".into(), description: None }),
        ActorStarting::TYPE_NAME
    )]
    #[case::actor_started(
        Event::ActorStarted(ActorStarted { name: "actor-a".into(), description: None }),
        ActorStarted::TYPE_NAME
    )]
    #[case::actor_shutdown_completed(
        Event::ActorShutdownCompleted(ActorShutdownCompleted { name: "actor-a".into() }),
        ActorShutdownCompleted::TYPE_NAME
    )]
    #[case::all_actors_spawned(
        Event::AllActorsSpawned(AllActorsSpawned),
        AllActorsSpawned::TYPE_NAME
    )]
    #[case::key_down(
        Event::KeyDown(KeyDown { key: KeyEvent { key: Key::Enter, modifiers: Modifiers::none() } }),
        KeyDown::TYPE_NAME
    )]
    #[case::key_up(
        Event::KeyUp(KeyUp { key: KeyEvent { key: Key::Char('a'), modifiers: Modifiers::none() } }),
        KeyUp::TYPE_NAME
    )]
    #[case::mode_changed(
        Event::ModeChanged(ModeChanged { from: Mode::Normal, to: Mode::Input }),
        ModeChanged::TYPE_NAME
    )]
    #[case::prompt_templates_loaded(
        Event::PromptTemplatesLoaded(PromptTemplatesLoaded { templates: vec![], error: None }),
        PromptTemplatesLoaded::TYPE_NAME
    )]
    fn event_type_name_returns_payload_type_name(#[case] event: Event, #[case] expected: &str) {
        // Given an Event variant with a payload.
        // When calling type_name().
        // Then it returns Some of the payload's TYPE_NAME.
        assert_eq!(event.type_name(), Some(expected));
    }
}
