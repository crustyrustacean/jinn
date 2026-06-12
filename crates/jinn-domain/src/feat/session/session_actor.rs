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
    EnqueueResumeTurn, EnqueueUserMessage, PushChatEntry, SetChatInputText,
};
use crate::feat::context::strategy::token_estimator::TiktokenCounter;
use crate::feat::provider::protocol::command::SendMessage;
use crate::feat::provider::protocol::event::{ModelsRefreshed, StreamCompleted, StreamToken};
use crate::feat::session::protocol::close_session::CloseSession;
use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;
use crate::feat::session::protocol::mark_session_interacted::MarkSessionInteracted;
use crate::feat::session::protocol::reset_session_history::ResetSessionHistory;
use crate::feat::session_lifecycle::protocol::command::{
    PersistSession, RunSessionSetup, RunSessionTeardown,
};

use crate::feat::tools_actor::protocol::event::{
    ToolBatchCompleted, ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted,
    ToolExecutionOutput, ToolExecutionStarted, ToolUseStarted,
};
use crate::init::EnvironmentLoaded;
use crate::protocol::{Command, Event};

use crate::SessionForkRequested;
use crate::SessionLoadRequested;

/// Session lifecycle and persistence actor.
///
/// Subscribes to session-related commands and events, mutates [`State`],
/// and emits new commands and events via the [`ActorContext`] message sink.
/// Also persists session snapshots to disk when session state changes.
pub struct SessionPersistenceActor {
    state: State,
    /// Runtime services (user preferences storage for startup config loading).
    services: Services,
    /// Token counter for recording token usage in the session ledger.
    counter: TiktokenCounter,
    /// Registry of builtin lifecycle handlers.
    builtin_registry: crate::feat::session_lifecycle::builtin::BuiltinRegistry,
    /// Shell captured at startup for running lifecycle commands.
    shell: String,
    /// Handle for cancelling a currently running lifecycle shell process.
    /// `None` when no lifecycle command is in flight. Carries the process-group
    /// PID (for kill) and the inner reader's `AbortHandle` (so aborting it
    /// surfaces the existing "... was cancelled" branch in the outer wrapper).
    lifecycle_child: Option<crate::feat::session_lifecycle::command_runner::LifecycleCancelHandle>,
}

pub struct SessionPersistenceActorDeps {
    /// Shared application state.
    pub state: State,
    /// Runtime services.
    pub services: Services,
    /// Token counter for usage tracking.
    pub counter: TiktokenCounter,
    /// Registry of builtin lifecycle handlers.
    pub builtin_registry: crate::feat::session_lifecycle::builtin::BuiltinRegistry,
    /// Shell captured at startup for running lifecycle commands.
    pub shell: String,
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
        ctx.subscribe_command::<crate::feat::chat_input::protocol::command::SubmitSteeringMessage>(
        );
        ctx.subscribe_command::<EnqueueResumeTurn>();
        ctx.subscribe_command::<SetChatInputText>();
        ctx.subscribe_command::<PushChatEntry>();
        ctx.subscribe_command::<SendMessage>();

        // Lifecycle command subscriptions.
        ctx.subscribe_command::<RunSessionSetup>();
        ctx.subscribe_command::<RunSessionTeardown>();
        ctx.subscribe_command::<crate::feat::session_lifecycle::protocol::command::FinishSessionTeardown>();
        ctx.subscribe_command::<crate::feat::session_lifecycle::protocol::command::FinishSessionSetup>();
        ctx.subscribe_command::<crate::feat::session_lifecycle::protocol::command::CancelLifecycleCommand>();
        ctx.subscribe_command::<crate::feat::session_lifecycle::protocol::command::SetSessionCwd>();

        ctx.subscribe_command::<PersistSession>();
        ctx.subscribe_command::<CloseSession>();
        ctx.subscribe_command::<crate::feat::session::protocol::archive_session::ArchiveSession>();
        ctx.subscribe_command::<crate::feat::session::protocol::submit_history_mutations::SubmitHistoryMutations>();
        ctx.subscribe_command::<MarkSessionInteracted>();
        ctx.subscribe_command::<ResetSessionHistory>();

        // Context-related subscriptions (relocated from PromptAssemblyActor).
        ctx.subscribe_command::<crate::feat::context::protocol::command::PinChatEntry>();
        ctx.subscribe_command::<crate::feat::context::protocol::command::UnpinChatEntry>();
        ctx.subscribe_command::<crate::feat::context::protocol::command::LoadPersonaPickerEntries>(
        );

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
        ctx.subscribe_event::<crate::feat::session::protocol::task_list_updated::TaskListUpdated>();
        ctx.subscribe_event::<ModelsRefreshed>();
        ctx.subscribe_event::<crate::feat::skills::skills_scan_actor::SkillsLoaded>();
        ctx.subscribe_event::<EnvironmentLoaded>();
        ctx.subscribe_event::<crate::feat::session::protocol::user_interacted::UserInteracted>();

        // Context-related subscriptions (relocated from PromptAssemblyActor).
        ctx.subscribe_event::<crate::feat::tools_actor::protocol::event::ToolsRegistered>();
        ctx.subscribe_event::<crate::feat::provider::protocol::event::PromptTemplatesLoaded>();
        ctx.subscribe_event::<crate::feat::context::protocol::event::PersonasLoaded>();

        ctx.set_description("Session lifecycle and persistence");

        Self {
            state: deps.state,
            services: deps.services,
            counter: deps.counter,
            builtin_registry: deps.builtin_registry,
            shell: deps.shell,
            lifecycle_child: None,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(event) => self.handle_event(&event, ctx).await,
            ActorEnvelope::Command(cmd) => {
                tracing::debug!(cmd = %cmd, "session-persistence: received command");
                self.handle_command(&cmd, ctx).await;
            }
            _ => {}
        }
    }

    async fn on_shutdown(&mut self, _ctx: &ActorContext) {
        // Run store shutdown - deletes empty unarchived sessions.
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
                self.on_tool_execution_completed(payload, ctx).await;
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
            Event::SkillsLoaded(payload) => {
                self.on_skills_loaded(payload);
            }
            Event::EnvironmentLoaded(payload) => {
                self.on_environment_loaded(&payload.config, ctx).await;
            }
            Event::ChatEntryPinChanged(payload) => {
                self.save_active_session(&payload.session_id).await;
            }
            Event::TaskListUpdated(payload) => {
                self.save_active_session(&payload.session_id).await;
            }
            Event::SessionLoadCompleted(payload) => {
                self.handle_session_load_completed(payload, ctx).await;
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
            Command::EnqueueResumeTurn(payload) => {
                self.handle_enqueue_resume_turn(payload, ctx).await;
            }
            Command::SubmitSteeringMessage(payload) => {
                self.handle_submit_steering_message(payload);
            }
            Command::SetChatInputText(payload) => self.handle_set_chat_input_text(payload),
            Command::PushChatEntry(payload) => {
                self.handle_push_chat_entry(payload, ctx).await;
            }
            Command::SendMessage(payload) => Self::handle_send_message(payload, ctx),
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
            Command::FinishSessionSetup(payload) => {
                self.handle_finish_session_setup(payload, ctx).await;
            }
            Command::CancelLifecycleCommand(payload) => {
                self.handle_cancel_lifecycle_command(payload, ctx);
            }
            Command::SetSessionCwd(payload) => {
                self.handle_set_session_cwd(payload, ctx);
            }

            Command::MarkSessionInteracted(payload) => {
                self.handle_mark_session_interacted(payload, ctx).await;
            }
            Command::SubmitHistoryMutations(payload) => {
                self.handle_submit_history_mutations(payload, ctx);
            }
            Command::ResetSessionHistory(payload) => {
                self.handle_reset_session_history(payload, ctx);
            }
            // Commands NOT subscribed to - these should not arrive.
            Command::SendToLlmProvider(..)
            | Command::ExecuteTool(..)
            | Command::ProceedWithShutdown(..)
            | Command::CancelStream(..)
            | Command::RefreshModels
            | Command::RescanPromptTemplates(..)
            | Command::ExecuteToolBatch(..)
            | Command::RegisterTools(..)
            | Command::ProviderSwitch(..)
            | Command::LoadProviderPickerEntries(..)
            | Command::CancelToolBatch(..)
            | Command::ScanSkills(..)
            | Command::ScanContextFiles(..)
            | Command::RescanPersonas(..)
            | Command::UpdatePreferences(..)
            | Command::UpdateAppState(..)
            | Command::LoadCompactionModelPickerEntries(..)
            | Command::TriggerCompaction(..)
            | Command::Dynamic(..)
            | Command::ExecuteWebFetch(..)
            | Command::AttachPlugin(..)
            | Command::DetachPlugin(..)
            | Command::TogglePlugin(..)
            | Command::RegisterPluginTools(..)
            | Command::SetManagedSession(..) => {}
        }
    }
}

#[cfg(test)]
mod dispatch_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use crate::common::actor::ActorEnvelope;
    use crate::feat::chat_input::protocol::command::EnqueueUserMessage;
    use crate::feat::provider::protocol::event::StreamToken;
    use crate::feat::tools_actor::protocol::event::ToolCallReceived;
    use crate::protocol::{Command, Event};

    fn test_actor() -> SessionPersistenceActor {
        use crate::common::app_state::AppState;
        use crate::common::state::State;
        use crate::feat::context::strategy::token_estimator::TiktokenCounter;

        SessionPersistenceActor {
            state: State::new(AppState::default()),
            services: crate::common::services::Services::new(),
            counter: TiktokenCounter::o200k_base(),
            builtin_registry: crate::feat::session_lifecycle::builtin::BuiltinRegistry::new(),
            shell: "/bin/sh".to_owned(),
            lifecycle_child: None,
        }
    }

    fn test_context() -> (
        std::sync::Arc<crate::common::actor::RecordingSink>,
        crate::common::actor::ActorContext,
    ) {
        let sink = std::sync::Arc::new(crate::common::actor::RecordingSink::new());
        let ctx = crate::common::actor::ActorContext::new("test-session-actor", sink.clone());
        (sink, ctx)
    }

    #[tokio::test]
    async fn handle_event_stream_token_dispatches_to_handler() {
        // Given an actor with a session in streaming state.
        let mut actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When handling a StreamToken event via the dispatch path.
        let event = Event::StreamToken(StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "hello".to_owned(),
            is_thinking: false,
            dispatched_at: jiff::Timestamp::now(),
        });
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then the handler was invoked (session still streaming = no crash).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        // Verify the token was appended (handler ran successfully).
        assert!(!session.history().is_empty());
    }

    #[tokio::test]
    async fn handle_event_tool_call_received_dispatches_to_handler() {
        // Given an actor with an active session.
        let mut actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = actor.state.read().session.active_session_id().clone();

        // When handling a ToolCallReceived event via the dispatch path.
        let event = Event::ToolCallReceived(ToolCallReceived {
            session_id: session_id.clone(),
            tool_call: jinn_provider::ToolCall {
                id: "tc_1".to_owned(),
                name: "bash".to_owned(),
                arguments: "{}".to_owned(),
            },
            dispatched_at: jiff::Timestamp::now(),
        });
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then the handler was invoked (no panic = dispatch worked).
    }

    #[tokio::test]
    async fn handle_command_enqueue_user_message_dispatches_to_handler() {
        // Given an actor with an active session.
        let mut actor = test_actor();
        let (_sink, ctx) = test_context();
        let session_id = actor.state.read().session.active_session_id().clone();

        // When handling an EnqueueUserMessage command via the dispatch path.
        let cmd = Command::EnqueueUserMessage(EnqueueUserMessage {
            session_id: session_id.clone(),
            entry: crate::protocol::ChatEntry::user("hello world"),
        });
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then the handler was invoked (no panic = dispatch worked).
    }

    #[tokio::test]
    async fn handle_event_models_refreshed_dispatches_to_handler() {
        // Given an actor.
        let mut actor = test_actor();
        let (_sink, ctx) = test_context();

        // When handling a ModelsRefreshed event via the dispatch path.
        let event = Event::ModelsRefreshed(ModelsRefreshed {
            session_id: crate::protocol::SessionId::new(),
            results: std::collections::HashMap::new(),
            errors: std::collections::HashMap::new(),
        });
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then no panic (dispatch to on_models_refreshed worked).
    }

    #[tokio::test]
    async fn handle_event_environment_loaded_dispatches_to_handler() {
        // Given an actor.
        let mut actor = test_actor();
        let (_sink, ctx) = test_context();

        // When handling an EnvironmentLoaded event via the dispatch path.
        let event = Event::EnvironmentLoaded(EnvironmentLoaded {
            config: crate::feat::provider_infra::ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
                alloys: vec![],
            },
        });
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then no panic (dispatch to on_environment_loaded worked).
    }
}
