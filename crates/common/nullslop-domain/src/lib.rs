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
//! `nullslop_domain::Command` is the same type as `nullslop_protocol::Command`.

pub mod char_counter;
pub mod chat_entry_selection;
pub mod chat_input_box;
pub mod chat_log;
pub mod chat_session;
pub mod component_ui;
pub mod context;
pub mod dashboard;
pub mod echo;
pub mod global;
pub mod llm;
pub mod navigation;
pub mod picker;
pub mod pinned_panel;
pub mod protocol;
pub mod provider;
pub mod session;
pub mod shutdown;
pub mod status_bar;
pub mod tools;

// Re-export protocol types at crate root — these are the same types as nullslop_protocol
pub use nullslop_protocol::entries_to_messages;
pub use nullslop_protocol::{ActiveTab, TabDirection};
pub use nullslop_protocol::{
    ActorName, AppMsg, ChatEntry, ChatEntryId, ChatEntryKind, Command, CommandAction,
    CoreNotification, Event, Intent, IntentResult, Key, KeyEvent, LlmMessage, Mode, Modifiers,
    PickerKind, PinPosition, PromptTemplate, SessionId, StrategyEntry, SwitchPromptStrategy,
};
pub use nullslop_protocol::{
    ActorShutdownCompleted, ActorStarted, ActorStarting, AssemblePrompt, CommandMsg, CommandName,
    EventMsg, EventTypeName, KeymapEntry, PickerEntry, PromptAssembled, PromptStrategyId,
    PromptStrategySwitched, SessionEntry, SessionLoadCompleted, SessionLoadRequested, SessionNew,
    SessionSaveRequested,
};
pub use nullslop_protocol::{
    ExecuteTool, ExecuteToolBatch, PushToolResult, RegisterTools, ToolBatchCompleted, ToolCall,
    ToolCallReceived, ToolCallStreaming, ToolDefinition, ToolExecutionCompleted, ToolResult,
    ToolUseStarted, ToolsRegistered,
};
