//! Event types for the component event pipeline.
//!
//! The [`Event`] enum is the unified type the host broadcasts to
//! inform internal handlers and actors about state changes and input.
//!
//! Individual event structs live in domain modules (`chat_input`, `system`,
//! `custom`, `actor`). Consumers import structs directly from those modules -
//! this facade only re-exports infrastructure types.
//!
//! # When adding a new event
//!
//! Every new event struct **must** be added as a variant on the [`Event`] enum
//! below. Creating the struct alone is not enough - the bus broadcasts based on
//! enum variants, so a missing variant means the event is invisible to the system.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

// Internal imports for enum definition, type_name(), and tests.
pub use crate::common::actor::protocol::dynamic_event::DynamicEvent;
use crate::common::actor::protocol::event::{
    ActorShutdownCompleted, ActorStarted, ActorStarting, AllActorsSpawned,
};
use crate::feat::chat_input::protocol::event::ChatEntrySubmitted;
// Re-export infrastructure types only. Domain structs are imported from their modules.
pub use crate::common::actor::event_msg::EventMsg;
use crate::feat::preferences_actor::protocol::event::PreferencesUpdated;
use crate::feat::provider::protocol::event::{
    ModelCacheLoaded, ModelsRefreshed, PromptTemplatesLoaded, ProviderSwitched, StreamCompleted,
    StreamToken,
};
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use crate::feat::session_lifecycle::protocol::event::{
    SessionCreated, SessionSetupCompleted, SessionTeardownFinished,
};
use crate::feat::skills::skills_scan_actor::SkillsLoaded;
use crate::feat::tools_actor::protocol::event::{
    ToolBatchCompleted, ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted,
    ToolExecutionOutput, ToolExecutionStarted, ToolUseStarted, ToolsRegistered,
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
#[allow(clippy::large_enum_variant)]
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
    /// Model cache loaded from disk at startup.
    ModelCacheLoaded(ModelCacheLoaded),
    /// Prompt templates loaded after a rescan.
    PromptTemplatesLoaded(PromptTemplatesLoaded),
    /// All tool calls in a batch have completed execution.
    ToolBatchCompleted(ToolBatchCompleted),
    /// A single tool execution completed.
    ToolExecutionCompleted(ToolExecutionCompleted),
    /// A tool has started executing (streaming tools only).
    ToolExecutionStarted(ToolExecutionStarted),
    /// Incremental output from a running tool.
    ToolExecutionOutput(ToolExecutionOutput),
    /// Tools were registered by an actor.
    ToolsRegistered(ToolsRegistered),
    /// Agent skills have been scanned and loaded.
    SkillsLoaded(SkillsLoaded),
    /// Project context files (AGENTS.md/CLAUDE.md) have been scanned and loaded.
    ContextFilesLoaded(crate::feat::context::protocol::event::ContextFilesLoaded),
    /// Personas have been scanned and loaded from disk.
    PersonasLoaded(crate::feat::context::protocol::event::PersonasLoaded),
    /// A chat entry was pinned or unpinned.
    ChatEntryPinChanged(crate::feat::context::protocol::event::ChatEntryPinChanged),
    /// A chat entry's context override was toggled (included/excluded from LLM context).
    ContextOverrideChanged(crate::feat::context::protocol::event::ContextOverrideChanged),
    /// Environment variables and API keys have been loaded.
    EnvironmentLoaded(EnvironmentLoaded),
    /// User preferences have been updated and persisted.
    PreferencesUpdated(PreferencesUpdated),
    /// The active session changed (tab switch or session load).
    ActiveSessionChanged(crate::protocol::system::ActiveSessionChanged),
    /// A new chat session was created.
    SessionCreated(SessionCreated),
    /// A lifecycle setup command completed.
    SessionSetupCompleted(SessionSetupCompleted),
    /// A lifecycle teardown command finished.
    SessionTeardownFinished(SessionTeardownFinished),
    /// A session was closed and removed from the sessions map.
    SessionClosed(crate::feat::session::protocol::session_closed::SessionClosed),
    /// A session was marked as interacted with by the user.
    UserInteracted(crate::feat::session::protocol::user_interacted::UserInteracted),
    /// A session was archived in persistent storage.
    SessionArchived(crate::feat::session::protocol::session_archived::SessionArchived),
    /// A session's phase changed (e.g., Idle → Sending).
    SessionPhaseChanged(crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged),
    /// A new entry was appended to a session's history.
    HistoryAppended(crate::feat::session::protocol::history_appended::HistoryAppended),
    /// A shared snapshot of session history is ready for workers to evaluate.
    HistorySnapshotReady(
        crate::feat::session::protocol::history_snapshot_ready::HistorySnapshotReady,
    ),
    /// A session has been fully loaded from persistent storage.
    SessionLoadCompleted(Box<SessionLoadCompleted>),

    /// A plugin was attached to a session.
    PluginAttached(crate::feat::plugin_dispatch::protocol::event::PluginAttached),
    /// A plugin was detached from a session.
    PluginDetached(crate::feat::plugin_dispatch::protocol::event::PluginDetached),
    /// A plugin was toggled on/off.
    PluginToggled(crate::feat::plugin_dispatch::protocol::event::PluginToggled),

    /// A task list was updated by a mutation tool.
    TaskListUpdated(crate::feat::session::protocol::task_list_updated::TaskListUpdated),
    /// A dynamic event from a plugin, carrying an arbitrary JSON payload.
    ///
    /// Broadcast by the runtime [`name`](DynamicEvent::name) field, not the
    /// static `EventMsg::TYPE_NAME`. If no actor subscribes to that name, the
    /// event is silently dropped.
    Dynamic(DynamicEvent),
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
            Self::ModelCacheLoaded(..) => Some(ModelCacheLoaded::TYPE_NAME),
            Self::PromptTemplatesLoaded(..) => Some(PromptTemplatesLoaded::TYPE_NAME),
            Self::ToolBatchCompleted(..) => Some(ToolBatchCompleted::TYPE_NAME),
            Self::ToolExecutionCompleted(..) => Some(ToolExecutionCompleted::TYPE_NAME),
            Self::ToolExecutionStarted(..) => Some(ToolExecutionStarted::TYPE_NAME),
            Self::ToolExecutionOutput(..) => Some(ToolExecutionOutput::TYPE_NAME),
            Self::ToolsRegistered(..) => Some(ToolsRegistered::TYPE_NAME),
            Self::SkillsLoaded(..) => Some(SkillsLoaded::TYPE_NAME),
            Self::ContextFilesLoaded(..) => {
                Some(crate::feat::context::protocol::event::ContextFilesLoaded::TYPE_NAME)
            }
            Self::PersonasLoaded(..) => {
                Some(crate::feat::context::protocol::event::PersonasLoaded::TYPE_NAME)
            }
            Self::ChatEntryPinChanged(..) => {
                Some(crate::feat::context::protocol::event::ChatEntryPinChanged::TYPE_NAME)
            }
            Self::ContextOverrideChanged(..) => {
                Some(crate::feat::context::protocol::event::ContextOverrideChanged::TYPE_NAME)
            }
            Self::EnvironmentLoaded(..) => Some(EnvironmentLoaded::TYPE_NAME),
            Self::PreferencesUpdated(..) => Some(PreferencesUpdated::TYPE_NAME),
            Self::ActiveSessionChanged(..) => {
                Some(crate::protocol::system::ActiveSessionChanged::TYPE_NAME)
            }
            Self::SessionCreated(..) => Some(SessionCreated::TYPE_NAME),
            Self::SessionSetupCompleted(..) => Some(SessionSetupCompleted::TYPE_NAME),
            Self::SessionTeardownFinished(..) => Some(SessionTeardownFinished::TYPE_NAME),
            Self::SessionClosed(..) => {
                Some(crate::feat::session::protocol::session_closed::SessionClosed::TYPE_NAME)
            }
            Self::UserInteracted(..) => {
                Some(crate::feat::session::protocol::user_interacted::UserInteracted::TYPE_NAME)
            }
            Self::SessionArchived(..) => {
                Some(crate::feat::session::protocol::session_archived::SessionArchived::TYPE_NAME)
            }
            Self::SessionPhaseChanged(..) => {
                Some(crate::feat::session::protocol::session_phase_changed::SessionPhaseChanged::TYPE_NAME)
            }
            Self::HistoryAppended(..) => {
                Some(crate::feat::session::protocol::history_appended::HistoryAppended::TYPE_NAME)
            }
            Self::HistorySnapshotReady(..) => {
                Some(crate::feat::session::protocol::history_snapshot_ready::HistorySnapshotReady::TYPE_NAME)
            }
            Self::SessionLoadCompleted(..) => Some(SessionLoadCompleted::TYPE_NAME),
            Self::PluginAttached(..) => {
                Some(crate::feat::plugin_dispatch::protocol::event::PluginAttached::TYPE_NAME)
            }
            Self::PluginDetached(..) => {
                Some(crate::feat::plugin_dispatch::protocol::event::PluginDetached::TYPE_NAME)
            }
            Self::PluginToggled(..) => {
                Some(crate::feat::plugin_dispatch::protocol::event::PluginToggled::TYPE_NAME)
            }

            Self::TaskListUpdated(..) => {
                Some(crate::feat::session::protocol::task_list_updated::TaskListUpdated::TYPE_NAME)
            }
            Self::Dynamic(..) => Some(DynamicEvent::TYPE_NAME),
        }
    }

    /// Returns the routing key for bus broadcast.
    ///
    /// Typed variants return their static type name as an owned `Cow`.
    /// `Dynamic` returns the runtime `.name` field as a borrowed `Cow`,
    /// allowing plugins to define arbitrary event names.
    #[must_use]
    pub fn routing_key(&self) -> Option<Cow<'_, str>> {
        match self {
            Self::Dynamic(d) => Some(Cow::Borrowed(&d.name)),
            _ => self.type_name().map(|s| Cow::Owned(s.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
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
                    crate::ChatEntryKind::User {
                        display: "hello".to_owned(),
                        expanded: "hello".to_owned()
                    }
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
    #[case::context_files_loaded(
        Event::ContextFilesLoaded(crate::feat::context::protocol::event::ContextFilesLoaded {
            session_id: SessionId::new(),
            files: vec![],
            error: None,
        }),
        crate::feat::context::protocol::event::ContextFilesLoaded::TYPE_NAME
    )]
    #[case::prompt_templates_loaded(
        Event::PromptTemplatesLoaded(PromptTemplatesLoaded { session_id: SessionId::new(), templates: vec![], error: None }),
        PromptTemplatesLoaded::TYPE_NAME
    )]
    #[case::context_override_changed(
        Event::ContextOverrideChanged(crate::feat::context::protocol::event::ContextOverrideChanged {
            session_id: SessionId::new(),
            entry_id: crate::feat::session::chat_entry::ChatEntryId::new(),
        }),
        crate::feat::context::protocol::event::ContextOverrideChanged::TYPE_NAME
    )]
    fn event_type_name_returns_payload_type_name(#[case] event: Event, #[case] expected: &str) {
        // Given an Event variant with a payload.
        // When calling type_name().
        // Then it returns Some of the payload's TYPE_NAME.
        assert_eq!(event.type_name(), Some(expected));
    }

    #[rstest::rstest]
    fn dynamic_event_routing_key_uses_runtime_name() {
        // Given a Dynamic event with a custom name.
        let event = Event::Dynamic(DynamicEvent {
            name: "plugin::custom_event".to_owned(),
            payload: serde_json::Value::Null,
        });

        // When calling routing_key.
        let key = event.routing_key();

        // Then it returns the runtime name, not the static TYPE_NAME.
        assert_eq!(key.as_deref(), Some("plugin::custom_event"));
        assert_ne!(key.as_deref(), Some(DynamicEvent::TYPE_NAME));
    }

    #[rstest::rstest]
    fn dynamic_event_type_name_returns_static() {
        // Given a Dynamic event.
        let event = Event::Dynamic(DynamicEvent {
            name: "plugin::custom".to_owned(),
            payload: serde_json::Value::Null,
        });

        // When calling type_name.
        // Then it returns the static TYPE_NAME.
        assert_eq!(event.type_name(), Some(DynamicEvent::TYPE_NAME));
    }

    #[rstest::rstest]
    fn context_override_changed_serialization_round_trip() {
        // Given a ContextOverrideChanged event.
        let entry_id = crate::feat::session::chat_entry::ChatEntryId::new();
        let event = Event::ContextOverrideChanged(
            crate::feat::context::protocol::event::ContextOverrideChanged {
                session_id: SessionId::new(),
                entry_id: entry_id.clone(),
            },
        );

        // When serialized and deserialized.
        let json = serde_json::to_string(&event).expect("serialize");
        let back: Event = serde_json::from_str(&json).expect("deserialize");

        // Then the entry_id is preserved.
        match back {
            Event::ContextOverrideChanged(payload) => {
                assert_eq!(payload.entry_id, entry_id);
            }
            other => panic!("expected ContextOverrideChanged, got {other:?}"),
        }
    }
}
