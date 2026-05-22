//! The domain layer — protocol types, actors, intents, and UI elements.
//!
//! This crate consolidates all domain types and logic into a single crate:
//!
//! - **Protocol types** (`protocol/`) — re-exports from `nullslop-protocol`:
//!   Command/Event mega-enums, foundational value types (ChatEntry, SessionId,
//!   Key, Mode, etc.)
//! - **Component UI** (`component_ui/`) — UiElement trait and registry
//! - **Domain slices** — actors, intents, UI elements, and state for each
//!   domain (provider, session, context, tools, etc.)
//!
//! Protocol types are re-exported at the crate root for convenience.
//! `nullslop_domain::Command` is the same type as `crate::protocol::Command`.

pub mod common;
pub mod feat;
pub mod init;

// Not yet reorganized (handled in later phases)
pub mod protocol;
pub mod schema;

// Re-export actor framework types
pub use common::actor::{
    Actor, ActorContext, ActorCounter, ActorEnvelope, ActorRef, ActorSendError, MessageSink,
    NoDirectMsg, RecordingSink, SendResult, SystemMessage,
};
pub use common::actor_host::{
    ActorHost, ActorHostService, ActorSpawnResult, FakeActorHost, InMemoryActorHost, RoutingEntry,
    ShutdownTracker, spawn, spawn_actor_impl, system_spawn,
};

// Re-export component types (state, UI)
pub use common::app_paths::AppPaths;
pub use common::app_state::pin_sort_key;
pub use common::app_state::{
    AppState, ContextAssemblyState, FocusScope, FrontendState, ProviderState, ScopeStack,
    SessionState,
};
pub use common::state::{State, StateReadGuard, StateWriteGuard};
pub use common::tui_signals::TuiSignals;
pub use common::{AppUiRegistry, register_all_ui_elements};
pub use feat::context::prompt_template::PromptTemplateStore;
pub use feat::provider_infra::NO_PROVIDER_ID;

// Re-export services types
pub use common::services::Services;
pub use common::services::test_services::TestServices;

// Re-export core types
pub use common::core::{
    ActorMessageSink, AppCore, SHUTDOWN_TIMEOUT, STARTUP_TIMEOUT, coordinated_shutdown,
    spawn_forwarding_task, wait_for_system_ready,
};

// Re-export intent types
pub use common::services::ActorChannelService;
pub use feat::intent::IntentHandler;

// Re-export providers types
pub use feat::provider_infra::TOOL_LOOP_TRIGGER;
pub use feat::provider_infra::cache_path;
pub use feat::provider_infra::{
    ApiKeys, ApiKeysService, ConfigStorageService, FakeLlmServiceFactory, FilesystemConfigStorage,
    InMemoryConfigStorage, LlmServiceFactoryService, ModelCache, NoProvidersAvailableFactory,
    ProviderId, ProviderRegistry, ProviderRegistryService, ProvidersConfig,
};

// Re-export context types

// Re-export session types
pub use feat::session::PoolConfig;
pub use feat::session::SessionStoreService;
pub use feat::session::SqliteSessionStore;
pub use feat::session::chat_session::SessionPhase;
pub use feat::session::no_api_keys_msg;
pub use feat::session::welcome_msg;

// Re-export preferences types
pub use feat::preferences_actor::ContextTokenBudgetConfig;
pub use feat::preferences_actor::FilesystemUserPreferencesStorage;
pub use feat::preferences_actor::InMemoryUserPreferencesStorage;
pub use feat::preferences_actor::RequestRetryConfig;
pub use feat::preferences_actor::UserPreferences;
pub use feat::preferences_actor::UserPreferencesStorageService;
pub use feat::preferences_actor::protocol::command::{PreferenceUpdate, UpdatePreferences};
pub use feat::preferences_actor::protocol::event::PreferencesUpdated;

// Re-export persona types
pub use feat::persona::{Persona, PersonaEntry};

// Re-export services submodules
pub use common::services::actor_channel;

// Re-export protocol types at crate root
pub use protocol::entries_to_messages;
pub use protocol::{
    ActorName, AppMsg, ChatEntry, ChatEntryId, ChatEntryKind, Command, Event, Intent, IntentResult,
    Key, KeyEvent, Mode, Modifiers, PickerKind, PinPosition, PromptTemplate,
};
pub use protocol::{CommandMsg, CommandName, EventMsg, EventTypeName, PickerEntry};

// Re-export domain types from their canonical locations
pub use common::actor::protocol::command::ProceedWithShutdown;
pub use common::actor::protocol::event::{
    ActorShutdownCompleted, ActorStarted, ActorStarting, AllActorsSpawned,
};
pub use feat::chat_input::protocol::command::{
    EnqueueUserMessage, PushChatEntry, SetChatInputText,
};
pub use feat::chat_input::protocol::event::ChatEntrySubmitted;
pub use feat::context::protocol::command::{AssemblePrompt, PinChatEntry, UnpinChatEntry};
pub use feat::context::protocol::event::PromptAssembled;
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
pub use feat::tools_actor::protocol::command::{ExecuteTool, ExecuteToolBatch, RegisterTools};
pub use feat::tools_actor::protocol::event::{
    ToolBatchCompleted, ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted,
    ToolExecutionOutput, ToolExecutionStarted, ToolUseStarted, ToolsRegistered,
};
pub use feat::tools_actor::tool_types::{ToolCall, ToolDefinition, ToolResult};
pub use feat::tools_actor::BoxedToolFuture;
pub use feat::tools_actor::builtin::{builtin_tools, BuiltinToolEntry};
