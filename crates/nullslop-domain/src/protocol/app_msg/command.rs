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
    AssemblePrompt, PinChatEntry, RestoreStrategyState, SwitchPromptStrategy, UnpinChatEntry,
};
use crate::feat::provider::protocol::command::{
    CancelStream, ProviderSwitch, RefreshModels, RescanPromptTemplates, SendMessage,
    SendToLlmProvider,
};
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use crate::feat::session::protocol::session_load_requested::SessionLoadRequested;
use crate::feat::skills::skills_scan_actor::ScanSkills;
use crate::feat::tools_actor::protocol::command::{ExecuteTool, ExecuteToolBatch, RegisterTools};
use crate::protocol::system::LoadPickerEntries;

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
    /// Load entries for the active picker from the actor system.
    #[serde(rename = "load_picker_entries")]
    LoadPickerEntries {
        /// Which picker kind to load entries for.
        #[serde(flatten)]
        payload: LoadPickerEntries,
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
            Self::ProceedWithShutdown { .. } => Some(ProceedWithShutdown::NAME),
            Self::SessionLoadCompleted { .. } => Some(SessionLoadCompleted::NAME),
            Self::LoadPickerEntries { .. } => Some(LoadPickerEntries::NAME),
            Self::SessionLoadRequested { .. } => Some("SessionLoadRequested"),
            Self::ScanSkills => Some(ScanSkills::NAME),
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
            Command::ProceedWithShutdown { payload } => {
                write!(
                    f,
                    "proceed with shutdown ({} completed, {} timed out)",
                    payload.completed.len(),
                    payload.timed_out.len()
                )
            }
            Command::SessionLoadCompleted { .. } => write!(f, "session load completed"),
            Command::LoadPickerEntries { payload } => {
                write!(f, "load {} picker entries", payload.kind)
            }
            Command::SessionLoadRequested { .. } => write!(f, "session load requested"),
            Command::ScanSkills => write!(f, "scan skills"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::SessionId;

    #[rstest::rstest]
    #[case::send_message(Command::SendMessage { payload: SendMessage { session_id: SessionId::new(), text: "hi".into() } })]
    #[case::assemble_prompt(Command::AssemblePrompt { payload: AssemblePrompt { session_id: SessionId::new(), history: vec![], tools: vec![], model_name: "test".to_owned() } })]
    #[case::switch_prompt_strategy(Command::SwitchPromptStrategy { payload: SwitchPromptStrategy { session_id: SessionId::new(), strategy_id: crate::PromptStrategyId::sliding_window() } })]
    #[case::restore_strategy_state(Command::RestoreStrategyState { payload: RestoreStrategyState { session_id: SessionId::new(), strategy_id: crate::PromptStrategyId::compaction(), blob: serde_json::json!({}) } })]
    #[case::push_chat_entry(Command::PushChatEntry { payload: PushChatEntry { session_id: SessionId::new(), entry: crate::ChatEntry::user("hi") } })]
    #[case::enqueue_user_message(Command::EnqueueUserMessage { payload: EnqueueUserMessage { session_id: SessionId::new(), text: "hello".into() } })]
    #[case::set_chat_input_text(Command::SetChatInputText { payload: SetChatInputText { session_id: SessionId::new(), text: "restored".into() } })]
    #[case::cancel_stream(Command::CancelStream { payload: CancelStream { session_id: SessionId::new() } })]
    #[case::provider_switch(Command::ProviderSwitch { payload: ProviderSwitch { provider_id: "ollama".into() } })]
    #[case::send_to_llm_provider(Command::SendToLlmProvider { payload: SendToLlmProvider { session_id: SessionId::new(), messages: vec![], provider_id: None } })]
    #[case::refresh_models(Command::RefreshModels)]
    #[case::rescan_prompt_templates(Command::RescanPromptTemplates)]
    #[case::register_tools(Command::RegisterTools { payload: RegisterTools { provider: "echo-actor".into(), definitions: vec![crate::ToolDefinition { name: "echo".into(), description: "echo".into(), parameters: serde_json::json!({}) }] } })]
    #[case::execute_tool_batch(Command::ExecuteToolBatch { payload: ExecuteToolBatch { session_id: SessionId::new(), tool_calls: vec![crate::ToolCall { id: "call_1".into(), name: "echo".into(), arguments: "{}".into() }] } })]
    #[case::execute_tool(Command::ExecuteTool { payload: ExecuteTool { session_id: SessionId::new(), tool_call: crate::ToolCall { id: "call_1".into(), name: "echo".into(), arguments: "{}".into() } } })]
    #[case::proceed_with_shutdown(Command::ProceedWithShutdown { payload: ProceedWithShutdown { completed: vec!["ext-a".into()], timed_out: vec!["ext-b".into()] } })]
    #[case::session_load_completed(Command::SessionLoadCompleted { payload: SessionLoadCompleted {
        session_id: SessionId::new(),
        title: "Test".to_owned(),
        history: vec![],
        active_strategy: crate::PromptStrategyId::passthrough(),
        blobs: std::collections::HashMap::new(),
    } })]
    #[case::pin_chat_entry(Command::PinChatEntry { payload: PinChatEntry { session_id: SessionId::new(), entry_id: crate::ChatEntryId::new(), position: crate::protocol::PinPosition::Top } })]
    #[case::unpin_chat_entry(Command::UnpinChatEntry { payload: UnpinChatEntry { session_id: SessionId::new(), entry_id: crate::ChatEntryId::new() } })]
    #[case::load_picker_entries(Command::LoadPickerEntries { payload: LoadPickerEntries { kind: crate::PickerKind::Provider } })]
    #[case::session_load_requested(Command::SessionLoadRequested { payload: SessionLoadRequested {
        session_id: SessionId::new(), byte_offset: 42u64,
    } })]
    #[case::scan_skills(Command::ScanSkills)]
    fn command_roundtrip_all_variants(#[case] cmd: Command) {
        // Given a command variant.
        let json = serde_json::to_string(&cmd).expect("serialize");

        // When deserialized.
        let back: Command = serde_json::from_str(&json).expect("deserialize");

        // Then it matches the original when re-serialized.
        let back_json = serde_json::to_string(&back).expect("re-serialize");
        assert_eq!(json, back_json);
    }

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
    #[case::provider(crate::PickerKind::Provider, "provider")]
    #[case::context_assembly(crate::PickerKind::ContextAssembly, "context-assembly")]
    #[case::keymap(crate::PickerKind::Keymap, "keymap")]
    #[case::session(crate::PickerKind::Session, "session")]
    fn picker_kind_display(#[case] kind: crate::PickerKind, #[case] expected: &str) {
        assert_eq!(kind.to_string(), expected);
    }
}
