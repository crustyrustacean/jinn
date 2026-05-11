//! Protocol types for the nullslop actor system.
//!
//! This module contains truly shared types that cross feature boundaries:
//! the `Command` and `Event` mega-enums, `Intent`, `Key`, `Mode`, and
//! other cross-cutting types. Domain-specific types have been moved to
//! their feature modules under `feat/` and are re-exported here for
//! backward compatibility.

pub mod action;
pub mod actor;
pub mod actor_name;
pub mod app_msg;
pub mod chat_input;
pub mod command;
pub mod context;
pub mod core_notification;
pub mod custom;
pub mod event;
pub mod intent;
pub mod intent_result;
pub mod key;
pub mod mode;
pub mod picker_kind;
pub mod prompt_template;
pub mod provider;
pub mod session;
pub mod system;
pub mod tab;
pub mod tool;

// Re-export primary types
pub use action::CommandAction;
pub use actor::{ActorShutdownCompleted, ActorStarted, ActorStarting};
pub use actor_name::ActorName;
pub use app_msg::AppMsg;
pub use crate::feat::session::chat_entry::{ChatEntry, ChatEntryId, ChatEntryKind, PinPosition};
pub use command::Command;
pub use context::{
    AssemblePrompt, PromptAssembled, PromptStrategyId, PromptStrategySwitched, SwitchPromptStrategy,
};
pub use crate::feat::picker::strategy_entry::StrategyEntry;
pub use core_notification::CoreNotification;
pub use custom::{CommandMsg, CommandName, EventMsg, EventTypeName};
pub use event::Event;
pub use intent::Intent;
pub use intent_result::IntentResult;
pub use key::{Key, KeyEvent, Modifiers};
pub use crate::feat::picker::keymap_entry::KeymapEntry;
pub use mode::Mode;
pub use nullslop_protocol_derive::{CommandMsg, EventMsg};
pub use picker_kind::PickerKind;
pub use prompt_template::PromptTemplate;
pub use crate::feat::provider::llm_message::LlmMessage;
pub use crate::feat::provider::entries_to_messages::entries_to_messages;
pub use crate::feat::provider::picker_entry::PickerEntry;
pub use session::SessionId;
pub use session::SessionLoadCompleted;
pub use session::SessionLoadRequested;
pub use session::SessionNew;
pub use session::SessionSaveRequested;
pub use crate::feat::session::picker_entry::SessionEntry;
pub use tab::ActiveTab;
pub use tab::TabDirection;
pub use tool::{
    ExecuteTool, ExecuteToolBatch, PushToolResult, RegisterTools, ToolBatchCompleted, ToolCall,
    ToolCallReceived, ToolCallStreaming, ToolDefinition, ToolExecutionCompleted, ToolResult,
    ToolUseStarted, ToolsRegistered,
};
