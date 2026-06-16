//! The domain layer - protocol types, actors, intents, and UI elements.
//!
//! This crate consolidates all domain types and logic into a single crate:
//!
//! - **Protocol types** (`protocol/`) - re-exports from `jinn-protocol`:
//!   Command/Event mega-enums, foundational value types (ChatEntry, SessionId,
//!   Key, Mode, etc.)
//! - **Component UI** (`component_ui/`) - UiElement trait and registry
//! - **Domain slices** - actors, intents, UI elements, and state for each
//!   domain (provider, session, context, tools, etc.)
//!
//! Protocol types are re-exported at the crate root for convenience.
//! `jinn_domain::Command` is the same type as `crate::protocol::Command`.

pub mod common;
pub mod feat;
pub mod init;

// Not yet reorganized (handled in later phases)
pub mod protocol;

// Re-export actor types that are still in use
pub use common::actor::{ActorCounter, NoDirectMsg};
// Re-export component types (state, UI)
pub use common::app_paths::AppPaths;
pub use common::app_state::pin_sort_key;
pub use common::app_state::{
    AppState, ContextAssemblyState, FocusScope, FrontendState, ProviderState, ScopeStack,
    SessionState,
};
pub use common::bridge::{Bridge, BridgeClosure};
pub use common::bus::BusMessage;
pub use common::render_ctx::RenderCtx;
pub use common::state::{State, StateReadGuard, StateWriteGuard};
pub use common::tui_signals::TuiSignals;
pub use common::{AppUiRegistry, register_all_ui_elements};
pub use feat::context::prompt_template::PromptTemplateStore;
pub use feat::plugin_dispatch::{
    BadgeDirective, BadgeSegment, InterceptOutcome, PluginSyncHooks, call_hooks_typed,
};
pub use feat::provider_infra::NO_PROVIDER_ID;

// Re-export services types
pub use common::services::NoopPluginFire;
pub use common::services::NoopPluginSyncCall;
pub use common::services::NoopSessionPluginRegistry;
pub use common::services::Services;
pub use common::services::bus_service::BusService;
pub use common::services::test_services::TestServices;

// Re-export core types
pub use common::core::{AppCore, SHUTDOWN_TIMEOUT, STARTUP_TIMEOUT, wait_for_system_ready};

// Re-export intent types
pub use feat::intent::IntentHandler;

// Re-export providers types
pub use feat::provider_infra::TOOL_LOOP_TRIGGER;
pub use feat::provider_infra::cache_path;
pub use feat::provider_infra::{
    ApiKeys, ApiKeysService, ConfigStorageService, FakeLlmServiceFactory, FilesystemConfigStorage,
    InMemoryConfigStorage, LlmServiceFactoryService, ModelCache, NoProvidersAvailableFactory,
    ProviderId, ProviderRegistry, ProviderRegistryService, ProvidersConfig, ScriptedResponse,
};
// Re-export context types

// Re-export session types
pub use feat::session::PoolConfig;
pub use feat::session::SessionStoreService;
pub use feat::session::SqliteSessionStore;

pub use feat::session::no_api_keys_msg;
pub use feat::session::phase_machine::PhaseKind;

// Re-export preferences types
pub use feat::preferences_actor::AppStateStorageService;
pub use feat::preferences_actor::FilesystemAppStateStorage;
pub use feat::preferences_actor::FilesystemUserPreferencesStorage;
pub use feat::preferences_actor::InMemoryAppStateStorage;
pub use feat::preferences_actor::InMemoryUserPreferencesStorage;
pub use feat::preferences_actor::RequestRetryConfig;
pub use feat::preferences_actor::UserPreferences;
pub use feat::preferences_actor::UserPreferencesStorageService;
pub use feat::preferences_actor::protocol::command::{PreferenceUpdate, UpdatePreferences};
pub use feat::preferences_actor::protocol::event::PreferencesUpdated;
pub use feat::preferences_actor::{InitOutcome, init_default_config_to, preferences_path};

// Re-export persona types
pub use feat::persona::{Persona, PersonaEntry};

// Re-export services submodules

// Re-export protocol types at crate root
pub use common::actor::protocol::dynamic_command::DynamicCommand;
pub use common::actor::protocol::dynamic_event::DynamicEvent;
pub use protocol::PickerEntry;
pub use protocol::entries_to_messages;
pub use protocol::{
    ChatEntry, ChatEntryId, ChatEntryKind, Intent, IntentResult, Key, KeyEvent, Mode, Modifiers,
    PickerKind, PinPosition, PromptTemplate,
};

// Re-export domain types from their canonical locations
pub use common::actor::protocol::command::ProceedWithShutdown;
pub use common::actor::protocol::event::{
    ActorShutdownCompleted, ActorStarted, ActorStarting, AllActorsSpawned,
};
pub use feat::attached_plugin::PluginInstanceId;
pub use feat::chat_input::protocol::command::{
    EnqueueUserMessage, PushChatEntry, SetChatInputEnabled, SetChatInputText,
};
pub use feat::chat_input::protocol::event::ChatEntrySubmitted;
pub use feat::context::assemble::AssembledPrompt;
pub use feat::context::protocol::command::{PinChatEntry, UnpinChatEntry};
pub use feat::provider::llm_message::LlmMessage;
pub use feat::provider::protocol::command::{
    CancelStream, ProviderSwitch, RefreshModels, RescanPromptTemplates, SendMessage,
    SendToLlmProvider,
};
pub use feat::provider::protocol::event::{
    ModelCacheLoaded, ModelsRefreshed, PromptTemplatesLoaded, ProviderSwitched, StreamCompleted,
    StreamCompletedReason, StreamToken,
};
pub use feat::session::protocol::session_fork_requested::SessionForkRequested;
pub use feat::session::protocol::session_id::SessionId;
pub use feat::session::protocol::session_load_completed::SessionLoadCompleted;
pub use feat::session::protocol::session_load_requested::SessionLoadRequested;
pub use feat::session::protocol::session_new::SessionNew;
pub use feat::tools_actor::BoxedToolFuture;
pub use feat::tools_actor::protocol::command::{ExecuteTool, ExecuteToolBatch, RegisterTools};
pub use feat::tools_actor::protocol::event::{
    ToolBatchCompleted, ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted,
    ToolExecutionOutput, ToolExecutionStarted, ToolUseStarted, ToolsRegistered,
};
pub use feat::tools_actor::registry::{BuiltinToolEntry, builtin_tools};
pub use feat::tools_actor::tool_types::{ToolCall, ToolDefinition, ToolResult};
