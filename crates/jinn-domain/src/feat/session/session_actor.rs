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

use crate::common::actor::{ActorContext, RecordingSink};
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
use crate::protocol::{Command, Event};

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

impl SessionPersistenceActor {
    /// Runs a command handler through the old dispatch, then publishes
    /// any emitted commands/events to the bus.
    async fn dispatch_command(&mut self, cmd: Command) {
        let sink = std::sync::Arc::new(RecordingSink::new());
        let ctx = ActorContext::new("session-persistence", sink.clone());
        self.handle_command(&cmd, &ctx).await;
        self.flush_sink_to_bus(sink).await;
    }

    /// Runs an event handler through the old dispatch, then publishes
    /// any emitted commands/events to the bus.
    async fn dispatch_event(&mut self, event: Event) {
        let sink = std::sync::Arc::new(RecordingSink::new());
        self.handle_event(&event).await;
        self.flush_sink_to_bus(sink).await;
    }

    /// Drains a RecordingSink and publishes each message to the bus.
    async fn flush_sink_to_bus(&self, sink: std::sync::Arc<RecordingSink>) {
        for cmd in sink.take_commands() {
            self.publish_command(cmd).await;
        }
        for event in sink.take_events() {
            self.publish_event(event).await;
        }
    }

    /// Publishes a single command to the bus by extracting the inner typed struct.
    async fn publish_command(&self, cmd: Command) {
        match cmd {
            Command::SessionLoadRequested(m) => self.publish(m).await,
            Command::LoadSessionPickerEntries(m) => self.publish(m).await,
            Command::SessionForkRequested(m) => self.publish(m).await,
            Command::EnqueueUserMessage(m) => self.publish(m).await,
            Command::SubmitSteeringMessage(m) => self.publish(m).await,
            Command::EnqueueResumeTurn(m) => self.publish(m).await,
            Command::SetChatInputText(m) => self.publish(m).await,
            Command::PushChatEntry(m) => self.publish(m).await,
            Command::SendMessage(m) => self.publish(m).await,
            Command::RunSessionSetup(m) => self.publish(m).await,
            Command::RunSessionTeardown(m) => self.publish(m).await,
            Command::FinishSessionTeardown(m) => self.publish(m).await,
            Command::FinishSessionSetup(m) => self.publish(m).await,
            Command::CancelLifecycleCommand(m) => self.publish(m).await,
            Command::SetSessionCwd(m) => self.publish(m).await,
            Command::PersistSession(m) => self.publish(m).await,
            Command::CloseSession(m) => self.publish(m).await,
            Command::ArchiveSession(m) => self.publish(m).await,
            Command::SubmitHistoryMutations(m) => self.publish(m).await,
            Command::MarkSessionInteracted(m) => self.publish(m).await,
            Command::PinChatEntry(m) => self.publish(m).await,
            Command::UnpinChatEntry(m) => self.publish(m).await,
            Command::LoadPersonaPickerEntries(m) => self.publish(m).await,
            Command::SendToLlmProvider(m) => self.publish(m).await,
            Command::ExecuteTool(m) => self.publish(m).await,
            Command::ProceedWithShutdown(m) => self.publish(m).await,
            Command::CancelStream(m) => self.publish(m).await,
            Command::RefreshModels => { /* no payload */ }
            Command::RescanPromptTemplates(m) => self.publish(m).await,
            Command::ExecuteToolBatch(m) => self.publish(m).await,
            Command::RegisterTools(m) => self.publish(m).await,
            Command::ProviderSwitch(m) => self.publish(m).await,
            Command::LoadProviderPickerEntries(m) => self.publish(m).await,
            Command::CancelToolBatch(m) => self.publish(m).await,
            Command::ScanSkills(m) => self.publish(m).await,
            Command::ScanContextFiles(m) => self.publish(m).await,
            Command::RescanPersonas(m) => self.publish(m).await,
            Command::UpdatePreferences(m) => self.publish(m).await,
            Command::UpdateAppState(m) => self.publish(m).await,
            Command::LoadCompactionModelPickerEntries(m) => self.publish(m).await,
            Command::TriggerCompaction(m) => self.publish(m).await,
            Command::Dynamic(m) => self.publish(m).await,
            Command::ExecuteWebFetch(m) => self.publish(m).await,
            Command::AttachPlugin(m) => self.publish(m).await,
            Command::DetachPlugin(m) => self.publish(m).await,
            Command::TogglePlugin(m) => self.publish(m).await,
        }
    }

    /// Publishes a single event to the bus by extracting the inner typed struct.
    async fn publish_event(&self, event: Event) {
        match event {
            Event::StreamToken(m) => self.publish(m).await,
            Event::StreamCompleted(m) => self.publish(m).await,
            Event::ToolUseStarted(m) => self.publish(m).await,
            Event::ToolCallReceived(m) => self.publish(m).await,
            Event::ToolCallStreaming(m) => self.publish(m).await,
            Event::ToolExecutionCompleted(m) => self.publish(m).await,
            Event::ToolBatchCompleted(m) => self.publish(m).await,
            Event::ToolExecutionStarted(m) => self.publish(m).await,
            Event::ToolExecutionOutput(m) => self.publish(m).await,
            Event::ModelsRefreshed(m) => self.publish(m).await,
            Event::SkillsLoaded(m) => self.publish(m).await,
            Event::EnvironmentLoaded(m) => self.publish(m).await,
            Event::ChatEntryPinChanged(m) => self.publish(m).await,
            Event::TaskListUpdated(m) => self.publish(m).await,
            Event::SessionLoadCompleted(m) => self.publish(*m).await,
            Event::ToolsRegistered(m) => self.publish(m).await,
            Event::PromptTemplatesLoaded(m) => self.publish(m).await,
            Event::PersonasLoaded(m) => self.publish(m).await,
            Event::SessionDiscoverySettled(m) => self.publish(m).await,
            Event::SessionPhaseChanged(m) => self.publish(m).await,
            Event::HistoryAppended(m) => self.publish(m).await,
            Event::ChatEntrySubmitted(m) => self.publish(m).await,
            Event::SessionSetupCompleted(m) => self.publish(m).await,
            Event::SessionTeardownFinished(m) => self.publish(m).await,
            Event::AppStateUpdated(m) => self.publish(m).await,
            Event::ActorStarting(m) => self.publish(m).await,
            Event::ActorStarted(m) => self.publish(m).await,
            Event::Dynamic(m) => self.publish(m).await,
            Event::UserInteracted(m) => self.publish(m).await,
            Event::ActorShutdownCompleted(m) => self.publish(m).await,
            Event::AllActorsSpawned(m) => self.publish(m).await,
            Event::ActiveSessionChanged(m) => self.publish(m).await,
            Event::SessionCreated(m) => self.publish(m).await,
            Event::SessionCwdChanged(m) => self.publish(m).await,
            Event::SessionClosed(m) => self.publish(m).await,
            Event::SessionArchived(m) => self.publish(m).await,
            Event::HistorySnapshotReady(m) => self.publish(m).await,
            Event::PluginAttached(m) => self.publish(m).await,
            Event::PluginDetached(m) => self.publish(m).await,
            Event::PluginToggled(m) => self.publish(m).await,
            Event::ModelCacheLoaded(m) => self.publish(m).await,
            Event::ProviderSwitched(m) => self.publish(m).await,
            Event::PreferencesUpdated(m) => self.publish(m).await,
            Event::ContextFilesLoaded(m) => self.publish(m).await,
            Event::ContextOverrideChanged(m) => self.publish(m).await,
            Event::KeyDown(m) => self.publish(m).await,
            Event::KeyUp(m) => self.publish(m).await,
            Event::ModeChanged(m) => self.publish(m).await,
        }
    }
}

// ---------------------------------------------------------------------------
// Command Message impls
// ---------------------------------------------------------------------------

macro_rules! command_message {
    ($msg:ty) => {
        impl kameo::message::Message<$msg> for SessionPersistenceActor {
            type Reply = ();

            async fn handle(
                &mut self,
                msg: $msg,
                _ctx: &mut kameo::message::Context<Self, Self::Reply>,
            ) {
                self.dispatch_command(Command::$msg(msg)).await;
            }
        }
    };
}

// Unfortunately, macro paths don't work for enum variant construction.
// The macro expands `Command::$msg(msg)` but the variant name must match
// the type name exactly. For types where the variant name differs from
// the type name, we write the impl manually.

// --- Persistence commands ---
impl kameo::message::Message<SessionLoadRequested> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: SessionLoadRequested,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::SessionLoadRequested(msg))
            .await;
    }
}

impl kameo::message::Message<LoadSessionPickerEntries> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: LoadSessionPickerEntries,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::LoadSessionPickerEntries(msg))
            .await;
    }
}

impl kameo::message::Message<SessionForkRequested> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: SessionForkRequested,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::SessionForkRequested(msg))
            .await;
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
        self.dispatch_command(Command::EnqueueUserMessage(msg))
            .await;
    }
}

impl kameo::message::Message<SubmitSteeringMessage> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: SubmitSteeringMessage,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::SubmitSteeringMessage(msg))
            .await;
    }
}

impl kameo::message::Message<EnqueueResumeTurn> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: EnqueueResumeTurn,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::EnqueueResumeTurn(msg)).await;
    }
}

impl kameo::message::Message<SetChatInputText> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: SetChatInputText,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::SetChatInputText(msg)).await;
    }
}

impl kameo::message::Message<PushChatEntry> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: PushChatEntry,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::PushChatEntry(msg)).await;
    }
}

impl kameo::message::Message<SendMessage> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: SendMessage,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::SendMessage(msg)).await;
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
        self.dispatch_command(Command::RunSessionSetup(msg)).await;
    }
}

impl kameo::message::Message<RunSessionTeardown> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: RunSessionTeardown,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::RunSessionTeardown(msg))
            .await;
    }
}

impl kameo::message::Message<FinishSessionTeardown> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: FinishSessionTeardown,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::FinishSessionTeardown(msg))
            .await;
    }
}

impl kameo::message::Message<FinishSessionSetup> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: FinishSessionSetup,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::FinishSessionSetup(msg))
            .await;
    }
}

impl kameo::message::Message<CancelLifecycleCommand> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: CancelLifecycleCommand,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::CancelLifecycleCommand(msg))
            .await;
    }
}

impl kameo::message::Message<SetSessionCwd> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: SetSessionCwd,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::SetSessionCwd(msg)).await;
    }
}

impl kameo::message::Message<PersistSession> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: PersistSession,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::PersistSession(msg)).await;
    }
}

impl kameo::message::Message<CloseSession> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: CloseSession,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::CloseSession(msg)).await;
    }
}

impl kameo::message::Message<ArchiveSession> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ArchiveSession,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::ArchiveSession(msg)).await;
    }
}

impl kameo::message::Message<SubmitHistoryMutations> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: SubmitHistoryMutations,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::SubmitHistoryMutations(msg))
            .await;
    }
}

impl kameo::message::Message<MarkSessionInteracted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: MarkSessionInteracted,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::MarkSessionInteracted(msg))
            .await;
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
        self.dispatch_command(Command::PinChatEntry(msg)).await;
    }
}

impl kameo::message::Message<UnpinChatEntry> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: UnpinChatEntry,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::UnpinChatEntry(msg)).await;
    }
}

impl kameo::message::Message<LoadPersonaPickerEntries> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: LoadPersonaPickerEntries,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_command(Command::LoadPersonaPickerEntries(msg))
            .await;
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
        self.dispatch_event(Event::StreamToken(msg)).await;
    }
}

impl kameo::message::Message<StreamCompleted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: StreamCompleted,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_event(Event::StreamCompleted(msg)).await;
    }
}

impl kameo::message::Message<ToolUseStarted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ToolUseStarted,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_event(Event::ToolUseStarted(msg)).await;
    }
}

impl kameo::message::Message<ToolCallReceived> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ToolCallReceived,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_event(Event::ToolCallReceived(msg)).await;
    }
}

impl kameo::message::Message<ToolCallStreaming> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ToolCallStreaming,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_event(Event::ToolCallStreaming(msg)).await;
    }
}

impl kameo::message::Message<ToolExecutionCompleted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ToolExecutionCompleted,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_event(Event::ToolExecutionCompleted(msg))
            .await;
    }
}

impl kameo::message::Message<ToolBatchCompleted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ToolBatchCompleted,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_event(Event::ToolBatchCompleted(msg)).await;
    }
}

impl kameo::message::Message<ToolExecutionStarted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ToolExecutionStarted,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_event(Event::ToolExecutionStarted(msg)).await;
    }
}

impl kameo::message::Message<ToolExecutionOutput> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ToolExecutionOutput,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_event(Event::ToolExecutionOutput(msg)).await;
    }
}

impl kameo::message::Message<ModelsRefreshed> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ModelsRefreshed,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_event(Event::ModelsRefreshed(msg)).await;
    }
}

impl kameo::message::Message<SkillsLoaded> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: SkillsLoaded,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_event(Event::SkillsLoaded(msg)).await;
    }
}

impl kameo::message::Message<EnvironmentLoaded> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: EnvironmentLoaded,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_event(Event::EnvironmentLoaded(msg)).await;
    }
}

impl kameo::message::Message<ChatEntryPinChanged> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ChatEntryPinChanged,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_event(Event::ChatEntryPinChanged(msg)).await;
    }
}

impl kameo::message::Message<TaskListUpdated> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: TaskListUpdated,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_event(Event::TaskListUpdated(msg)).await;
    }
}

impl kameo::message::Message<UserInteracted> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: UserInteracted,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_event(Event::UserInteracted(msg)).await;
    }
}

impl kameo::message::Message<ToolsRegistered> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: ToolsRegistered,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_event(Event::ToolsRegistered(msg)).await;
    }
}

impl kameo::message::Message<PromptTemplatesLoaded> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: PromptTemplatesLoaded,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_event(Event::PromptTemplatesLoaded(msg)).await;
    }
}

impl kameo::message::Message<PersonasLoaded> for SessionPersistenceActor {
    type Reply = ();
    async fn handle(
        &mut self,
        msg: PersonasLoaded,
        _ctx: &mut kameo::message::Context<Self, Self::Reply>,
    ) {
        self.dispatch_event(Event::PersonasLoaded(msg)).await;
    }
}

// ---------------------------------------------------------------------------
// Old dispatch methods — kept for bridge compatibility during migration
// ---------------------------------------------------------------------------

impl SessionPersistenceActor {
    /// Dispatches a bus event to the appropriate handler.
    async fn handle_event(&mut self, event: &Event) {
        match event {
            Event::StreamToken(payload) => self.on_stream_token(payload),
            Event::StreamCompleted(payload) => self.on_stream_completed(payload).await,
            Event::ToolUseStarted(payload) => self.on_tool_use_started(payload),
            Event::ToolCallReceived(payload) => self.on_tool_call_received(payload),
            Event::ToolCallStreaming(payload) => self.on_tool_call_streaming(payload),
            Event::ToolExecutionCompleted(payload) => {
                self.on_tool_execution_completed(payload).await;
            }
            Event::ToolBatchCompleted(payload) => {
                self.on_tool_batch_completed(payload).await;
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
                self.on_environment_loaded(&payload.config).await;
            }
            Event::ChatEntryPinChanged(payload) => {
                self.save_active_session(&payload.session_id).await;
            }
            Event::TaskListUpdated(payload) => {
                self.save_active_session(&payload.session_id).await;
            }
            Event::SessionLoadCompleted(payload) => {
                self.handle_session_load_completed(payload).await;
            }
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
            Command::SessionLoadRequested(payload) => self.on_load_requested(payload).await,
            Command::SessionForkRequested(payload) => {
                self.on_session_fork_requested(payload).await;
            }
            Command::LoadSessionPickerEntries(payload) => {
                self.handle_load_session_picker_entries(payload).await;
            }
            Command::EnqueueUserMessage(payload) => {
                self.handle_enqueue_user_message(payload).await;
            }
            Command::EnqueueResumeTurn(payload) => {
                self.handle_enqueue_resume_turn(payload).await;
            }
            Command::SubmitSteeringMessage(payload) => {
                self.handle_submit_steering_message(payload);
            }
            Command::SetChatInputText(payload) => self.handle_set_chat_input_text(payload),
            Command::PushChatEntry(payload) => {
                self.handle_push_chat_entry(payload).await;
            }
            Command::SendMessage(payload) => self.handle_send_message(payload).await,
            Command::RunSessionSetup(payload) => {
                self.handle_run_session_setup(payload).await;
            }
            Command::RunSessionTeardown(payload) => {
                self.handle_run_session_teardown(payload).await;
            }
            Command::CloseSession(payload) => {
                self.handle_close_session(payload).await;
            }
            Command::ArchiveSession(payload) => {
                self.handle_archive_session(payload).await;
            }
            Command::PersistSession(payload) => {
                self.handle_persist_session(payload).await;
            }
            Command::PinChatEntry(payload) => {
                self.handle_pin_chat_entry(payload).await;
            }
            Command::UnpinChatEntry(payload) => {
                self.handle_unpin_chat_entry(payload).await;
            }
            Command::LoadPersonaPickerEntries(payload) => {
                self.handle_load_persona_picker_entries(payload).await;
            }
            Command::FinishSessionTeardown(payload) => {
                self.handle_finish_session_teardown(payload).await;
            }
            Command::FinishSessionSetup(payload) => {
                self.handle_finish_session_setup(payload).await;
            }
            Command::CancelLifecycleCommand(payload) => {
                self.handle_cancel_lifecycle_command(payload);
            }
            Command::SetSessionCwd(payload) => {
                self.handle_set_session_cwd(payload).await;
            }
            Command::MarkSessionInteracted(payload) => {
                self.handle_mark_session_interacted(payload).await;
            }
            Command::SubmitHistoryMutations(payload) => {
                self.handle_submit_history_mutations(payload).await;
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
            | Command::TogglePlugin(..) => {}
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
