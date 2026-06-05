//! Command types for the component command pipeline.
//!
//! The [`Command`] enum contains only domain-level variants. All UI operations
//! are handled by the [`IntentHandler`](jinn_intent::IntentHandler) via the
//! [`Intent`](jinn_intent::Intent) enum.
//!
//! # When adding a new domain command
//!
//! Every new command struct **must** be added as a variant on the [`Command`] enum
//! below. Creating the struct alone is not enough - the bus dispatches based on
//! enum variants, so a missing variant means the command is invisible to the system.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

pub use crate::common::actor::command_msg::CommandMsg;
use crate::common::actor::protocol::command::ProceedWithShutdown;
pub use crate::common::actor::protocol::dynamic_command::DynamicCommand;
use crate::feat::chat_input::protocol::command::{
    EnqueueUserMessage, PushChatEntry, SetChatInputText,
};
use crate::feat::context::protocol::command::{
    LoadPersonaPickerEntries, PinChatEntry, RescanPersonas, UnpinChatEntry,
};
use crate::feat::preferences_actor::protocol::command::UpdatePreferences;
use crate::feat::provider::protocol::command::{
    CancelStream, LoadProviderPickerEntries, ProviderSwitch, RefreshModels, RescanPromptTemplates,
    SendMessage, SendToLlmProvider,
};
use crate::feat::session::protocol::close_session::CloseSession;
use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;
use crate::feat::session::protocol::mark_session_interacted::MarkSessionInteracted;
use crate::feat::session::protocol::session_fork_requested::SessionForkRequested;
use crate::feat::session::protocol::session_load_requested::SessionLoadRequested;
use crate::feat::session::protocol::submit_history_mutations::SubmitHistoryMutations;
use crate::feat::session_lifecycle::protocol::command::{
    CancelLifecycleCommand, FinishSessionSetup, FinishSessionTeardown, PersistSession,
    RunSessionSetup, RunSessionTeardown,
};
use crate::feat::skills::skills_scan_actor::ScanSkills;
use crate::feat::tools_actor::protocol::command::{
    CancelToolBatch, ExecuteTool, ExecuteToolBatch, ExecuteWebFetch, RegisterTools,
};

/// Every domain command the actor system can receive.
///
/// UI operations have been migrated to the Intent/IntentHandler pipeline.
/// This enum contains only commands that require actor coordination
/// or domain processing.
#[allow(
    clippy::large_enum_variant,
    reason = "boxing would cascade through all match arms"
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    /// Send a message to the AI provider.
    SendMessage(SendMessage),
    /// Pin a chat entry so it survives context management.
    PinChatEntry(PinChatEntry),
    /// Remove the pin from a chat entry.
    UnpinChatEntry(UnpinChatEntry),
    /// Enqueue a user message for queued processing.
    EnqueueUserMessage(EnqueueUserMessage),
    /// Set the chat input buffer text directly.
    SetChatInputText(SetChatInputText),
    /// Push a chat entry into the conversation history.
    PushChatEntry(PushChatEntry),
    /// Cancel the active provider stream.
    CancelStream(CancelStream),
    /// Switch the active LLM provider.
    ProviderSwitch(ProviderSwitch),
    /// Send conversation context to the LLM provider.
    SendToLlmProvider(SendToLlmProvider),
    /// Refresh the model list from all providers.
    RefreshModels,
    /// Rescan the prompt templates directory.
    RescanPromptTemplates,
    /// Register tools that an actor can execute.
    RegisterTools(RegisterTools),
    /// Request execution of a batch of tool calls.
    ExecuteToolBatch(ExecuteToolBatch),
    /// Execute a single tool call (routed to provider actor).
    ExecuteTool(ExecuteTool),
    /// Cancel all pending tool executions for a session.
    CancelToolBatch(CancelToolBatch),
    /// Execute a web-fetch tool call.
    ExecuteWebFetch(ExecuteWebFetch),
    /// Proceed with shutdown after actor coordination.
    ProceedWithShutdown(ProceedWithShutdown),
    /// Load entries for the provider/model picker.
    LoadProviderPickerEntries(LoadProviderPickerEntries),
    /// Load entries for the compaction model picker.
    LoadCompactionModelPickerEntries(
        crate::feat::provider::protocol::command::LoadCompactionModelPickerEntries,
    ),
    /// Load entries for the session picker.
    LoadSessionPickerEntries(LoadSessionPickerEntries),
    /// Request to load a full session from disk by byte offset.
    SessionLoadRequested(SessionLoadRequested),
    /// Scan the agent skills directory and reload skills.
    ScanSkills,
    /// Rescan the personas directory and reload persona files.
    RescanPersonas(RescanPersonas),
    /// Load entries for the persona picker.
    LoadPersonaPickerEntries(LoadPersonaPickerEntries),
    /// Update one or more user preferences (persisted to jinn.toml).
    UpdatePreferences(UpdatePreferences),
    /// Request to fork a session at a specific entry ordinal.
    SessionForkRequested(SessionForkRequested),
    /// Run a lifecycle setup command asynchronously.
    RunSessionSetup(RunSessionSetup),
    /// Run a lifecycle teardown command asynchronously.
    RunSessionTeardown(RunSessionTeardown),
    /// Close a session from the sessions map.
    CloseSession(CloseSession),
    /// Mark a session as having been interacted with by the user.
    MarkSessionInteracted(MarkSessionInteracted),
    /// Archive a session without running teardown.
    ArchiveSession(crate::feat::session::protocol::archive_session::ArchiveSession),
    /// Persist a session's full state to SQLite immediately.
    PersistSession(PersistSession),
    /// Submit a batch of history mutations for deferred application.
    SubmitHistoryMutations(SubmitHistoryMutations),
    /// Finish an async teardown shell command (result from spawned task).
    FinishSessionTeardown(FinishSessionTeardown),
    /// Finish an async setup shell command (result from spawned task).
    FinishSessionSetup(FinishSessionSetup),
    /// Request to cancel a running lifecycle command.
    CancelLifecycleCommand(CancelLifecycleCommand),
    /// Attach an attachable plugin to a session.
    AttachPlugin(crate::feat::plugin_dispatch::protocol::command::AttachPlugin),
    /// Detach a plugin from a session.
    DetachPlugin(crate::feat::plugin_dispatch::protocol::command::DetachPlugin),
    /// Toggle an attached plugin on/off.
    TogglePlugin(crate::feat::plugin_dispatch::protocol::command::TogglePlugin),

    /// A dynamic command from a plugin, carrying an arbitrary JSON payload.
    Dynamic(DynamicCommand),
    /// Trigger compaction for a session (from /compact or /compact-all).
    TriggerCompaction(crate::feat::session::protocol::trigger_compaction::TriggerCompaction),
}

impl Command {
    /// Returns the routing name for this command, if it has one.
    #[must_use]
    pub fn command_name(&self) -> Option<&'static str> {
        match self {
            Self::SendMessage(..) => Some(SendMessage::NAME),
            Self::PinChatEntry(..) => Some(PinChatEntry::NAME),
            Self::UnpinChatEntry(..) => Some(UnpinChatEntry::NAME),
            Self::EnqueueUserMessage(..) => Some(EnqueueUserMessage::NAME),
            Self::SetChatInputText(..) => Some(SetChatInputText::NAME),
            Self::PushChatEntry(..) => Some(PushChatEntry::NAME),
            Self::CancelStream(..) => Some(CancelStream::NAME),
            Self::ProviderSwitch(..) => Some(ProviderSwitch::NAME),

            Self::SendToLlmProvider(..) => Some(SendToLlmProvider::NAME),
            Self::RefreshModels => Some(RefreshModels::NAME),
            Self::RescanPromptTemplates => Some(RescanPromptTemplates::NAME),
            Self::RegisterTools(..) => Some(RegisterTools::NAME),
            Self::ExecuteToolBatch(..) => Some(ExecuteToolBatch::NAME),
            Self::ExecuteTool(..) => Some(ExecuteTool::NAME),
            Self::CancelToolBatch(..) => Some(CancelToolBatch::NAME),
            Self::ExecuteWebFetch(..) => Some(ExecuteWebFetch::NAME),
            Self::ProceedWithShutdown(..) => Some(ProceedWithShutdown::NAME),
            Self::LoadProviderPickerEntries(..) => Some(LoadProviderPickerEntries::NAME),
            Self::LoadCompactionModelPickerEntries(..) => Some(
                crate::feat::provider::protocol::command::LoadCompactionModelPickerEntries::NAME,
            ),
            Self::LoadSessionPickerEntries(..) => Some(LoadSessionPickerEntries::NAME),
            Self::SessionLoadRequested(..) => Some(SessionLoadRequested::NAME),
            Self::ScanSkills => Some(ScanSkills::NAME),
            Self::RescanPersonas(..) => Some(RescanPersonas::NAME),
            Self::LoadPersonaPickerEntries(..) => Some(LoadPersonaPickerEntries::NAME),
            Self::UpdatePreferences(..) => Some(UpdatePreferences::NAME),
            Self::SessionForkRequested(..) => Some(SessionForkRequested::NAME),
            Self::RunSessionSetup(..) => Some(RunSessionSetup::NAME),
            Self::RunSessionTeardown(..) => Some(RunSessionTeardown::NAME),
            Self::CloseSession(..) => Some(CloseSession::NAME),
            Self::MarkSessionInteracted(..) => Some(MarkSessionInteracted::NAME),
            Self::ArchiveSession(..) => {
                Some(crate::feat::session::protocol::archive_session::ArchiveSession::NAME)
            }
            Self::PersistSession(..) => Some(PersistSession::NAME),
            Self::SubmitHistoryMutations(..) => Some(SubmitHistoryMutations::NAME),
            Self::FinishSessionTeardown(..) => Some(FinishSessionTeardown::NAME),
            Self::FinishSessionSetup(..) => Some(FinishSessionSetup::NAME),
            Self::CancelLifecycleCommand(..) => Some(CancelLifecycleCommand::NAME),
            Self::AttachPlugin(..) => {
                Some(crate::feat::plugin_dispatch::protocol::command::AttachPlugin::NAME)
            }
            Self::DetachPlugin(..) => {
                Some(crate::feat::plugin_dispatch::protocol::command::DetachPlugin::NAME)
            }
            Self::TogglePlugin(..) => {
                Some(crate::feat::plugin_dispatch::protocol::command::TogglePlugin::NAME)
            }
            Self::Dynamic(..) | Self::TriggerCompaction(..) => None,
        }
    }

    /// Returns the routing key for bus dispatch.
    #[must_use]
    pub fn routing_key(&self) -> Option<Cow<'_, str>> {
        match self {
            Self::Dynamic(d) => Some(Cow::Borrowed(&d.name)),
            _ => self.command_name().map(|s| Cow::Owned(s.to_owned())),
        }
    }
}

impl std::fmt::Display for Command {
    #[expect(
        clippy::too_many_lines,
        reason = "large match on enum variants, each arm is 1-4 lines"
    )]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Command::SendMessage(..) => write!(f, "send message"),
            Command::PinChatEntry(payload) => {
                write!(
                    f,
                    "pin entry '{}' as {}",
                    payload.entry_id, payload.position
                )
            }
            Command::UnpinChatEntry(payload) => {
                write!(f, "unpin entry '{}'", payload.entry_id)
            }
            Command::EnqueueUserMessage(..) => write!(f, "enqueue user message"),
            Command::SetChatInputText(..) => write!(f, "set chat input text"),
            Command::PushChatEntry(..) => write!(f, "push chat entry"),
            Command::CancelStream(..) => write!(f, "cancel stream"),
            Command::ProviderSwitch(payload) => {
                write!(f, "provider switch to '{}'", payload.provider_id)
            }

            Command::SendToLlmProvider(..) => write!(f, "send to LLM provider"),
            Command::RefreshModels => write!(f, "refresh models"),
            Command::RescanPromptTemplates => write!(f, "rescan prompt templates"),
            Command::RegisterTools(payload) => {
                write!(
                    f,
                    "register {} tools from '{}'",
                    payload.definitions.len(),
                    payload.provider
                )
            }
            Command::ExecuteToolBatch(payload) => {
                write!(f, "execute {} tool calls", payload.tool_calls.len())
            }
            Command::ExecuteTool(payload) => {
                write!(
                    f,
                    "execute tool '{}' ({})",
                    payload.tool_call.name, payload.tool_call.id
                )
            }
            Command::CancelToolBatch(..) => write!(f, "cancel tool batch"),
            Command::ExecuteWebFetch(payload) => {
                write!(
                    f,
                    "execute web-fetch '{}' ({})",
                    payload.tool_call.name, payload.tool_call.id
                )
            }
            Command::ProceedWithShutdown(payload) => {
                write!(
                    f,
                    "proceed with shutdown ({} completed, {} timed out)",
                    payload.completed.len(),
                    payload.timed_out.len()
                )
            }
            Command::LoadProviderPickerEntries(..) => write!(f, "load provider picker entries"),
            Command::LoadCompactionModelPickerEntries(..) => {
                write!(f, "load compaction model picker entries")
            }
            Command::LoadSessionPickerEntries(..) => write!(f, "load session picker entries"),
            Command::SessionLoadRequested(..) => write!(f, "session load requested"),
            Command::ScanSkills => write!(f, "scan skills"),
            Command::RescanPersonas(..) => write!(f, "rescan personas"),
            Command::LoadPersonaPickerEntries(..) => {
                write!(f, "load persona picker entries")
            }
            Command::UpdatePreferences(payload) => {
                write!(f, "update preferences ({} diff(s))", payload.updates.len())
            }
            Command::SessionForkRequested(payload) => {
                write!(f, "session fork at ordinal {}", payload.at_ordinal)
            }
            Command::RunSessionSetup(payload) => {
                write!(f, "run session setup for {}", payload.session_id)
            }
            Command::RunSessionTeardown(payload) => {
                write!(f, "run session teardown for {}", payload.session_id)
            }
            Command::CloseSession(payload) => {
                write!(f, "close session {}", payload.session_id)
            }
            Command::MarkSessionInteracted(payload) => {
                write!(f, "mark session interacted {}", payload.session_id)
            }
            Command::ArchiveSession(payload) => {
                write!(f, "archive session {}", payload.session_id)
            }
            Command::PersistSession(payload) => {
                write!(f, "persist session {}", payload.session_id)
            }
            Command::FinishSessionTeardown(payload) => {
                write!(f, "finish session teardown for {}", payload.session_id)
            }
            Command::FinishSessionSetup(payload) => {
                write!(f, "finish session setup for {}", payload.session_id)
            }
            Command::CancelLifecycleCommand(payload) => {
                write!(f, "cancel lifecycle command for {}", payload.session_id)
            }

            Command::SubmitHistoryMutations(payload) => {
                write!(
                    f,
                    "submit {} history mutations for {}",
                    payload.mutations.len(),
                    payload.session_id
                )
            }
            Command::AttachPlugin(p) => {
                write!(f, "attach plugin {} to {}", p.plugin_name, p.session_id)
            }
            Command::DetachPlugin(p) => {
                write!(f, "detach plugin {} from {}", p.plugin_name, p.session_id)
            }
            Command::TogglePlugin(p) => {
                write!(f, "toggle plugin {} on {}", p.plugin_name, p.session_id)
            }

            Command::Dynamic(d) => {
                write!(f, "dynamic command '{}'", d.name)
            }
            Command::TriggerCompaction(payload) => {
                write!(
                    f,
                    "trigger compaction for {} (compact_all={})",
                    payload.session_id, payload.compact_all
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use crate::protocol::SessionId;

    #[rstest::rstest]
    fn command_name_returns_name_for_routable_commands() {
        assert_eq!(
            Command::PushChatEntry(PushChatEntry {
                session_id: SessionId::new(),
                entry: crate::ChatEntry::user("test"),
            })
            .command_name(),
            Some(PushChatEntry::NAME)
        );
        assert_eq!(
            Command::CancelStream(CancelStream {
                session_id: SessionId::new(),
            })
            .command_name(),
            Some(CancelStream::NAME)
        );
    }

    #[rstest::rstest]
    fn command_name_uses_derived_constant_for_session_load_requested() {
        let cmd = Command::SessionLoadRequested(SessionLoadRequested {
            session_id: SessionId::new(),
        });

        assert_eq!(
            cmd.command_name(),
            Some(SessionLoadRequested::NAME),
            "command_name must match the derived CommandMsg::NAME for routing to work"
        );
    }

    #[rstest::rstest]
    #[case::provider(crate::PickerKind::Provider, "models")]
    #[case::session(crate::PickerKind::Session, "sessions")]
    fn picker_kind_display(#[case] kind: crate::PickerKind, #[case] expected: &str) {
        assert_eq!(kind.to_string(), expected);
    }

    #[rstest::rstest]
    fn routing_key_returns_runtime_name_for_dynamic() {
        let cmd = Command::Dynamic(DynamicCommand {
            name: "welcome::show".to_owned(),
            payload: serde_json::Value::Null,
        });

        let key = cmd.routing_key().expect("should have routing key");
        assert_eq!(&*key, "welcome::show");
    }

    #[rstest::rstest]
    fn routing_key_returns_static_name_for_typed_command() {
        let cmd = Command::CancelStream(CancelStream {
            session_id: SessionId::new(),
        });

        let key = cmd.routing_key().expect("should have routing key");
        assert_eq!(&*key, CancelStream::NAME);
    }
}
