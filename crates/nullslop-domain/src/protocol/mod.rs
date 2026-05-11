//! Protocol types for the nullslop actor system.
//!
//! This module contains truly shared types that cross feature boundaries:
//! the `Command` and `Event` mega-enums, `Intent`, `Key`, `Mode`, and
//! other cross-cutting types. Domain-specific types live in their feature
//! modules under `feat/` and are re-exported from `lib.rs` for convenience.

pub mod actor_name;
pub mod app_msg;
pub mod command;
pub mod core_notification;
pub mod custom;
pub mod event;
pub mod intent;
pub mod intent_result;
pub mod key;
pub mod mode;
pub mod picker_kind;
pub mod prompt_template;
pub mod system;
pub mod tab;

// Re-export primary types defined in this module
pub use actor_name::ActorName;
pub use app_msg::AppMsg;
pub use command::Command;
pub use core_notification::CoreNotification;
pub use custom::{CommandMsg, CommandName, EventMsg, EventTypeName};
pub use event::Event;
pub use intent::Intent;
pub use intent_result::IntentResult;
pub use key::{Key, KeyEvent, Modifiers};
pub use mode::Mode;
pub use nullslop_protocol_derive::{CommandMsg, EventMsg};
pub use picker_kind::PickerKind;
pub use prompt_template::PromptTemplate;
pub use tab::ActiveTab;
pub use tab::TabDirection;

// Re-export domain types that are widely used as cross-cutting protocol concerns
pub use crate::feat::context::protocol::strategy_id::PromptStrategyId;
pub use crate::feat::context::protocol::command::SwitchPromptStrategy;
pub use crate::feat::provider::llm_message::LlmMessage;
pub use crate::feat::session::protocol::session_id::SessionId;

// Re-export domain types used by the picker and UI
pub use crate::feat::picker::keymap_entry::KeymapEntry;
pub use crate::feat::picker::strategy_entry::StrategyEntry;
pub use crate::feat::provider::entries_to_messages::entries_to_messages;
pub use crate::feat::provider::picker_entry::PickerEntry;
pub use crate::feat::session::chat_entry::{ChatEntry, ChatEntryId, ChatEntryKind, PinPosition};
pub use crate::feat::tools::tool_types::{ToolCall, ToolDefinition, ToolResult};
pub use crate::feat::session::picker_entry::SessionEntry;
