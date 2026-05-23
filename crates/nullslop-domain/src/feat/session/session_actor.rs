//! Session lifecycle and persistence actor - owns session state from input to streaming.
//!
//! This actor is the **sole owner** of session-related state: chat history, input
//! buffers, session phase transitions, tool call state, and streaming tokens. It
//! also handles persisting sessions to disk and restoring them on load.
//!
//! # State ownership
//!
//! This actor is the **sole writer** of the following `AppState` fields:
//! - session history (entries, tool calls, streaming state)
//! - session input buffers
//! - session phase (idle → sending → streaming → idle)
//! - `active_session`, `session_load_guard`
//!
//! # Lock discipline
//!
//! All handlers follow the same pattern: acquire state lock → mutate → release →
//! then emit. Never hold the lock during emission.

mod handlers;
mod helpers;

pub use handlers::lifecycle::setup_running_msg;

use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::chat_input::protocol::command::{
    EnqueueUserMessage, PushChatEntry, SetChatInputText,
};
use crate::feat::context::strategy::token_estimator::TiktokenCounter;
use crate::feat::provider::protocol::command::SendMessage;
use crate::feat::provider::protocol::event::{ModelsRefreshed, StreamCompleted, StreamToken};
use crate::feat::session::protocol::close_session::CloseSession;
use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use crate::feat::session_lifecycle::protocol::command::{
    PersistSession, RunSessionSetup, RunSessionTeardown,
};
use crate::feat::tools_actor::protocol::event::{
    ToolBatchCompleted, ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted,
    ToolExecutionOutput, ToolExecutionStarted, ToolUseStarted,
};
use crate::init::EnvironmentLoaded;
use crate::protocol::{Command, Event};

use super::SessionStoreService;
use crate::SessionForkRequested;
use crate::SessionLoadRequested;

/// Session lifecycle and persistence actor.
///
/// Subscribes to session-related commands and events, mutates [`State`],
/// and emits new commands and events via the [`ActorContext`] message sink.
/// Also persists session snapshots to disk when session state changes.
pub struct SessionPersistenceActor {
    /// Shared application state.
    pub(in crate::feat::session::session_actor) state: State,
    /// Runtime services (user preferences storage for startup config loading).
    pub(in crate::feat::session::session_actor) services: Option<Services>,
    /// The session store service for writing session snapshots.
    pub(in crate::feat::session::session_actor) store: Option<SessionStoreService>,
    /// Token counter for recording token usage in the session ledger.
    pub(in crate::feat::session::session_actor) counter: TiktokenCounter,
    /// Registry of builtin lifecycle handlers.
    pub(in crate::feat::session::session_actor) builtin_registry:
        crate::feat::session_lifecycle::builtin::BuiltinRegistry,
}

/// Dependencies for [`SessionPersistenceActor`].
pub struct SessionPersistenceActorDeps {
    /// Shared application state.
    pub state: State,
    /// Runtime services.
    pub services: Option<Services>,
    /// Session persistence store.
    pub store: Option<SessionStoreService>,
    /// Token counter for usage tracking.
    pub counter: TiktokenCounter,
    /// Registry of builtin lifecycle handlers.
    pub builtin_registry: crate::feat::session_lifecycle::builtin::BuiltinRegistry,
}

impl Actor for SessionPersistenceActor {
    type Message = NoDirectMsg;
    type Deps = SessionPersistenceActorDeps;

    fn activate(deps: Self::Deps, ctx: &mut ActorContext) -> Self {
        // Persistence subscriptions.
        ctx.subscribe_command::<SessionLoadRequested>();
        ctx.subscribe_command::<LoadSessionPickerEntries>();
        ctx.subscribe_command::<SessionForkRequested>();

        // Session lifecycle subscriptions.
        ctx.subscribe_command::<EnqueueUserMessage>();
        ctx.subscribe_command::<SetChatInputText>();
        ctx.subscribe_command::<PushChatEntry>();
        ctx.subscribe_command::<SendMessage>();
        ctx.subscribe_command::<SessionLoadCompleted>();

        // Lifecycle command subscriptions.
        ctx.subscribe_command::<RunSessionSetup>();
        ctx.subscribe_command::<RunSessionTeardown>();
        ctx.subscribe_command::<crate::feat::session_lifecycle::protocol::command::FinishSessionTeardown>();
        ctx.subscribe_command::<PersistSession>();
        ctx.subscribe_command::<CloseSession>();
        ctx.subscribe_command::<crate::feat::session::protocol::archive_session::ArchiveSession>();
        ctx.subscribe_command::<crate::feat::session::protocol::soft_cancel_turn::SoftCancelTurn>();

        // Context-related subscriptions (relocated from PromptAssemblyActor).
        ctx.subscribe_command::<crate::feat::context::protocol::command::PinChatEntry>();
        ctx.subscribe_command::<crate::feat::context::protocol::command::UnpinChatEntry>();
        ctx.subscribe_command::<crate::feat::context::protocol::command::LoadPersonaPickerEntries>(
        );

        // Compaction command subscriptions.
        ctx.subscribe_command::<crate::feat::compaction_actor::protocol::command::BeginCompaction>(
        );
        ctx.subscribe_command::<crate::feat::compaction_actor::protocol::command::EndCompaction>();

        // Event subscriptions.
        ctx.subscribe_event::<StreamToken>();
        ctx.subscribe_event::<StreamCompleted>();
        ctx.subscribe_event::<ToolUseStarted>();
        ctx.subscribe_event::<ToolCallReceived>();
        ctx.subscribe_event::<ToolCallStreaming>();
        ctx.subscribe_event::<ToolExecutionCompleted>();
        ctx.subscribe_event::<ToolBatchCompleted>();
        ctx.subscribe_event::<ToolExecutionStarted>();
        ctx.subscribe_event::<ToolExecutionOutput>();
        ctx.subscribe_event::<crate::feat::context::protocol::event::ChatEntryPinChanged>();
        ctx.subscribe_event::<ModelsRefreshed>();
        ctx.subscribe_event::<EnvironmentLoaded>();

        // Context-related subscriptions (relocated from PromptAssemblyActor).
        ctx.subscribe_event::<crate::feat::tools_actor::protocol::event::ToolsRegistered>();
        ctx.subscribe_event::<crate::feat::provider::protocol::event::PromptTemplatesLoaded>();
        ctx.subscribe_event::<crate::feat::context::protocol::event::PersonasLoaded>();

        ctx.set_description("Session lifecycle and persistence");

        Self {
            state: deps.state,
            services: deps.services,
            store: deps.store,
            counter: deps.counter,
            builtin_registry: deps.builtin_registry,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(event) => self.handle_event(&event, ctx).await,
            ActorEnvelope::Command(cmd) => self.handle_command(&cmd, ctx).await,
            _ => {}
        }
    }

    async fn on_shutdown(&mut self, _ctx: &ActorContext) {
        // Run store shutdown — deletes empty unarchived sessions.
    }
}

impl SessionPersistenceActor {
    /// Dispatches a bus event to the appropriate handler.
    async fn handle_event(&mut self, event: &Event, ctx: &ActorContext) {
        match event {
            Event::StreamToken(payload) => self.on_stream_token(payload),
            Event::StreamCompleted(payload) => self.on_stream_completed(payload, ctx).await,
            Event::ToolUseStarted(payload) => self.on_tool_use_started(payload),
            Event::ToolCallReceived(payload) => self.on_tool_call_received(payload),
            Event::ToolCallStreaming(payload) => self.on_tool_call_streaming(payload),
            Event::ToolExecutionCompleted(payload) => {
                self.on_tool_execution_completed(payload).await;
            }
            Event::ToolBatchCompleted(payload) => {
                self.on_tool_batch_completed(payload, ctx);
            }
            Event::ToolExecutionStarted(payload) => {
                self.on_tool_execution_started(payload);
            }
            Event::ToolExecutionOutput(payload) => {
                self.on_tool_execution_output(payload);
            }
            Event::ModelsRefreshed(payload) => {
                self.on_models_refreshed(payload);
            }
            Event::EnvironmentLoaded(payload) => {
                self.on_environment_loaded(&payload.config, ctx).await;
            }
            Event::ChatEntryPinChanged(payload) => {
                self.save_active_session(&payload.session_id).await;
            }

            // Context-related events (relocated from PromptAssemblyActor).
            Event::ToolsRegistered(payload) => {
                self.on_tools_registered(payload);
            }
            Event::PromptTemplatesLoaded(payload) => {
                self.on_prompt_templates_loaded(payload);
            }
            Event::PersonasLoaded(payload) => {
                self.on_personas_loaded(payload);
            }

            _ => {}
        }
    }

    /// Dispatches a command to the appropriate handler.
    async fn handle_command(&mut self, cmd: &Command, ctx: &ActorContext) {
        match cmd {
            Command::SessionLoadRequested(payload) => self.on_load_requested(payload, ctx).await,
            Command::SessionForkRequested(payload) => {
                self.on_session_fork_requested(payload, ctx).await;
            }
            Command::LoadSessionPickerEntries(payload) => {
                self.handle_load_session_picker_entries(payload).await;
            }
            Command::EnqueueUserMessage(payload) => {
                self.handle_enqueue_user_message(payload, ctx).await;
            }
            Command::SetChatInputText(payload) => self.handle_set_chat_input_text(payload),
            Command::PushChatEntry(payload) => {
                self.handle_push_chat_entry(payload, ctx).await;
            }
            Command::SendMessage(payload) => Self::handle_send_message(payload, ctx),
            Command::SessionLoadCompleted(payload) => {
                self.handle_session_load_completed(payload, ctx).await;
            }
            Command::RunSessionSetup(payload) => {
                self.handle_run_session_setup(payload, ctx).await;
            }
            Command::RunSessionTeardown(payload) => {
                self.handle_run_session_teardown(payload, ctx).await;
            }
            Command::CloseSession(payload) => {
                self.handle_close_session(payload, ctx).await;
            }
            Command::ArchiveSession(payload) => {
                self.handle_archive_session(payload, ctx).await;
            }
            Command::PersistSession(payload) => {
                self.handle_persist_session(payload).await;
            }
            Command::BeginCompaction(payload) => {
                self.handle_begin_compaction(payload, ctx).await;
            }
            Command::EndCompaction(payload) => {
                self.handle_end_compaction(payload, ctx).await;
            }
            Command::SoftCancelTurn(payload) => {
                self.handle_soft_cancel_turn(payload);
            }
            // Context-related commands (relocated from PromptAssemblyActor).
            Command::PinChatEntry(payload) => {
                self.handle_pin_chat_entry(payload, ctx);
            }
            Command::UnpinChatEntry(payload) => {
                self.handle_unpin_chat_entry(payload, ctx);
            }
            Command::LoadPersonaPickerEntries(payload) => {
                self.handle_load_persona_picker_entries(payload);
            }

            Command::FinishSessionTeardown(payload) => {
                self.handle_finish_session_teardown(payload, ctx).await;
            }
            // Commands NOT subscribed to - these should not arrive.
            Command::SendToLlmProvider(..)
            | Command::ExecuteTool(..)
            | Command::ProceedWithShutdown(..)
            | Command::CancelStream(..)
            | Command::CancelCompaction(..)
            | Command::RefreshModels
            | Command::RescanPromptTemplates
            | Command::ExecuteToolBatch(..)
            | Command::RegisterTools(..)
            | Command::ProviderSwitch(..)
            | Command::LoadProviderPickerEntries(..)
            | Command::CancelToolBatch(..)
            | Command::ScanSkills
            | Command::RescanPersonas(..)
            | Command::UpdatePreferences(..)
            | Command::CompactContext(..)
            | Command::EnqueueCompaction(..)
            | Command::StartWorkflow(..)
            | Command::CancelWorkflow(..) => {}
        }
    }
}
