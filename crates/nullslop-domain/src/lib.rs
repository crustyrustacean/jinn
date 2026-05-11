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
pub mod context;
pub mod intent;
pub mod prompt_template;
pub mod protocol;
pub mod session;

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
pub use common::{register_all, register_tui_elements, AppUiRegistry};
pub use prompt_template::PromptTemplateStore;
pub use feat::provider_infra::NO_PROVIDER_ID;

// Re-export services types
pub use common::services::Services;
pub use common::services::test_services::TestServices;

// Re-export core types
pub use common::core::{
    ActorMessageSink, AppCore, SHUTDOWN_TIMEOUT, coordinated_shutdown, spawn_forwarding_task,
};

// Re-export intent types
pub use intent::IntentHandler;
pub use common::services::{ActorChannelService, CoreChannelService};

// Re-export providers types
pub use feat::provider_infra::TOOL_LOOP_TRIGGER;
pub use feat::provider_infra::cache_path;
pub use feat::provider_infra::{
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
pub use common::services::actor_channel;
pub use common::services::core_channel;
pub use common::services::strategy_registry;

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
