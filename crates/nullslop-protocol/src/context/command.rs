//! Command types for prompt assembly.

use serde::{Deserialize, Serialize};

use crate::ChatEntry;
use crate::ChatEntryId;
use crate::CommandMsg;
use crate::PinPosition;
use crate::SessionId;
use crate::context::PromptStrategyId;
use crate::tool::ToolDefinition;

/// Request to assemble a prompt from the given history.
///
/// Sent by the message queue handler when a message needs to go to the LLM.
/// The `PromptAssemblyActor` receives this, runs the appropriate strategy,
/// and emits [`PromptAssembled`](super::PromptAssembled) when done.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("context")]
pub struct AssemblePrompt {
    /// The session this assembly is for.
    pub session_id: SessionId,
    /// The full conversation history to assemble from.
    pub history: Vec<ChatEntry>,
    /// Tool definitions available for this session.
    pub tools: Vec<ToolDefinition>,
    /// The name of the model being used.
    pub model_name: String,
}

/// Request to switch the prompt assembly strategy for a session.
///
/// Sent when a user or system action changes the active strategy.
/// The `PromptAssemblyActor` receives this, creates the new strategy
/// via the factory, and emits [`PromptStrategySwitched`](super::PromptStrategySwitched).
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("context")]
pub struct SwitchPromptStrategy {
    /// The session whose strategy should be switched.
    pub session_id: SessionId,
    /// The strategy to switch to.
    pub strategy_id: PromptStrategyId,
}

/// Restore a strategy's persisted state for a session.
///
/// Sent when a session is loaded and the host wants to rehydrate
/// strategy-specific state (e.g., compaction summaries) into the actor.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("context")]
pub struct RestoreStrategyState {
    /// The session whose strategy state should be restored.
    pub session_id: SessionId,
    /// The strategy the state belongs to.
    pub strategy_id: PromptStrategyId,
    /// The opaque state blob to restore.
    pub blob: serde_json::Value,
}

/// Pin a chat entry so it survives context management strategies.
///
/// The entry will be positioned according to `position` in the assembled prompt.
/// Phase 1's `pin_entry()` method on `ChatSessionState` handles the actual mutation.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("context")]
pub struct PinChatEntry {
    /// The session containing the entry.
    pub session_id: SessionId,
    /// The entry to pin.
    pub entry_id: ChatEntryId,
    /// Where the pinned entry should appear in the assembled prompt.
    pub position: PinPosition,
}

/// Remove the pin from a chat entry, allowing normal context management.
///
/// If the entry is not pinned, this is a no-op.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("context")]
pub struct UnpinChatEntry {
    /// The session containing the entry.
    pub session_id: SessionId,
    /// The entry to unpin.
    pub entry_id: ChatEntryId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn assemble_prompt_serialization_roundtrip() {
        // Given an AssemblePrompt command.
        let cmd = AssemblePrompt {
            session_id: SessionId::new(),
            history: vec![ChatEntry::user("hello")],
            tools: vec![],
            model_name: "test-model".to_owned(),
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&cmd).expect("serialize");
        let back: AssemblePrompt = serde_json::from_str(&json).expect("deserialize");

        // Then fields are preserved.
        assert_eq!(back.model_name, "test-model");
        assert_eq!(back.history.len(), 1);
        assert!(back.tools.is_empty());
    }

    #[rstest::rstest]
    fn switch_prompt_strategy_serialization_roundtrip() {
        // Given a SwitchPromptStrategy command.
        let cmd = SwitchPromptStrategy {
            session_id: SessionId::new(),
            strategy_id: PromptStrategyId::sliding_window(),
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&cmd).expect("serialize");
        let back: SwitchPromptStrategy = serde_json::from_str(&json).expect("deserialize");

        // Then fields are preserved.
        assert_eq!(back.strategy_id, PromptStrategyId::sliding_window());
    }

    #[rstest::rstest]
    fn restore_strategy_state_serialization_roundtrip() {
        // Given a RestoreStrategyState command.
        let cmd = RestoreStrategyState {
            session_id: SessionId::new(),
            strategy_id: PromptStrategyId::compaction(),
            blob: serde_json::json!({"compaction_count": 5}),
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&cmd).expect("serialize");
        let back: RestoreStrategyState = serde_json::from_str(&json).expect("deserialize");

        // Then fields are preserved.
        assert_eq!(back.strategy_id, PromptStrategyId::compaction());
        assert_eq!(back.blob["compaction_count"], 5);
    }

    #[rstest::rstest]
    fn pin_chat_entry_serialization_roundtrip() {
        // Given a PinChatEntry command.
        let cmd = PinChatEntry {
            session_id: SessionId::new(),
            entry_id: ChatEntryId::new(),
            position: PinPosition::Top,
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&cmd).expect("serialize");
        let back: PinChatEntry = serde_json::from_str(&json).expect("deserialize");

        // Then fields are preserved.
        assert_eq!(back.position, PinPosition::Top);
    }

    #[rstest::rstest]
    fn unpin_chat_entry_serialization_roundtrip() {
        // Given an UnpinChatEntry command.
        let cmd = UnpinChatEntry {
            session_id: SessionId::new(),
            entry_id: ChatEntryId::new(),
        };

        // When serialized and deserialized.
        let json = serde_json::to_string(&cmd).expect("serialize");
        let back: UnpinChatEntry = serde_json::from_str(&json).expect("deserialize");

        // Then fields are preserved.
        assert_eq!(back.entry_id, cmd.entry_id);
    }
}
