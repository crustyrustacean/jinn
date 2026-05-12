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

// Not yet reorganized (handled in later phases)
pub mod protocol;

// Re-export actor framework types
pub use common::actor::{
    Actor, ActorContext, ActorEnvelope, ActorRef, ActorSendError, MessageSink, RecordingSink,
    SendResult, SystemMessage,
};
pub use common::actor_host::{
    ActorHost, ActorHostService, ActorSpawnResult, FakeActorHost, InMemoryActorHost, RoutingEntry,
    spawn_actor,
};

// Re-export component types (state, UI)
pub use common::app_state::pin_sort_key;
pub use common::app_state::{
    AppState, ContextAssemblyState, FrontendState, ProviderState, SessionState,
    ShutdownCoordinatorState,
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
    ActorMessageSink, AppCore, SHUTDOWN_TIMEOUT, coordinated_shutdown, spawn_forwarding_task,
};

// Re-export intent types
pub use common::services::{ActorChannelService, CoreChannelService};
pub use feat::intent::IntentHandler;

// Re-export providers types
pub use feat::provider_infra::TOOL_LOOP_TRIGGER;
pub use feat::provider_infra::cache_path;
pub use feat::provider_infra::{
    ApiKeys, ApiKeysService, ConfigStorageService, FakeLlmServiceFactory, FilesystemConfigStorage,
    InMemoryConfigStorage, LlmServiceFactoryService, ModelCache, NoProvidersAvailableFactory,
    ProviderRegistry, ProviderRegistryService, ProvidersConfig,
};

// Re-export context types
pub use feat::context::DefaultStrategyDiscovery;
pub use feat::context::DefaultStrategyFactory;

// Re-export session types
pub use feat::session::JsonlSessionStore;
pub use feat::session::SessionStoreService;

// Re-export prompt template utilities
pub use feat::context::prompt_template::{ensure_prompts_dir_with_example, prompts_dir};

// Re-export services submodules
pub use common::services::actor_channel;
pub use common::services::core_channel;
pub use common::services::strategy_registry;

// Re-export protocol types at crate root
pub use protocol::entries_to_messages;
pub use protocol::{ActiveTab, TabDirection};
pub use protocol::{
    ActorName, AppMsg, ChatEntry, ChatEntryId, ChatEntryKind, Command, CoreNotification, Event,
    Intent, IntentResult, Key, KeyEvent, Mode, Modifiers, PickerKind, PromptTemplate,
};
pub use protocol::{
    CommandMsg, CommandName, EventMsg, EventTypeName, KeymapEntry, PickerEntry, StrategyEntry,
};

// Re-export domain types from their canonical locations
pub use common::actor::protocol::command::ProceedWithShutdown;
pub use common::actor::protocol::event::{ActorShutdownCompleted, ActorStarted, ActorStarting};
pub use feat::chat_input::protocol::command::{
    EnqueueUserMessage, PushChatEntry, SetChatInputText,
};
pub use feat::chat_input::protocol::event::ChatEntrySubmitted;
pub use feat::context::protocol::command::{
    AssemblePrompt, PinChatEntry, RestoreStrategyState, SwitchPromptStrategy, UnpinChatEntry,
};
pub use feat::context::protocol::event::{
    PromptAssembled, PromptStrategySwitched, StrategyStateUpdated,
};
pub use feat::context::protocol::strategy_id::PromptStrategyId;
pub use feat::provider::llm_message::LlmMessage;
pub use feat::provider::protocol::command::{
    CancelStream, ProviderSwitch, RefreshModels, RescanPromptTemplates, SendMessage,
    SendToLlmProvider,
};
pub use feat::provider::protocol::event::{
    ModelsRefreshed, PromptTemplatesLoaded, ProviderSwitched, StreamCompleted,
    StreamCompletedReason, StreamToken,
};
pub use feat::session::protocol::session_id::SessionId;
pub use feat::session::protocol::session_load_completed::SessionLoadCompleted;
pub use feat::session::protocol::session_load_requested::SessionLoadRequested;
pub use feat::session::protocol::session_new::SessionNew;
pub use feat::session::protocol::session_save_requested::SessionSaveRequested;
pub use feat::tools_actor::protocol::command::{ExecuteTool, ExecuteToolBatch, RegisterTools};
pub use feat::tools_actor::protocol::event::{
    ToolBatchCompleted, ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted,
    ToolUseStarted, ToolsRegistered,
};
pub use feat::tools_actor::tool_types::{ToolCall, ToolDefinition, ToolResult};
