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

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

pub use crate::common::actor::command_msg::CommandMsg;
use crate::common::actor::protocol::command::ProceedWithShutdown;
pub use crate::common::actor::protocol::dynamic_command::DynamicCommand;
use crate::feat::chat_input::protocol::command::{
    EnqueueUserMessage, PushChatEntry, SetChatInputText,
};
use crate::feat::compaction_actor::protocol::command::{
    BeginCompaction, CancelCompaction, CompactContext, EndCompaction, EnqueueCompaction,
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
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use crate::feat::session::protocol::session_load_requested::SessionLoadRequested;
use crate::feat::session::protocol::soft_cancel_turn::SoftCancelTurn;
use crate::feat::session::protocol::schedule_auto_compaction::ScheduleAutoCompaction;
use crate::feat::session_lifecycle::protocol::command::{
    FinishSessionTeardown, PersistSession, RunSessionSetup, RunSessionTeardown,
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
#[expect(
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
    /// Session data loaded from disk by the persistence actor.
    SessionLoadCompleted(SessionLoadCompleted),
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
    /// Update one or more user preferences (persisted to nullslop.toml).
    UpdatePreferences(UpdatePreferences),
    /// Request to fork a session at a specific entry ordinal.
    SessionForkRequested(SessionForkRequested),
    /// Run a lifecycle setup command asynchronously.
    RunSessionSetup(RunSessionSetup),
    /// Run a lifecycle teardown command asynchronously.
    RunSessionTeardown(RunSessionTeardown),
    /// Request to compact the conversation context for a session.
    CompactContext(CompactContext),
    /// Enqueue a compaction via the session queue (waits for session to be idle).
    EnqueueCompaction(EnqueueCompaction),
    /// Begin a context compaction — marks entries ignored, sets phase to Compacting.
    BeginCompaction(BeginCompaction),
    /// End a context compaction — inserts result entry, sets phase to Idle.
    EndCompaction(EndCompaction),
    /// Cancel an in-progress context compaction — aborts the LLM call.
    CancelCompaction(CancelCompaction),
    /// Close a session from the sessions map.
    CloseSession(CloseSession),
    /// Mark a session as having been interacted with by the user.
    MarkSessionInteracted(MarkSessionInteracted),
    /// Archive a session without running teardown.
    ArchiveSession(crate::feat::session::protocol::archive_session::ArchiveSession),
    /// Persist a session's full state to SQLite immediately.
    PersistSession(PersistSession),
    /// Request graceful termination of the current turn.
    SoftCancelTurn(SoftCancelTurn),
    /// Schedule auto-compaction at the next turn boundary.
    ///
    /// Sent by the CompactionActor when token threshold is exceeded.
    /// The session transitions directly to Compacting — never through Idle.
    ScheduleAutoCompaction(ScheduleAutoCompaction),
    /// Finish an async teardown shell command (result from spawned task).
    FinishSessionTeardown(FinishSessionTeardown),
    /// Request to load (initialize) a named workflow without executing it.
    InitWorkflow(crate::feat::workflow::protocol::command::InitWorkflow),
    /// Request to start a named workflow.
    StartWorkflow(crate::feat::workflow::protocol::command::StartWorkflow),
    /// Request to cancel a running workflow.
    CancelWorkflow(crate::feat::workflow::protocol::command::CancelWorkflow),
    /// Request to re-run a workflow from a specific node.
    RerunFromNode(crate::feat::workflow::protocol::command::RerunFromNode),
    /// Load entries for the workflow picker.
    LoadWorkflowPickerEntries(crate::feat::workflow::protocol::command::LoadWorkflowPickerEntries),
    /// Rescan the judges directory and reload judge definitions.
    RescanJudges(crate::feat::judge::RescanJudges),
    /// Cancel a pending judge evaluation cycle (ESC on Idle+busy origin).
    CancelPendingJudgeEvaluation(crate::feat::judge::CancelPendingJudgeEvaluation),
    /// A dynamic command from a plugin, carrying an arbitrary JSON payload.
    ///
    /// Routed by the runtime [`name`](DynamicCommand::name) field, not the
    /// static `CommandMsg::NAME`. If no actor subscribes to that name, the
    /// command is silently dropped.
    Dynamic(DynamicCommand),
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
            Self::SessionLoadCompleted(..) => Some(SessionLoadCompleted::NAME),
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
            Self::CompactContext(..) => Some(CompactContext::NAME),
            Self::EnqueueCompaction(..) => Some(EnqueueCompaction::NAME),
            Self::BeginCompaction(..) => Some(BeginCompaction::NAME),
            Self::EndCompaction(..) => Some(EndCompaction::NAME),
            Self::CancelCompaction(..) => Some(CancelCompaction::NAME),
            Self::CloseSession(..) => Some(CloseSession::NAME),
            Self::MarkSessionInteracted(..) => Some(MarkSessionInteracted::NAME),
            Self::ArchiveSession(..) => {
                Some(crate::feat::session::protocol::archive_session::ArchiveSession::NAME)
            }
            Self::PersistSession(..) => Some(PersistSession::NAME),
            Self::SoftCancelTurn(..) => Some(SoftCancelTurn::NAME),
            Self::ScheduleAutoCompaction(..) => Some(ScheduleAutoCompaction::NAME),
            Self::FinishSessionTeardown(..) => Some(FinishSessionTeardown::NAME),
            Self::InitWorkflow(..) => {
                Some(crate::feat::workflow::protocol::command::InitWorkflow::NAME)
            }
            Self::StartWorkflow(..) => {
                Some(crate::feat::workflow::protocol::command::StartWorkflow::NAME)
            }
            Self::CancelWorkflow(..) => {
                Some(crate::feat::workflow::protocol::command::CancelWorkflow::NAME)
            }
            Self::RerunFromNode(..) => {
                Some(crate::feat::workflow::protocol::command::RerunFromNode::NAME)
            }
            Self::LoadWorkflowPickerEntries(..) => {
                Some(crate::feat::workflow::protocol::command::LoadWorkflowPickerEntries::NAME)
            }
            Self::RescanJudges(..) => Some(crate::feat::judge::RescanJudges::NAME),
            Self::CancelPendingJudgeEvaluation(..) => {
                Some(crate::feat::judge::CancelPendingJudgeEvaluation::NAME)
            }
            Self::Dynamic(..) => Some(DynamicCommand::NAME),
        }
    }

    /// Returns the routing key for bus dispatch.
    ///
    /// Typed variants return their static command name as an owned `Cow`.
    /// `Dynamic` returns the runtime `.name` field as a borrowed `Cow`,
    /// allowing plugins to define arbitrary routing keys.
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
            Command::SessionLoadCompleted(..) => write!(f, "session load completed"),
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
            Command::CompactContext(payload) => {
                write!(f, "compact context for {}", payload.session_id)
            }
            Command::EnqueueCompaction(payload) => {
                write!(f, "enqueue compaction for {}", payload.session_id)
            }
            Command::BeginCompaction(payload) => {
                write!(f, "begin compaction for {}", payload.session_id)
            }
            Command::EndCompaction(payload) => {
                write!(f, "end compaction for {}", payload.session_id)
            }
            Command::CancelCompaction(payload) => {
                write!(f, "cancel compaction for {}", payload.session_id)
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
            Command::SoftCancelTurn(payload) => {
                write!(f, "soft cancel turn for {}", payload.session_id)
            }
            Command::ScheduleAutoCompaction(payload) => {
                write!(f, "schedule auto-compaction for {}", payload.session_id)
            }
            Command::FinishSessionTeardown(payload) => {
                write!(f, "finish session teardown for {}", payload.session_id)
            }
            Command::InitWorkflow(payload) => {
                write!(
                    f,
                    "init workflow '{}' ({})",
                    payload.name, payload.workflow_id
                )
            }
            Command::StartWorkflow(payload) => {
                write!(
                    f,
                    "start workflow '{}' ({})",
                    payload.name, payload.workflow_id
                )
            }
            Command::CancelWorkflow(payload) => {
                write!(f, "cancel workflow {}", payload.workflow_id)
            }
            Command::RerunFromNode(payload) => {
                write!(
                    f,
                    "rerun workflow {} from node '{}'",
                    payload.workflow_id, payload.node_name
                )
            }
            Command::LoadWorkflowPickerEntries(..) => {
                write!(f, "load workflow picker entries")
            }
            Command::RescanJudges(..) => {
                write!(f, "rescan judges")
            }
            Command::CancelPendingJudgeEvaluation(payload) => {
                write!(
                    f,
                    "cancel pending judge evaluation for {}",
                    payload.origin_session_id
                )
            }
            Command::Dynamic(d) => {
                write!(f, "dynamic command '{}'", d.name)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::protocol::SessionId;

    #[rstest::rstest]
    fn command_name_returns_name_for_routable_commands() {
        // Given routable command variants.
        // When calling command_name().
        // Then they return their routing name.
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
        // Given a SessionLoadRequested command.
        let cmd = Command::SessionLoadRequested(SessionLoadRequested {
            session_id: SessionId::new(),
        });

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
    #[case::session(crate::PickerKind::Session, "sessions")]
    fn picker_kind_display(#[case] kind: crate::PickerKind, #[case] expected: &str) {
        assert_eq!(kind.to_string(), expected);
    }

    #[rstest::rstest]
    fn routing_key_returns_runtime_name_for_dynamic() {
        // Given a Dynamic command with a custom name.
        let cmd = Command::Dynamic(DynamicCommand {
            name: "welcome::show".to_owned(),
            payload: serde_json::Value::Null,
        });

        // When calling routing_key().
        let key = cmd.routing_key().expect("should have routing key");

        // Then it returns the runtime name, not the static "dynamic".
        assert_eq!(&*key, "welcome::show");
    }

    #[rstest::rstest]
    fn routing_key_returns_static_name_for_typed_command() {
        // Given a typed command.
        let cmd = Command::CancelStream(CancelStream {
            session_id: SessionId::new(),
        });

        // When calling routing_key().
        let key = cmd.routing_key().expect("should have routing key");

        // Then it returns the static command name.
        assert_eq!(&*key, CancelStream::NAME);
    }
}
