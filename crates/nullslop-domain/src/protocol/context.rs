//! Context domain: prompt assembly commands, events, and strategy identification.
//!
//! Re-exports from `feat::context::protocol`.

pub use crate::feat::context::protocol::command::{
    AssemblePrompt, PinChatEntry, RestoreStrategyState, SwitchPromptStrategy, UnpinChatEntry,
};
pub use crate::feat::context::protocol::event::{
    PromptAssembled, PromptStrategySwitched, StrategyStateUpdated,
};
pub use crate::feat::context::protocol::strategy_id::PromptStrategyId;
