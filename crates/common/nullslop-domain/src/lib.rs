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

pub mod actor;
pub mod actor_host;
pub mod char_counter;
pub mod chat_entry_selection;
pub mod chat_input_box;
pub mod chat_log;
pub mod chat_session;
pub mod component;
pub mod component_ui;
pub mod context;
pub mod dashboard;
pub mod echo;
pub mod global;
pub mod llm;
pub mod navigation;
pub mod picker;
pub mod pinned_panel;
pub mod prompt_template;
pub mod protocol;
pub mod provider;
pub mod providers;
pub mod services;
pub mod session;
pub mod shutdown;
pub mod status_bar;
pub mod tools;

// Re-export actor framework types
pub use actor::{
    Actor, ActorContext, ActorEnvelope, ActorRef, ActorSendError, MessageSink, RecordingSink,
    SendResult, SystemMessage,
};
pub use actor_host::{
    ActorHost, ActorHostService, ActorSpawnResult, FakeActorHost, InMemoryActorHost, RoutingEntry,
    spawn_actor,
};

// Re-export component types (state, UI)
pub use component::app_state::pin_sort_key;
pub use component::{
    AppState, AppUiRegistry, ChatInputBoxState, ChatSessionState, DashboardState, FrontendState,
    PinnedPanelState, ProviderState, ShutdownCoordinatorState, ShutdownTrackerState, State,
    StateReadGuard, StateWriteGuard, TuiSignals,
};
pub use component::{register_all, register_tui_elements};
pub use prompt_template::PromptTemplateStore;
pub use providers::NO_PROVIDER_ID;

// Re-export services types
pub use services::Services;
pub use services::test_services::TestServices;
pub use services::{ActorChannelService, CoreChannelService};

// Re-export providers types
pub use providers::TOOL_LOOP_TRIGGER;
pub use providers::cache_path;
pub use providers::{
    ApiKeys, ApiKeysService, ConfigStorageService, FakeLlmServiceFactory, FilesystemConfigStorage,
    InMemoryConfigStorage, LlmServiceFactoryService, ModelCache, NoProvidersAvailableFactory,
    ProviderRegistry, ProviderRegistryService, ProvidersConfig,
};

// Re-export context types
pub use context::DefaultStrategyDiscovery;
pub use context::DefaultStrategyFactory;

// Re-export session types
pub use session::JsonlSessionStore;
pub use session::SessionStoreService;

// Re-export prompt template utilities
pub use prompt_template::{ensure_prompts_dir_with_example, prompts_dir};

// Re-export services submodules
pub use services::actor_channel;
pub use services::core_channel;
pub use services::strategy_registry;

// Re-export protocol types at crate root — these are the same types as nullslop_protocol
pub use protocol::entries_to_messages;
pub use protocol::{ActiveTab, TabDirection};
pub use protocol::{
    ActorName, AppMsg, ChatEntry, ChatEntryId, ChatEntryKind, Command, CommandAction,
    CoreNotification, Event, Intent, IntentResult, Key, KeyEvent, LlmMessage, Mode, Modifiers,
    PickerKind, PinPosition, PromptTemplate, SessionId, StrategyEntry, SwitchPromptStrategy,
};
pub use protocol::{
    ActorShutdownCompleted, ActorStarted, ActorStarting, AssemblePrompt, CommandMsg, CommandName,
    EventMsg, EventTypeName, KeymapEntry, PickerEntry, PromptAssembled, PromptStrategyId,
    PromptStrategySwitched, SessionEntry, SessionLoadCompleted, SessionLoadRequested, SessionNew,
    SessionSaveRequested,
};
pub use protocol::{
    ExecuteTool, ExecuteToolBatch, PushToolResult, RegisterTools, ToolBatchCompleted, ToolCall,
    ToolCallReceived, ToolCallStreaming, ToolDefinition, ToolExecutionCompleted, ToolResult,
    ToolUseStarted, ToolsRegistered,
};
