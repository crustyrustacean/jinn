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
use crate::feat::session::protocol::session_fork_requested::SessionForkRequested;
use crate::feat::session::protocol::session_load_requested::SessionLoadRequested;
use crate::feat::session::protocol::submit_history_mutations::SubmitHistoryMutations;
use crate::feat::session::protocol::task_list_updated::TaskListUpdated;
use crate::feat::session::protocol::user_interacted::UserInteracted;
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
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;

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

impl kameo::Actor for SessionPersistenceActor {
    type Args = SessionPersistenceActorDeps;
    type Error = std::convert::Infallible;

    async fn on_start(
        args: Self::Args,
        actor_ref: kameo::prelude::ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        let bus = &args.deps.services.bus;

        // Persistence subscriptions.
        bus.register::<SessionLoadRequested>(actor_ref.clone().recipient::<SessionLoadRequested>())
            .await;
        bus.register::<LoadSessionPickerEntries>(
            actor_ref.clone().recipient::<LoadSessionPickerEntries>(),
        )
        .await;
        bus.register::<SessionForkRequested>(actor_ref.clone().recipient::<SessionForkRequested>())
            .await;

        // Session lifecycle subscriptions.
        bus.register::<EnqueueUserMessage>(actor_ref.clone().recipient::<EnqueueUserMessage>())
            .await;
        bus.register::<SubmitSteeringMessage>(
            actor_ref.clone().recipient::<SubmitSteeringMessage>(),
        )
        .await;
        bus.register::<EnqueueResumeTurn>(actor_ref.clone().recipient::<EnqueueResumeTurn>())
            .await;
        bus.register::<SetChatInputText>(actor_ref.clone().recipient::<SetChatInputText>())
            .await;
        bus.register::<PushChatEntry>(actor_ref.clone().recipient::<PushChatEntry>())
            .await;
        bus.register::<SendMessage>(actor_ref.clone().recipient::<SendMessage>())
            .await;

        // Lifecycle command subscriptions.
        bus.register::<RunSessionSetup>(actor_ref.clone().recipient::<RunSessionSetup>())
            .await;
        bus.register::<RunSessionTeardown>(actor_ref.clone().recipient::<RunSessionTeardown>())
            .await;
        bus.register::<FinishSessionTeardown>(
            actor_ref.clone().recipient::<FinishSessionTeardown>(),
        )
        .await;
        bus.register::<FinishSessionSetup>(actor_ref.clone().recipient::<FinishSessionSetup>())
            .await;
        bus.register::<CancelLifecycleCommand>(
            actor_ref.clone().recipient::<CancelLifecycleCommand>(),
        )
        .await;
        bus.register::<SetSessionCwd>(actor_ref.clone().recipient::<SetSessionCwd>())
            .await;

        bus.register::<PersistSession>(actor_ref.clone().recipient::<PersistSession>())
            .await;
        bus.register::<CloseSession>(actor_ref.clone().recipient::<CloseSession>())
            .await;
        bus.register::<ArchiveSession>(actor_ref.clone().recipient::<ArchiveSession>())
            .await;
        bus.register::<SubmitHistoryMutations>(
            actor_ref.clone().recipient::<SubmitHistoryMutations>(),
        )
        .await;
        bus.register::<MarkSessionInteracted>(
            actor_ref.clone().recipient::<MarkSessionInteracted>(),
        )
        .await;

        // Context-related subscriptions.
        bus.register::<PinChatEntry>(actor_ref.clone().recipient::<PinChatEntry>())
            .await;
        bus.register::<UnpinChatEntry>(actor_ref.clone().recipient::<UnpinChatEntry>())
            .await;
        bus.register::<LoadPersonaPickerEntries>(
            actor_ref.clone().recipient::<LoadPersonaPickerEntries>(),
        )
        .await;

        // Event subscriptions.
        bus.register::<StreamToken>(actor_ref.clone().recipient::<StreamToken>())
            .await;
        bus.register::<StreamCompleted>(actor_ref.clone().recipient::<StreamCompleted>())
            .await;
        bus.register::<ToolUseStarted>(actor_ref.clone().recipient::<ToolUseStarted>())
            .await;
        bus.register::<ToolCallReceived>(actor_ref.clone().recipient::<ToolCallReceived>())
            .await;
        bus.register::<ToolCallStreaming>(actor_ref.clone().recipient::<ToolCallStreaming>())
            .await;
        bus.register::<ToolExecutionCompleted>(
            actor_ref.clone().recipient::<ToolExecutionCompleted>(),
        )
        .await;
        bus.register::<ToolBatchCompleted>(actor_ref.clone().recipient::<ToolBatchCompleted>())
            .await;
        bus.register::<ToolExecutionStarted>(actor_ref.clone().recipient::<ToolExecutionStarted>())
            .await;
        bus.register::<ToolExecutionOutput>(actor_ref.clone().recipient::<ToolExecutionOutput>())
            .await;
        bus.register::<ChatEntryPinChanged>(actor_ref.clone().recipient::<ChatEntryPinChanged>())
            .await;
        bus.register::<TaskListUpdated>(actor_ref.clone().recipient::<TaskListUpdated>())
            .await;
        bus.register::<ModelsRefreshed>(actor_ref.clone().recipient::<ModelsRefreshed>())
            .await;
        bus.register::<SkillsLoaded>(actor_ref.clone().recipient::<SkillsLoaded>())
            .await;
        bus.register::<EnvironmentLoaded>(actor_ref.clone().recipient::<EnvironmentLoaded>())
            .await;
        bus.register::<UserInteracted>(actor_ref.clone().recipient::<UserInteracted>())
            .await;
        bus.register::<ToolsRegistered>(actor_ref.clone().recipient::<ToolsRegistered>())
            .await;
        bus.register::<PromptTemplatesLoaded>(
            actor_ref.clone().recipient::<PromptTemplatesLoaded>(),
        )
        .await;
        bus.register::<PersonasLoaded>(actor_ref.clone().recipient::<PersonasLoaded>())
            .await;
        bus.register::<SessionLoadCompleted>(actor_ref.clone().recipient::<SessionLoadCompleted>())
            .await;

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
// Bridge: impl Message<T> blocks that delegate to old handler methods
//         via a temporary RecordingSink → publish to bus
// ---------------------------------------------------------------------------


// ---------------------------------------------------------------------------
// Command Message impls
// ---------------------------------------------------------------------------

// --- Persistence commands ---
impl kameo::message::Message<SessionLoadRequested> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: SessionLoadRequested,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.on_load_requested(&msg).await;
    }
}

impl kameo::message::Message<LoadSessionPickerEntries> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: LoadSessionPickerEntries,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_load_session_picker_entries(&msg).await;
    }
}

impl kameo::message::Message<SessionForkRequested> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: SessionForkRequested,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.on_session_fork_requested(&msg).await;
    }
}

// --- Session lifecycle commands ---
impl kameo::message::Message<EnqueueUserMessage> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: EnqueueUserMessage,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_enqueue_user_message(&msg).await;
    }
}

impl kameo::message::Message<SubmitSteeringMessage> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: SubmitSteeringMessage,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_submit_steering_message(&msg);
    }
}

impl kameo::message::Message<EnqueueResumeTurn> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: EnqueueResumeTurn,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_enqueue_resume_turn(&msg).await;
    }
}

impl kameo::message::Message<SetChatInputText> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: SetChatInputText,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_set_chat_input_text(&msg);
    }
}

impl kameo::message::Message<PushChatEntry> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: PushChatEntry,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_push_chat_entry(&msg).await;
    }
}

impl kameo::message::Message<SendMessage> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: SendMessage,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_send_message(&msg).await;
    }
}

// --- Lifecycle command messages ---
impl kameo::message::Message<RunSessionSetup> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: RunSessionSetup,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_run_session_setup(&msg).await;
    }
}

impl kameo::message::Message<RunSessionTeardown> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: RunSessionTeardown,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_run_session_teardown(&msg).await;
    }
}

impl kameo::message::Message<FinishSessionTeardown> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: FinishSessionTeardown,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_finish_session_teardown(&msg).await;
    }
}

impl kameo::message::Message<FinishSessionSetup> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: FinishSessionSetup,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_finish_session_setup(&msg).await;
    }
}

impl kameo::message::Message<CancelLifecycleCommand> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: CancelLifecycleCommand,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_cancel_lifecycle_command(&msg);
    }
}

impl kameo::message::Message<SetSessionCwd> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: SetSessionCwd,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_set_session_cwd(&msg).await;
    }
}

impl kameo::message::Message<PersistSession> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: PersistSession,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_persist_session(&msg).await;
    }
}

impl kameo::message::Message<CloseSession> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: CloseSession,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_close_session(&msg).await;
    }
}

impl kameo::message::Message<ArchiveSession> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ArchiveSession,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_archive_session(&msg).await;
    }
}

impl kameo::message::Message<SubmitHistoryMutations> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: SubmitHistoryMutations,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_submit_history_mutations(&msg).await;
    }
}

impl kameo::message::Message<MarkSessionInteracted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: MarkSessionInteracted,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_mark_session_interacted(&msg).await;
    }
}

// --- Context-related commands ---
impl kameo::message::Message<PinChatEntry> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: PinChatEntry,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_pin_chat_entry(&msg).await;
    }
}

impl kameo::message::Message<UnpinChatEntry> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: UnpinChatEntry,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_unpin_chat_entry(&msg).await;
    }
}

impl kameo::message::Message<LoadPersonaPickerEntries> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: LoadPersonaPickerEntries,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_load_persona_picker_entries(&msg).await;
    }
}

// ---------------------------------------------------------------------------
// Event Message impls
// ---------------------------------------------------------------------------

impl kameo::message::Message<StreamToken> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: StreamToken,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.on_stream_token(&msg);
    }
}

impl kameo::message::Message<StreamCompleted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: StreamCompleted,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.on_stream_completed(&msg).await;
    }
}

impl kameo::message::Message<ToolUseStarted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ToolUseStarted,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.on_tool_use_started(&msg);
    }
}

impl kameo::message::Message<ToolCallReceived> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ToolCallReceived,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.on_tool_call_received(&msg);
    }
}

impl kameo::message::Message<ToolCallStreaming> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ToolCallStreaming,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.on_tool_call_streaming(&msg);
    }
}

impl kameo::message::Message<ToolExecutionCompleted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ToolExecutionCompleted,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.on_tool_execution_completed(&msg).await;
    }
}

impl kameo::message::Message<ToolBatchCompleted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ToolBatchCompleted,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.on_tool_batch_completed(&msg).await;
    }
}

impl kameo::message::Message<ToolExecutionStarted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ToolExecutionStarted,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.on_tool_execution_started(&msg);
    }
}

impl kameo::message::Message<ToolExecutionOutput> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ToolExecutionOutput,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.on_tool_execution_output(&msg);
    }
}

impl kameo::message::Message<ModelsRefreshed> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ModelsRefreshed,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.on_models_refreshed(&msg);
    }
}

impl kameo::message::Message<SkillsLoaded> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: SkillsLoaded,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.on_skills_loaded(&msg);
    }
}

impl kameo::message::Message<EnvironmentLoaded> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: EnvironmentLoaded,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.on_environment_loaded(&msg.config).await;
    }
}

impl kameo::message::Message<ChatEntryPinChanged> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ChatEntryPinChanged,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.save_active_session(&msg.session_id).await;
    }
}

impl kameo::message::Message<TaskListUpdated> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: TaskListUpdated,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.save_active_session(&msg.session_id).await;
    }
}

impl kameo::message::Message<UserInteracted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: UserInteracted,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        // No-op: user interaction tracking is handled by other actors.
        let _ = msg;
    }
}

impl kameo::message::Message<ToolsRegistered> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ToolsRegistered,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.on_tools_registered(&msg);
    }
}

impl kameo::message::Message<PromptTemplatesLoaded> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: PromptTemplatesLoaded,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.on_prompt_templates_loaded(&msg);
    }
}

impl kameo::message::Message<PersonasLoaded> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: PersonasLoaded,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.on_personas_loaded(&msg);
    }
}

impl kameo::message::Message<SessionLoadCompleted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: SessionLoadCompleted,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.handle_session_load_completed(&msg).await;
    }
}


//FIXME: disabled during actor migration — tests reference deleted types
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
        });

        // Then the handler was invoked (no panic = dispatch worked).
    }

    #[tokio::test]
    async fn enqueue_user_message_handler_does_not_panic() {
        // Given an actor with an active session.
        let mut actor = test_actor().await;
        let session_id = actor.state.read().session.active_session_id().clone();

        // When handling an EnqueueUserMessage command.
        actor
            .handle_enqueue_user_message(&EnqueueUserMessage {
                session_id: session_id.clone(),
                entry: crate::protocol::ChatEntry::user("hello world"),
            })
            .await;

        // Then the handler was invoked (no panic = dispatch worked).
    }

    #[tokio::test]
    async fn models_refreshed_handler_does_not_panic() {
        // Given an actor.
        let actor = test_actor().await;

        // When handling a ModelsRefreshed event.
        actor.on_models_refreshed(&ModelsRefreshed {
            session_id: crate::protocol::SessionId::new(),
            results: std::collections::HashMap::new(),
            errors: std::collections::HashMap::new(),
        });

        // Then no panic (handler worked).
    }

    #[tokio::test]
    async fn environment_loaded_handler_does_not_panic() {
        // Given an actor.
        let mut actor = test_actor().await;

        // When handling an EnvironmentLoaded event.
        actor
            .on_environment_loaded(&crate::feat::provider_infra::ProvidersConfig {
                providers: vec![],
                aliases: vec![],
                default_provider: None,
            })
            .await;

        // Then no panic (handler worked).
    }
}
