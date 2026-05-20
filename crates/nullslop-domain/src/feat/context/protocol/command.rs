//! Command types for prompt assembly.

use serde::{Deserialize, Serialize};

use crate::feat::tools_actor::tool_types::ToolDefinition;
use crate::protocol::ChatEntry;
use crate::protocol::ChatEntryId;
use crate::protocol::CommandMsg;
use crate::protocol::PinPosition;
use crate::protocol::SessionId;

/// Request to assemble a prompt from the given history.
///
/// Sent by the message queue handler when a message needs to go to the LLM.
/// The `PromptAssemblyActor` receives this, runs compaction logic,
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

/// Pin a chat entry so it survives context management strategies.
///
/// The entry will be positioned according to `position` in the assembled prompt.
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

/// Rescan the personas directory and reload persona files.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("context")]
pub struct RescanPersonas;

/// Load entries for the persona picker.
#[derive(Debug, Clone, Serialize, Deserialize, CommandMsg)]
#[cmd("context")]
pub struct LoadPersonaPickerEntries;
