//! The domain layer — protocol types, actors, intents, and UI elements.
//!
//! This crate consolidates all domain types and logic into a single crate:
//!
//! - **Protocol types** (`protocol/`) — Command/Event mega-enums, foundational
//!   value types (ChatEntry, SessionId, Key, Mode, etc.)
//! - **Component UI** (`component_ui/`) — UiElement trait and registry
//! - **Domain slices** — actors, intents, UI elements, and state for each
//!   domain (provider, session, context, tools, etc.)
//!
//! Protocol types are re-exported at the crate root for convenience.

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

// Re-export protocol types at crate root
pub use protocol::action::CommandAction;
pub use protocol::actor::{ActorShutdownCompleted, ActorStarted, ActorStarting};
pub use protocol::actor_name::ActorName;
pub use protocol::app_msg::AppMsg;
pub use protocol::chat::{ChatEntry, ChatEntryId, ChatEntryKind, PinPosition};
pub use protocol::command::Command;
pub use protocol::context::{
    AssemblePrompt, PromptAssembled, PromptStrategyId, PromptStrategySwitched, SwitchPromptStrategy,
};
pub use protocol::context_strategy_picker::entries::StrategyEntry;
pub use protocol::core_notification::CoreNotification;
pub use protocol::custom::{CommandMsg, CommandName, EventMsg, EventTypeName};
pub use protocol::event::Event;
pub use protocol::intent::Intent;
pub use protocol::intent_result::IntentResult;
pub use protocol::key::{Key, KeyEvent, Modifiers};
pub use protocol::keymap_picker::entries::KeymapEntry;
pub use protocol::mode::Mode;
pub use nullslop_protocol_derive::{CommandMsg, EventMsg};
pub use protocol::picker_kind::PickerKind;
pub use protocol::prompt_template::PromptTemplate;
pub use protocol::provider::LlmMessage;
pub use protocol::provider::entries_to_messages;
pub use protocol::provider_picker::entries::PickerEntry;
pub use protocol::session::{SessionId, SessionLoadCompleted, SessionLoadRequested, SessionNew, SessionSaveRequested};
pub use protocol::session_picker::entries::SessionEntry;
pub use protocol::tab::{ActiveTab, TabDirection};
pub use protocol::tool::{
    ExecuteTool, ExecuteToolBatch, PushToolResult, RegisterTools, ToolBatchCompleted, ToolCall,
    ToolCallReceived, ToolCallStreaming, ToolDefinition, ToolExecutionCompleted, ToolResult,
    ToolUseStarted, ToolsRegistered,
};
