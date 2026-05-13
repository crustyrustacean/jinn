//! Command types for the component command pipeline.
//!
//! The [`Command`] enum contains only domain-level variants. All UI operations
//! are handled by the [`IntentHandler`](nullslop_intent::IntentHandler) via the
//! [`Intent`](nullslop_intent::Intent) enum.
//!
//! # When adding a new domain command
//!
//! Every new command struct **must** be added as a variant on the [`Command`] enum
//! below. Creating the struct alone is not enough — the bus dispatches based on
//! enum variants, so a missing variant means the command is invisible to the system.

use serde::{Deserialize, Serialize};

pub use crate::common::actor::command_msg::CommandMsg;
use crate::common::actor::protocol::command::ProceedWithShutdown;
use crate::feat::chat_input::protocol::command::{
    EnqueueUserMessage, PushChatEntry, SetChatInputText,
};
use crate::feat::context::protocol::command::{
    AssemblePrompt, LoadContextStrategyPickerEntries, LoadPersonaPickerEntries, PinChatEntry,
    RescanPersonas, RestoreStrategyState, SwitchPromptStrategy, UnpinChatEntry,
};
use crate::feat::provider::protocol::command::{
    CancelStream, LoadProviderPickerEntries, ProviderSwitch, RefreshModels, RescanPromptTemplates,
    SendMessage, SendToLlmProvider,
};
use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use crate::feat::session::protocol::session_load_requested::SessionLoadRequested;
use crate::feat::skills::skills_scan_actor::ScanSkills;
use crate::feat::tools_actor::protocol::command::{
    CancelToolBatch, ExecuteTool, ExecuteToolBatch, RegisterTools,
};

/// Every domain command the actor system can receive.
///
/// UI operations have been migrated to the Intent/IntentHandler pipeline.
/// This enum contains only commands that require actor coordination
/// or domain processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Command {
    /// Send a message to the AI provider.
    #[serde(rename = "send_message")]
    SendMessage {
        /// The message to send.
        #[serde(flatten)]
        payload: SendMessage,
    },
    /// Switch the prompt assembly strategy for a session.
    #[serde(rename = "switch_prompt_strategy")]
    SwitchPromptStrategy {
        /// The switch request.
        #[serde(flatten)]
        payload: SwitchPromptStrategy,
    },
    /// Restore a strategy's persisted state for a session.
    #[serde(rename = "restore_strategy_state")]
    RestoreStrategyState {
        /// The state to restore.
        #[serde(flatten)]
        payload: RestoreStrategyState,
    },
    /// Pin a chat entry so it survives context management.
    #[serde(rename = "pin_chat_entry")]
    PinChatEntry {
        /// The pin request.
        #[serde(flatten)]
        payload: PinChatEntry,
    },
    /// Remove the pin from a chat entry.
    #[serde(rename = "unpin_chat_entry")]
    UnpinChatEntry {
        /// The unpin request.
        #[serde(flatten)]
        payload: UnpinChatEntry,
    },
    /// Enqueue a user message for queued processing.
    #[serde(rename = "enqueue_user_message")]
    EnqueueUserMessage {
        /// The message to enqueue.
        #[serde(flatten)]
        payload: EnqueueUserMessage,
    },
    /// Set the chat input buffer text directly.
    #[serde(rename = "set_chat_input_text")]
    SetChatInputText {
        /// The new input text.
        #[serde(flatten)]
        payload: SetChatInputText,
    },
    /// Push a chat entry into the conversation history.
    #[serde(rename = "push_chat_entry")]
    PushChatEntry {
        /// The chat entry to add.
        #[serde(flatten)]
        payload: PushChatEntry,
    },
    /// Cancel the active provider stream.
    #[serde(rename = "cancel_stream")]
    CancelStream {
        /// The cancel stream command.
        #[serde(flatten)]
        payload: CancelStream,
    },
    /// Switch the active LLM provider.
    #[serde(rename = "provider_switch")]
    ProviderSwitch {
        /// The provider switch details.
        #[serde(flatten)]
        payload: ProviderSwitch,
    },
    /// Request prompt assembly from the context actor.
    #[serde(rename = "assemble_prompt")]
    AssemblePrompt {
        /// The assembly request.
        #[serde(flatten)]
        payload: AssemblePrompt,
    },
    /// Send conversation context to the LLM provider.
    #[serde(rename = "send_to_llm_provider")]
    SendToLlmProvider {
        /// The full conversation history as LLM messages.
        #[serde(flatten)]
        payload: SendToLlmProvider,
    },
    /// Refresh the model list from all providers.
    #[serde(rename = "refresh_models")]
    RefreshModels,
    /// Rescan the prompt templates directory.
    #[serde(rename = "rescan_prompt_templates")]
    RescanPromptTemplates,
    /// Register tools that an actor can execute.
    #[serde(rename = "register_tools")]
    RegisterTools {
        /// The registration payload.
        #[serde(flatten)]
        payload: RegisterTools,
    },
    /// Request execution of a batch of tool calls.
    #[serde(rename = "execute_tool_batch")]
    ExecuteToolBatch {
        /// The batch execution payload.
        #[serde(flatten)]
        payload: ExecuteToolBatch,
    },
    /// Execute a single tool call (routed to provider actor).
    #[serde(rename = "execute_tool")]
    ExecuteTool {
        /// The single tool execution payload.
        #[serde(flatten)]
        payload: ExecuteTool,
    },
    /// Cancel all pending tool executions for a session.
    #[serde(rename = "cancel_tool_batch")]
    CancelToolBatch {
        /// The cancellation payload.
        #[serde(flatten)]
        payload: CancelToolBatch,
    },
    /// Proceed with shutdown after actor coordination.
    #[serde(rename = "proceed_with_shutdown")]
    ProceedWithShutdown {
        /// Which actors finished or timed out.
        #[serde(flatten)]
        payload: ProceedWithShutdown,
    },
    /// Session data loaded from disk by the persistence actor.
    #[serde(rename = "session_load_completed")]
    SessionLoadCompleted {
        /// The loaded session data.
        #[serde(flatten)]
        payload: SessionLoadCompleted,
    },
    /// Load entries for the provider/model picker.
    #[serde(rename = "load_provider_picker_entries")]
    LoadProviderPickerEntries {
        #[serde(flatten)]
        payload: LoadProviderPickerEntries,
    },
    /// Load entries for the session picker.
    #[serde(rename = "load_session_picker_entries")]
    LoadSessionPickerEntries {
        #[serde(flatten)]
        payload: LoadSessionPickerEntries,
    },
    /// Load entries for the context strategy picker.
    #[serde(rename = "load_context_strategy_picker_entries")]
    LoadContextStrategyPickerEntries {
        #[serde(flatten)]
        payload: LoadContextStrategyPickerEntries,
    },
    /// Request to load a full session from disk by byte offset.
    #[serde(rename = "session_load_requested")]
    SessionLoadRequested {
        /// The session to load.
        #[serde(flatten)]
        payload: SessionLoadRequested,
    },
    /// Scan the agent skills directory and reload skills.
    #[serde(rename = "scan_skills")]
    ScanSkills,
    /// Rescan the personas directory and reload persona files.
    #[serde(rename = "rescan_personas")]
    RescanPersonas {
        #[serde(flatten)]
        payload: RescanPersonas,
    },
    /// Load entries for the persona picker.
    #[serde(rename = "load_persona_picker_entries")]
    LoadPersonaPickerEntries {
        #[serde(flatten)]
        payload: LoadPersonaPickerEntries,
    },
}

impl Command {
    /// Returns the routing name for this command, if it has one.
    #[must_use]
    pub fn command_name(&self) -> Option<&'static str> {
        match self {
            Self::SendMessage { .. } => Some(SendMessage::NAME),
            Self::SwitchPromptStrategy { .. } => Some(SwitchPromptStrategy::NAME),
            Self::RestoreStrategyState { .. } => Some(RestoreStrategyState::NAME),
            Self::PinChatEntry { .. } => Some(PinChatEntry::NAME),
            Self::UnpinChatEntry { .. } => Some(UnpinChatEntry::NAME),
            Self::EnqueueUserMessage { .. } => Some(EnqueueUserMessage::NAME),
            Self::SetChatInputText { .. } => Some(SetChatInputText::NAME),
            Self::PushChatEntry { .. } => Some(PushChatEntry::NAME),
            Self::CancelStream { .. } => Some(CancelStream::NAME),
            Self::ProviderSwitch { .. } => Some(ProviderSwitch::NAME),
            Self::AssemblePrompt { .. } => Some(AssemblePrompt::NAME),
            Self::SendToLlmProvider { .. } => Some(SendToLlmProvider::NAME),
            Self::RefreshModels => Some(RefreshModels::NAME),
            Self::RescanPromptTemplates => Some(RescanPromptTemplates::NAME),
            Self::RegisterTools { .. } => Some(RegisterTools::NAME),
            Self::ExecuteToolBatch { .. } => Some(ExecuteToolBatch::NAME),
            Self::ExecuteTool { .. } => Some(ExecuteTool::NAME),
            Self::CancelToolBatch { .. } => Some(CancelToolBatch::NAME),
            Self::ProceedWithShutdown { .. } => Some(ProceedWithShutdown::NAME),
            Self::SessionLoadCompleted { .. } => Some(SessionLoadCompleted::NAME),
            Self::LoadProviderPickerEntries { .. } => Some(LoadProviderPickerEntries::NAME),
            Self::LoadSessionPickerEntries { .. } => Some(LoadSessionPickerEntries::NAME),
            Self::LoadContextStrategyPickerEntries { .. } => {
                Some(LoadContextStrategyPickerEntries::NAME)
            }
            Self::SessionLoadRequested { .. } => Some(SessionLoadRequested::NAME),
            Self::ScanSkills => Some(ScanSkills::NAME),
            Self::RescanPersonas { .. } => Some(RescanPersonas::NAME),
            Self::LoadPersonaPickerEntries { .. } => Some(LoadPersonaPickerEntries::NAME),
        }
    }
}

impl std::fmt::Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Command::SendMessage { .. } => write!(f, "send message"),
            Command::SwitchPromptStrategy { .. } => write!(f, "switch prompt strategy"),
            Command::RestoreStrategyState { .. } => write!(f, "restore strategy state"),
            Command::PinChatEntry { payload } => {
                write!(
                    f,
                    "pin entry '{}' as {}",
                    payload.entry_id, payload.position
                )
            }
            Command::UnpinChatEntry { payload } => {
                write!(f, "unpin entry '{}'", payload.entry_id)
            }
            Command::EnqueueUserMessage { .. } => write!(f, "enqueue user message"),
            Command::SetChatInputText { .. } => write!(f, "set chat input text"),
            Command::PushChatEntry { .. } => write!(f, "push chat entry"),
            Command::CancelStream { .. } => write!(f, "cancel stream"),
            Command::ProviderSwitch { payload } => {
                write!(f, "provider switch to '{}'", payload.provider_id)
            }
            Command::AssemblePrompt { .. } => write!(f, "assemble prompt"),
            Command::SendToLlmProvider { .. } => write!(f, "send to LLM provider"),
            Command::RefreshModels => write!(f, "refresh models"),
            Command::RescanPromptTemplates => write!(f, "rescan prompt templates"),
            Command::RegisterTools { payload } => {
                write!(
                    f,
                    "register {} tools from '{}'",
                    payload.definitions.len(),
                    payload.provider
                )
            }
            Command::ExecuteToolBatch { payload } => {
                write!(f, "execute {} tool calls", payload.tool_calls.len())
            }
            Command::ExecuteTool { payload } => {
                write!(
                    f,
                    "execute tool '{}' ({})",
                    payload.tool_call.name, payload.tool_call.id
                )
            }
            Command::CancelToolBatch { .. } => write!(f, "cancel tool batch"),
            Command::ProceedWithShutdown { payload } => {
                write!(
                    f,
                    "proceed with shutdown ({} completed, {} timed out)",
                    payload.completed.len(),
                    payload.timed_out.len()
                )
            }
            Command::SessionLoadCompleted { .. } => write!(f, "session load completed"),
            Command::LoadProviderPickerEntries { .. } => write!(f, "load provider picker entries"),
            Command::LoadSessionPickerEntries { .. } => write!(f, "load session picker entries"),
            Command::LoadContextStrategyPickerEntries { .. } => {
                write!(f, "load context strategy picker entries")
            }
            Command::SessionLoadRequested { .. } => write!(f, "session load requested"),
            Command::ScanSkills => write!(f, "scan skills"),
            Command::RescanPersonas { .. } => write!(f, "rescan personas"),
            Command::LoadPersonaPickerEntries { .. } => {
                write!(f, "load persona picker entries")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::SessionId;

    #[rstest::rstest]
    fn command_name_returns_name_for_routable_commands() {
        // Given routable command variants.
        // When calling command_name().
        // Then they return their routing name.
        assert_eq!(
            Command::PushChatEntry {
                payload: PushChatEntry {
                    session_id: SessionId::new(),
                    entry: crate::ChatEntry::user("test"),
                },
            }
            .command_name(),
            Some(PushChatEntry::NAME)
        );
        assert_eq!(
            Command::CancelStream {
                payload: CancelStream {
                    session_id: SessionId::new(),
                },
            }
            .command_name(),
            Some(CancelStream::NAME)
        );
    }

    #[rstest::rstest]
    fn command_name_uses_derived_constant_for_session_load_requested() {
        // Given a SessionLoadRequested command.
        let cmd = Command::SessionLoadRequested {
            payload: SessionLoadRequested {
                session_id: SessionId::new(),
                byte_offset: 0,
            },
        };

        // When calling command_name().
        // Then it returns the derived NAME constant (not a hardcoded string).
        assert_eq!(
            cmd.command_name(),
            Some(SessionLoadRequested::NAME),
            "command_name must match the derived CommandMsg::NAME for routing to work"
        );
    }

    #[rstest::rstest]
    #[case::provider(crate::PickerKind::Provider, "models")]
    #[case::context_assembly(crate::PickerKind::ContextAssembly, "context-assembly")]
    #[case::keymap(crate::PickerKind::Keymap, "keybinds")]
    #[case::session(crate::PickerKind::Session, "sessions")]
    fn picker_kind_display(#[case] kind: crate::PickerKind, #[case] expected: &str) {
        assert_eq!(kind.to_string(), expected);
    }
}
