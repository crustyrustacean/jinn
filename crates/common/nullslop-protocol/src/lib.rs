//! Shared protocol types for communication between the nullslop host and actors.
//!
//! This crate defines the common language of commands, events, key representations,
//! interaction modes, and chat data that the host and all actors agree on.
//! Every type here is serializable and travels across the actor boundary.
//!
//! Runtime-mutable state types (`AppState`, `ShutdownTracker`, `ChatInputBoxState`)
//! live in `nullslop-component`.

pub mod action;
pub mod actor;
pub mod actor_name;
pub mod app_msg;
pub mod chat;
pub mod chat_input;
pub mod command;
pub mod context;
pub mod context_strategy_picker;
pub mod core_notification;
pub mod custom;
pub mod event;
pub mod intent;
pub mod intent_result;
pub mod key;
pub mod keymap_picker;
pub mod mode;
pub mod picker_kind;
pub mod prompt_template;
pub mod provider;
pub mod provider_picker;
pub mod session;
pub mod session_picker;
pub mod system;
pub mod tab;
pub mod tool;

// Re-export primary types
pub use action::CommandAction;
pub use actor::{ActorShutdownCompleted, ActorStarted, ActorStarting};
pub use actor_name::ActorName;
pub use app_msg::AppMsg;
pub use chat::{ChatEntry, ChatEntryId, ChatEntryKind, PinPosition};
pub use command::Command;
pub use context_strategy_picker::entries::StrategyEntry;
pub use context::{
    AssemblePrompt, PromptAssembled, PromptStrategyId, PromptStrategySwitched, SwitchPromptStrategy,
};
pub use core_notification::CoreNotification;
pub use custom::{CommandMsg, CommandName, EventMsg, EventTypeName};
pub use event::Event;
pub use intent::Intent;
pub use keymap_picker::entries::KeymapEntry;
pub use intent_result::IntentResult;
pub use key::{Key, KeyEvent, Modifiers};
pub use mode::Mode;
pub use nullslop_protocol_derive::{CommandMsg, EventMsg};
pub use picker_kind::PickerKind;
pub use prompt_template::PromptTemplate;
pub use provider::entries_to_messages;
pub use provider::LlmMessage;
pub use provider_picker::entries::PickerEntry;
pub use session::SessionId;
pub use session::SessionLoadCompleted;
pub use session::SessionLoadRequested;
pub use session::SessionNew;
pub use session::SessionSaveRequested;
pub use session_picker::entries::SessionEntry;
// OpenPicker removed — now an Intent variant.
pub use tab::ActiveTab;
pub use tab::TabDirection;
pub use tool::{
    ExecuteTool, ExecuteToolBatch, PushToolResult, RegisterTools, ToolBatchCompleted, ToolCall,
    ToolCallReceived, ToolCallStreaming, ToolDefinition, ToolExecutionCompleted, ToolResult,
    ToolUseStarted, ToolsRegistered,
};
