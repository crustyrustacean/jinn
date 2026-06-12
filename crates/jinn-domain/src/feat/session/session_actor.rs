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

use kameo::prelude::{Actor, ActorRef, Context, Message};

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::services::bus_service::BusService;
use crate::common::state::State;
use crate::feat::chat_input::protocol::command::{
    EnqueueResumeTurn, EnqueueUserMessage, PushChatEntry, SetChatInputText, SubmitSteeringMessage,
};
use crate::feat::context::protocol::command::{
    LoadPersonaPickerEntries, PinChatEntry, UnpinChatEntry,
};
use crate::feat::context::protocol::event::{ChatEntryPinChanged, PersonasLoaded};
use crate::feat::context::strategy::token_estimator::TiktokenCounter;
use crate::feat::provider::protocol::command::SendMessage;
use crate::feat::provider::protocol::event::{
    ModelsRefreshed, PromptTemplatesLoaded, StreamCompleted, StreamToken,
};
use crate::feat::session::protocol::archive_session::ArchiveSession;
use crate::feat::session::protocol::close_session::CloseSession;
use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;
use crate::feat::session::protocol::mark_session_interacted::MarkSessionInteracted;
use crate::feat::session::protocol::reset_session_history::ResetSessionHistory;
use crate::feat::session::protocol::session_fork_requested::SessionForkRequested;
use crate::feat::session::protocol::session_load_requested::SessionLoadRequested;
use crate::feat::session::protocol::submit_history_mutations::SubmitHistoryMutations;
use crate::feat::session::protocol::task_list_updated::TaskListUpdated;
use crate::feat::session_lifecycle::protocol::command::PersistSession;
use crate::feat::session_lifecycle::protocol::command::{
    CancelLifecycleCommand, FinishSessionSetup, FinishSessionTeardown, RunSessionSetup,
    RunSessionTeardown, SetSessionCwd,
};
use crate::feat::skills::skills_scan_actor::SkillsLoaded;
use crate::feat::tools_actor::protocol::event::{
    ToolBatchCompleted, ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted,
    ToolExecutionOutput, ToolExecutionStarted, ToolUseStarted, ToolsRegistered,
};
use crate::init::EnvironmentLoaded;

/// Session lifecycle and persistence actor.
///
/// Subscribes to session-related commands and events, mutates [`State`],
/// and emits new commands and events via the message bus.
/// Also persists session snapshots to disk when session state changes.
pub struct SessionPersistenceActor {
    state: State,
    /// Runtime services (user preferences storage for startup config loading).
    services: crate::common::services::Services,
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

impl BusPublish for SessionPersistenceActor {
    fn bus(&self) -> &BusService {
        &self.services.bus
    }
}

pub struct SessionPersistenceActorDeps {
    pub deps: ActorDeps,
    pub state: State,
    pub counter: TiktokenCounter,
    pub builtin_registry: crate::feat::session_lifecycle::builtin::BuiltinRegistry,
    pub shell: String,
}

impl Actor for SessionPersistenceActor {
    type Args = SessionPersistenceActorDeps;
    type Error = std::convert::Infallible;

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        let bus = &args.deps.services.bus;

        // Persistence subscriptions.
        bus.subscribe::<SessionLoadRequested, _>(&actor_ref).await;
        bus.subscribe::<LoadSessionPickerEntries, _>(&actor_ref).await;
        bus.subscribe::<SessionForkRequested, _>(&actor_ref).await;

        // Session lifecycle subscriptions.
        bus.subscribe::<EnqueueUserMessage, _>(&actor_ref).await;
        bus.subscribe::<SubmitSteeringMessage, _>(&actor_ref).await;
        bus.subscribe::<EnqueueResumeTurn, _>(&actor_ref).await;
        bus.subscribe::<SetChatInputText, _>(&actor_ref).await;
        bus.subscribe::<PushChatEntry, _>(&actor_ref).await;
        bus.subscribe::<SendMessage, _>(&actor_ref).await;

        // Lifecycle command subscriptions.
        bus.subscribe::<RunSessionSetup, _>(&actor_ref).await;
        bus.subscribe::<RunSessionTeardown, _>(&actor_ref).await;
        bus.subscribe::<FinishSessionTeardown, _>(&actor_ref).await;
        bus.subscribe::<FinishSessionSetup, _>(&actor_ref).await;
        bus.subscribe::<CancelLifecycleCommand, _>(&actor_ref).await;
        bus.subscribe::<SetSessionCwd, _>(&actor_ref).await;

        bus.subscribe::<PersistSession, _>(&actor_ref).await;
        bus.subscribe::<CloseSession, _>(&actor_ref).await;
        bus.subscribe::<ArchiveSession, _>(&actor_ref).await;
        bus.subscribe::<SubmitHistoryMutations, _>(&actor_ref).await;
        bus.subscribe::<MarkSessionInteracted, _>(&actor_ref).await;
        bus.subscribe::<ResetSessionHistory, _>(&actor_ref).await;

        // Context-related subscriptions.
        bus.subscribe::<PinChatEntry, _>(&actor_ref).await;
        bus.subscribe::<UnpinChatEntry, _>(&actor_ref).await;
        bus.subscribe::<LoadPersonaPickerEntries, _>(&actor_ref).await;

        // Event subscriptions.
        bus.subscribe::<StreamToken, _>(&actor_ref).await;
        bus.subscribe::<StreamCompleted, _>(&actor_ref).await;
        bus.subscribe::<ToolUseStarted, _>(&actor_ref).await;
        bus.subscribe::<ToolCallReceived, _>(&actor_ref).await;
        bus.subscribe::<ToolCallStreaming, _>(&actor_ref).await;
        bus.subscribe::<ToolExecutionCompleted, _>(&actor_ref).await;
        bus.subscribe::<ToolBatchCompleted, _>(&actor_ref).await;
        bus.subscribe::<ToolExecutionStarted, _>(&actor_ref).await;
        bus.subscribe::<ToolExecutionOutput, _>(&actor_ref).await;
        bus.subscribe::<ChatEntryPinChanged, _>(&actor_ref).await;
        bus.subscribe::<TaskListUpdated, _>(&actor_ref).await;
        bus.subscribe::<ModelsRefreshed, _>(&actor_ref).await;
        bus.subscribe::<SkillsLoaded, _>(&actor_ref).await;
        bus.subscribe::<EnvironmentLoaded, _>(&actor_ref).await;
        bus.subscribe::<ToolsRegistered, _>(&actor_ref).await;
        bus.subscribe::<PromptTemplatesLoaded, _>(&actor_ref).await;
        bus.subscribe::<PersonasLoaded, _>(&actor_ref).await;

        Ok(Self {
            state: args.state,
            services: args.deps.services,
            counter: args.counter,
            builtin_registry: args.builtin_registry,
            shell: args.shell,
            lifecycle_child: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Message handlers — direct handler calls
// ---------------------------------------------------------------------------

impl Message<SessionLoadRequested> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: SessionLoadRequested, _ctx: &mut Context<Self, Self::Reply>) {
        self.on_load_requested(&msg).await;
    }
}

impl Message<LoadSessionPickerEntries> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: LoadSessionPickerEntries, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_load_session_picker_entries(&msg).await;
    }
}

impl Message<SessionForkRequested> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: SessionForkRequested, _ctx: &mut Context<Self, Self::Reply>) {
        self.on_session_fork_requested(&msg).await;
    }
}

impl Message<EnqueueUserMessage> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: EnqueueUserMessage, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_enqueue_user_message(&msg).await;
    }
}

impl Message<SubmitSteeringMessage> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: SubmitSteeringMessage, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_submit_steering_message(&msg);
    }
}

impl Message<EnqueueResumeTurn> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: EnqueueResumeTurn, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_enqueue_resume_turn(&msg).await;
    }
}

impl Message<SetChatInputText> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: SetChatInputText, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_set_chat_input_text(&msg);
    }
}

impl Message<PushChatEntry> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: PushChatEntry, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_push_chat_entry(&msg).await;
    }
}

impl Message<SendMessage> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: SendMessage, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_send_message(&msg).await;
    }
}

impl Message<RunSessionSetup> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: RunSessionSetup, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_run_session_setup(&msg).await;
    }
}

impl Message<RunSessionTeardown> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: RunSessionTeardown, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_run_session_teardown(&msg).await;
    }
}

impl Message<FinishSessionTeardown> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: FinishSessionTeardown, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_finish_session_teardown(&msg).await;
    }
}

impl Message<FinishSessionSetup> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: FinishSessionSetup, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_finish_session_setup(&msg).await;
    }
}

impl Message<CancelLifecycleCommand> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: CancelLifecycleCommand, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_cancel_lifecycle_command(&msg);
    }
}

impl Message<SetSessionCwd> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: SetSessionCwd, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_set_session_cwd(&msg).await;
    }
}

impl Message<PersistSession> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: PersistSession, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_persist_session(&msg).await;
    }
}

impl Message<CloseSession> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: CloseSession, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_close_session(&msg).await;
    }
}

impl Message<ArchiveSession> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: ArchiveSession, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_archive_session(&msg).await;
    }
}

impl Message<PinChatEntry> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: PinChatEntry, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_pin_chat_entry(&msg).await;
    }
}

impl Message<UnpinChatEntry> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: UnpinChatEntry, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_unpin_chat_entry(&msg).await;
    }
}

impl Message<LoadPersonaPickerEntries> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: LoadPersonaPickerEntries, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_load_persona_picker_entries(&msg).await;
    }
}

impl Message<MarkSessionInteracted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: MarkSessionInteracted, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_mark_session_interacted(&msg).await;
    }
}

impl Message<SubmitHistoryMutations> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: SubmitHistoryMutations, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_submit_history_mutations(&msg).await;
    }
}

impl Message<ResetSessionHistory> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: ResetSessionHistory, _ctx: &mut Context<Self, Self::Reply>) {
        self.handle_reset_session_history(&msg);
    }
}

// Event handlers

impl Message<StreamToken> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: StreamToken, _ctx: &mut Context<Self, Self::Reply>) {
        self.on_stream_token(&msg);
    }
}

impl Message<StreamCompleted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: StreamCompleted, _ctx: &mut Context<Self, Self::Reply>) {
        self.on_stream_completed(&msg).await;
    }
}

impl Message<ToolUseStarted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: ToolUseStarted, _ctx: &mut Context<Self, Self::Reply>) {
        self.on_tool_use_started(&msg);
    }
}

impl Message<ToolCallReceived> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: ToolCallReceived, _ctx: &mut Context<Self, Self::Reply>) {
        self.on_tool_call_received(&msg);
    }
}

impl Message<ToolCallStreaming> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: ToolCallStreaming, _ctx: &mut Context<Self, Self::Reply>) {
        self.on_tool_call_streaming(&msg);
    }
}

impl Message<ToolExecutionCompleted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: ToolExecutionCompleted, _ctx: &mut Context<Self, Self::Reply>) {
        self.on_tool_execution_completed(&msg).await;
    }
}

impl Message<ToolBatchCompleted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: ToolBatchCompleted, _ctx: &mut Context<Self, Self::Reply>) {
        self.on_tool_batch_completed(&msg);
    }
}

impl Message<ToolExecutionStarted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: ToolExecutionStarted, _ctx: &mut Context<Self, Self::Reply>) {
        self.on_tool_execution_started(&msg);
    }
}

impl Message<ToolExecutionOutput> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: ToolExecutionOutput, _ctx: &mut Context<Self, Self::Reply>) {
        self.on_tool_execution_output(&msg);
    }
}

impl Message<ModelsRefreshed> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: ModelsRefreshed, _ctx: &mut Context<Self, Self::Reply>) {
        self.on_models_refreshed(&msg);
    }
}

impl Message<SkillsLoaded> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: SkillsLoaded, _ctx: &mut Context<Self, Self::Reply>) {
        self.on_skills_loaded(&msg);
    }
}

impl Message<EnvironmentLoaded> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: EnvironmentLoaded, _ctx: &mut Context<Self, Self::Reply>) {
        self.on_environment_loaded(&msg.config).await;
    }
}

impl Message<ChatEntryPinChanged> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: ChatEntryPinChanged, _ctx: &mut Context<Self, Self::Reply>) {
        self.save_active_session(&msg.session_id).await;
    }
}

impl Message<TaskListUpdated> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: TaskListUpdated, _ctx: &mut Context<Self, Self::Reply>) {
        self.save_active_session(&msg.session_id).await;
    }
}

impl Message<ToolsRegistered> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: ToolsRegistered, _ctx: &mut Context<Self, Self::Reply>) {
        self.on_tools_registered(&msg);
    }
}

impl Message<PromptTemplatesLoaded> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: PromptTemplatesLoaded, _ctx: &mut Context<Self, Self::Reply>) {
        self.on_prompt_templates_loaded(&msg);
    }
}

impl Message<PersonasLoaded> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(&mut self, msg: PersonasLoaded, _ctx: &mut Context<Self, Self::Reply>) {
        self.on_personas_loaded(&msg);
    }
}

// FIXME: plugin migration — dispatch tests disabled during actor migration
// #[cfg(test)]
#[cfg(any())]
mod dispatch_tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;
    use crate::feat::chat_input::protocol::command::EnqueueUserMessage;
    use crate::feat::provider::protocol::event::StreamToken;
    use crate::feat::tools_actor::protocol::event::ToolCallReceived;

    async fn test_actor() -> SessionPersistenceActor {
        use crate::common::app_state::AppState;
        use crate::common::state::State;
        use crate::feat::context::strategy::token_estimator::TiktokenCounter;

        SessionPersistenceActor {
            state: State::new(AppState::default()),
            services: crate::common::services::Services::new_fake().await,
            counter: TiktokenCounter::o200k_base(),
            builtin_registry: crate::feat::session_lifecycle::builtin::BuiltinRegistry::new(),
            shell: "/bin/sh".to_owned(),
            lifecycle_child: None,
        }
    }

    #[tokio::test]
    async fn stream_token_handler_appends_token() {
        // Given an actor with a session in streaming state.
        let mut actor = test_actor().await;
        let session_id = {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.begin_streaming();
            state.session.active_session_id().clone()
        };

        // When handling a StreamToken event.
        actor.on_stream_token(&StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "hello".to_owned(),
            is_thinking: false,
            dispatched_at: jiff::Timestamp::now(),
        });

        // Then the handler was invoked (session still streaming = no crash).
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(!session.history().is_empty());
    }

    #[tokio::test]
    async fn tool_call_received_handler_does_not_panic() {
        // Given an actor with an active session.
        let actor = test_actor().await;
        let session_id = actor.state.read().session.active_session_id().clone();

        // When handling a ToolCallReceived event.
        actor.on_tool_call_received(&ToolCallReceived {
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
