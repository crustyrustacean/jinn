//! Session lifecycle and persistence actor — owns session state from input to streaming.
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
//! - `active_session`, `session_loading`
//!
//! # Lock discipline
//!
//! All handlers follow the same pattern: acquire state lock → mutate → release →
//! then emit. Never hold the lock during emission.

mod handlers;

use super::SessionStoreService;

use crate::SessionLoadRequested;
use crate::common::actor::{Actor, ActorContext, ActorEnvelope, NoDirectMsg};
use crate::common::services::Services;
use crate::common::state::State;
use crate::feat::chat_input::protocol::command::{
    EnqueueUserMessage, PushChatEntry, SetChatInputText,
};
use crate::feat::context::protocol::command::SwitchPromptStrategy;
use crate::feat::context::protocol::event::PromptAssembled;
use crate::feat::context::strategy::token_estimator::TiktokenCounter;
use crate::feat::provider::protocol::command::SendMessage;
use crate::feat::provider::protocol::event::{ModelsRefreshed, StreamCompleted, StreamToken};
use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use crate::feat::tools_actor::protocol::event::{
    ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted, ToolUseStarted,
};
use crate::init::EnvironmentLoaded;
use crate::protocol::{Command, Event, PromptStrategyId};

/// Session lifecycle and persistence actor.
///
/// Subscribes to session-related commands and events, mutates [`State`],
/// and emits new commands and events via the [`ActorContext`] message sink.
/// Also persists session snapshots to disk when session state changes.
pub struct SessionPersistenceActor {
    /// Shared application state.
    pub(super) state: State,
    /// Runtime services (user preferences storage for startup config loading).
    pub(super) services: Option<Services>,
    /// The session store service for writing session snapshots.
    pub(super) store: Option<SessionStoreService>,
    /// Token counter for recording token usage in the session ledger.
    pub(super) counter: TiktokenCounter,
}

impl Actor for SessionPersistenceActor {
    type Message = NoDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        // Persistence subscriptions.
        ctx.subscribe_command::<SessionLoadRequested>();
        ctx.subscribe_command::<LoadSessionPickerEntries>();

        // Session lifecycle subscriptions.
        ctx.subscribe_command::<EnqueueUserMessage>();
        ctx.subscribe_command::<SetChatInputText>();
        ctx.subscribe_command::<PushChatEntry>();
        ctx.subscribe_command::<SendMessage>();
        ctx.subscribe_command::<SessionLoadCompleted>();

        // Event subscriptions.
        ctx.subscribe_event::<PromptAssembled>();
        ctx.subscribe_event::<StreamToken>();
        ctx.subscribe_event::<StreamCompleted>();
        ctx.subscribe_event::<ToolUseStarted>();
        ctx.subscribe_event::<ToolCallReceived>();
        ctx.subscribe_event::<ToolCallStreaming>();
        ctx.subscribe_event::<ToolExecutionCompleted>();
        ctx.subscribe_event::<ModelsRefreshed>();
        ctx.subscribe_event::<EnvironmentLoaded>();

        ctx.set_description("Session lifecycle and persistence");

        #[expect(clippy::expect_used, reason = "State is always injected at startup")]
        let state = ctx
            .take_data::<State>()
            .expect("SessionPersistenceActor requires State injection");
        let store = ctx.take_data::<SessionStoreService>();
        let services = ctx.take_data::<Services>();
        let counter = ctx
            .take_data::<TiktokenCounter>()
            .unwrap_or_else(TiktokenCounter::o200k_base);

        Self {
            state,
            services,
            store,
            counter,
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(event) => self.handle_event(&event, ctx).await,
            ActorEnvelope::Command(cmd) => self.handle_command(&cmd, ctx).await,
            _ => {}
        }
    }
}

impl SessionPersistenceActor {
    /// Dispatches a bus event to the appropriate handler.
    async fn handle_event(&mut self, event: &Event, ctx: &ActorContext) {
        match event {
            Event::PromptAssembled(payload) => self.handle_prompt_assembled(payload, ctx),
            Event::StreamToken(payload) => self.on_stream_token(payload),
            Event::StreamCompleted(payload) => self.on_stream_completed(payload, ctx).await,
            Event::ToolUseStarted(payload) => self.on_tool_use_started(payload),
            Event::ToolCallReceived(payload) => self.on_tool_call_received(payload),
            Event::ToolCallStreaming(payload) => self.on_tool_call_streaming(payload),
            Event::ToolExecutionCompleted(payload) => {
                self.on_tool_execution_completed(payload).await;
            }
            Event::ModelsRefreshed(payload) => {
                self.on_models_refreshed(payload);
            }
            Event::EnvironmentLoaded(payload) => {
                self.on_environment_loaded(&payload.config, ctx);
            }
            _ => {}
        }
    }

    /// Dispatches a command to the appropriate handler.
    async fn handle_command(&mut self, cmd: &Command, ctx: &ActorContext) {
        match cmd {
            Command::SessionLoadRequested(payload) => self.on_load_requested(payload, ctx).await,
            Command::LoadSessionPickerEntries(payload) => {
                self.handle_load_session_picker_entries(payload).await;
            }
            Command::EnqueueUserMessage(payload) => {
                self.handle_enqueue_user_message(payload, ctx).await;
            }
            Command::SetChatInputText(payload) => self.handle_set_chat_input_text(payload),
            Command::PushChatEntry(payload) => self.handle_push_chat_entry(payload, ctx),
            Command::SendMessage(payload) => Self::handle_send_message(payload, ctx),
            Command::SessionLoadCompleted(payload) => {
                self.handle_session_load_completed(payload, ctx);
            }
            // Commands NOT subscribed to — these should not arrive.
            Command::AssemblePrompt(..)
            | Command::SendToLlmProvider(..)
            | Command::ExecuteTool(..)
            | Command::ProceedWithShutdown(..)
            | Command::CancelStream(..)
            | Command::RefreshModels
            | Command::RescanPromptTemplates
            | Command::ExecuteToolBatch(..)
            | Command::RegisterTools(..)
            | Command::ProviderSwitch(..)
            | Command::LoadProviderPickerEntries(..)
            | Command::LoadContextStrategyPickerEntries(..)
            | Command::PinChatEntry(..)
            | Command::UnpinChatEntry(..)
            | Command::SwitchPromptStrategy(..)
            | Command::RestoreStrategyState(..)
            | Command::CancelToolBatch(..)
            | Command::ScanSkills
            | Command::RescanPersonas(..)
            | Command::LoadPersonaPickerEntries(..)
            | Command::UpdatePreferences(..)
            | Command::ReloadScripts(..) => {}
        }
    }

    /// Loads session picker entries from the session store into `AppState`.
    async fn handle_load_session_picker_entries(&self, _payload: &LoadSessionPickerEntries) {
        if let Some(ref store) = self.store {
            let theme = {
                let state = self.state.read();
                state.frontend.theme.clone()
            };
            let entries =
                crate::feat::session::entries::load_session_entries_from_store(store, &theme).await;
            let mut state = self.state.write();
            state.frontend.session_picker.set_items(entries);
        }
    }

    /// Applies config defaults to the default session profile on startup.
    ///
    /// Loads user preferences and applies `last_model` and `last_strategy`
    /// to the default session, then sends an `UpdatePreferences` command so
    /// the preferences pipeline handles persistence and state sync.
    ///
    /// NOTE: Using `active_session_mut()` is acceptable here because this runs
    /// at startup before any user interaction. There is only one session.
    fn on_environment_loaded(
        &self,
        _config: &crate::feat::provider_infra::ProvidersConfig,
        ctx: &ActorContext,
    ) {
        let Some(ref services) = self.services else {
            return;
        };

        let prefs = match services.user_preferences_storage.load() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(err = ?e, "session-actor failed to load preferences on startup");
                return;
            }
        };

        let session_id;
        {
            let mut state = self.state.write();

            // Apply config defaults to the default session.
            let session = state.active_session_mut();
            if let Some(ref model) = prefs.last_model {
                session.set_model(model.clone());
            }
            if let Some(ref strategy_str) = prefs.last_strategy {
                let strategy_id = PromptStrategyId::new(strategy_str.clone());
                session.switch_strategy(strategy_id.clone());
            }
            session_id = state.session.active_session.clone();
        }

        // Send UpdatePreferences command so the pipeline handles persistence + state sync.
        if let Err(e) = ctx.send_command(Command::UpdatePreferences(crate::feat::preferences_actor::protocol::command::UpdatePreferences {
                updates: vec![
                    crate::feat::preferences_actor::protocol::command::PreferenceUpdate::SetLastModel(prefs.last_model.clone()),
                    crate::feat::preferences_actor::protocol::command::PreferenceUpdate::SetLastStrategy(prefs.last_strategy.clone()),
                ],
            })) {
            tracing::warn!(err = ?e, "session-actor failed to send UpdatePreferences on startup");
        }

        // Emit SwitchPromptStrategy so the context actor initializes the strategy.
        if let Some(ref strategy_str) = prefs.last_strategy {
            let strategy_id = PromptStrategyId::new(strategy_str.clone());
            if let Err(e) = ctx.send_command(Command::SwitchPromptStrategy(SwitchPromptStrategy {
                session_id,
                strategy_id,
            })) {
                tracing::warn!(err = ?e, "session-actor failed to emit SwitchPromptStrategy on startup");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    #[expect(
        clippy::unused_trait_names,
        reason = "Actor trait needed for activate() method resolution"
    )]
    use crate::common::actor::Actor;
    use crate::common::actor::RecordingSink;
    use crate::common::actor::{ActorContext, ActorEnvelope, MessageSink};
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::chat_input::protocol::command::{
        EnqueueUserMessage, PushChatEntry, SetChatInputText,
    };
    // no context imports needed in tests currently
    use super::super::session_store::{SessionStoreService, SqliteSessionStore};
    use crate::common::services::Services;
    use crate::feat::provider::protocol::command::SendMessage;
    use crate::feat::provider::protocol::event::{
        ModelsRefreshed, StreamCompleted, StreamCompletedReason, StreamToken,
    };
    use crate::feat::session::chat_session::ChatSessionState;
    use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
    use crate::feat::tools_actor::protocol::event::{
        ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted, ToolUseStarted,
    };
    use crate::protocol::{
        ChatEntry, ChatEntryKind, Command, Event, SessionId, ToolCall, ToolResult,
    };
    use tempfile::TempDir;

    use super::SessionPersistenceActor;
    use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;

    // --- Test helpers ---

    /// Creates a test context for persistence tests (with State injection).
    fn test_context(sink: Arc<RecordingSink>) -> ActorContext {
        let mut ctx = ActorContext::new("session-persistence", sink as Arc<dyn MessageSink>);
        ctx.set_data(State::new(AppState::default()));
        ctx
    }

    /// Creates a session store in a temp directory.
    fn make_store() -> (TempDir, SessionStoreService) {
        let dir = TempDir::new().expect("temp dir");
        let store = SqliteSessionStore::new_in(dir.path().to_path_buf());
        let service = SessionStoreService::new(Arc::new(store));
        (dir, service)
    }

    /// Creates a lifecycle test actor with a fresh AppState and fake services.
    fn create_lifecycle_actor() -> (
        SessionPersistenceActor,
        State,
        Arc<RecordingSink>,
        ActorContext,
    ) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("session-actor", sink.clone() as Arc<dyn MessageSink>);
        let state = State::new(AppState::default());
        ctx.set_data(state.clone());
        ctx.set_data(Services::new());
        let actor = SessionPersistenceActor::activate(&mut ctx);
        (actor, state, sink, ctx)
    }

    // --- LoadSessionPickerEntries ---

    #[rstest::rstest]
    #[tokio::test]
    async fn load_session_picker_entries_populates_picker() {
        // Given a SessionPersistenceActor with a store containing a saved session.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        // Save a session directly to the store.
        let session_id = SessionId::new();
        let mut session = ChatSessionState::new();
        session.set_session_id(session_id.clone());
        session.set_title("Test Session".to_owned());
        session.push_entry(ChatEntry::user("hello"));
        session.push_entry(ChatEntry::assistant("world"));
        store_service.save(&session).await.expect("save");

        // When processing LoadSessionPickerEntries.
        actor
            .handle(
                ActorEnvelope::Command(Command::LoadSessionPickerEntries(LoadSessionPickerEntries)),
                &ctx,
            )
            .await;

        // Then the session picker has entries.
        let guard = actor.state.read();
        let items = guard.frontend.session_picker.items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Test Session");
    }

    // =========================================================
    // Save behavior tests
    // =========================================================

    #[rstest::rstest]
    #[tokio::test]
    async fn enqueue_user_message_saves_session_to_store() {
        // Given a SessionPersistenceActor with a store.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        ctx.set_data(Services::new());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        let session_id = SessionId::new();

        // When processing EnqueueUserMessage while idle.
        actor
            .handle(
                ActorEnvelope::Command(Command::EnqueueUserMessage(EnqueueUserMessage {
                    session_id: session_id.clone(),
                    text: "hello world".into(),
                })),
                &ctx,
            )
            .await;

        // Then the session is persisted with the title from the first user message.
        let summaries = store_service
            .load_summaries()
            .await
            .expect("load_summaries");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].title, "hello world");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn stream_completed_saves_session_to_store() {
        // Given a SessionPersistenceActor with a store and a streaming session.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        ctx.set_data(Services::new());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        let session_id = SessionId::new();
        {
            let mut guard = actor.state.write();
            let session = guard.session_mut_or_create(&session_id);
            session.set_title("my question".to_owned());
            session.push_entry(ChatEntry::user("my question"));
            session.begin_streaming();
        }

        // When processing StreamCompleted with Finished reason.
        actor
            .handle(
                ActorEnvelope::Event(Event::StreamCompleted(StreamCompleted {
                    session_id: session_id.clone(),
                    reason: StreamCompletedReason::Finished,
                    assistant_content: Some("response".to_owned()),
                    tool_calls: None,
                })),
                &ctx,
            )
            .await;

        // Then the session is persisted.
        let summaries = store_service
            .load_summaries()
            .await
            .expect("load_summaries");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].title, "my question");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn stream_completed_canceled_does_not_save() {
        // Given a SessionPersistenceActor with a store and a streaming session.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        ctx.set_data(Services::new());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        let session_id = SessionId::new();
        {
            let mut guard = actor.state.write();
            guard.session_mut_or_create(&session_id).begin_streaming();
        }

        // When processing StreamCompleted with Canceled reason.
        actor
            .handle(
                ActorEnvelope::Event(Event::StreamCompleted(StreamCompleted {
                    session_id: session_id.clone(),
                    reason: StreamCompletedReason::Canceled,
                    assistant_content: None,
                    tool_calls: None,
                })),
                &ctx,
            )
            .await;

        // Then no session is persisted.
        let summaries = store_service
            .load_summaries()
            .await
            .expect("load_summaries");
        assert!(summaries.is_empty());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn tool_execution_completed_saves_session_to_store() {
        // Given a SessionPersistenceActor with a store.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        ctx.set_data(Services::new());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        let session_id = SessionId::new();
        {
            let mut guard = actor.state.write();
            let session = guard.session_mut_or_create(&session_id);
            session.set_title("do the thing".to_owned());
            session.push_entry(ChatEntry::user("do the thing"));
        }

        // When processing ToolExecutionCompleted.
        actor
            .handle(
                ActorEnvelope::Event(Event::ToolExecutionCompleted(ToolExecutionCompleted {
                    session_id: session_id.clone(),
                    result: ToolResult {
                        tool_call_id: "call_1".to_owned(),
                        name: "bash".to_owned(),
                        content: "ok".to_owned(),
                        success: true,
                    },
                })),
                &ctx,
            )
            .await;

        // Then the session is persisted.
        let summaries = store_service
            .load_summaries()
            .await
            .expect("load_summaries");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].title, "do the thing");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn save_title_derived_from_first_user_message_first_line() {
        // Given a session actor with a store.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        ctx.set_data(Services::new());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        let session_id = SessionId::new();

        // When enqueuing a multi-line user message while idle.
        actor
            .handle(
                ActorEnvelope::Command(Command::EnqueueUserMessage(EnqueueUserMessage {
                    session_id: session_id.clone(),
                    text: "line one\nline two".into(),
                })),
                &ctx,
            )
            .await;

        // Then the title is the first line only.
        let summaries = store_service
            .load_summaries()
            .await
            .expect("load_summaries");
        assert_eq!(summaries[0].title, "line one");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn save_title_is_untitled_when_no_user_messages() {
        // Given a session with no user messages.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        ctx.set_data(Services::new());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        let session_id = SessionId::new();
        {
            let mut guard = actor.state.write();
            guard
                .session_mut_or_create(&session_id)
                .push_entry(ChatEntry::system("system msg"));
        }

        // When saving via tool execution completion.
        actor
            .handle(
                ActorEnvelope::Event(Event::ToolExecutionCompleted(ToolExecutionCompleted {
                    session_id: session_id.clone(),
                    result: ToolResult {
                        tool_call_id: "call_1".to_owned(),
                        name: "bash".to_owned(),
                        content: "ok".to_owned(),
                        success: true,
                    },
                })),
                &ctx,
            )
            .await;

        // Then the title is "Untitled Session".
        let summaries = store_service
            .load_summaries()
            .await
            .expect("load_summaries");
        assert_eq!(summaries[0].title, "Untitled Session");
    }

    // =========================================================
    // Lifecycle tests — migrated from coordinator
    // =========================================================

    // --- EnqueueUserMessage ---

    #[rstest::rstest]
    #[tokio::test]
    async fn enqueue_user_message_dispatches_assemble_prompt_when_idle() {
        // Given a session actor with an idle session.
        let (mut actor, state, sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        // When processing EnqueueUserMessage while idle.
        actor
            .handle(
                ActorEnvelope::Command(Command::EnqueueUserMessage(EnqueueUserMessage {
                    session_id: session_id.clone(),
                    text: "hello".into(),
                })),
                &ctx,
            )
            .await;

        // Then the session is now sending.
        {
            let guard = state.read();
            let session = guard.session(&session_id);
            assert!(session.is_sending());
        }

        // And an AssemblePrompt command was emitted.
        let cmds = sink.commands();
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Command::AssemblePrompt(..)))
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn assemble_prompt_includes_session_history() {
        // Given a session actor with a session that has existing history.
        let (mut actor, state, sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();
        {
            let mut guard = state.write();
            let session = guard.session_mut_or_create(&session_id);
            session.push_entry(ChatEntry::user("previous message"));
            session.push_entry(ChatEntry::assistant("previous reply"));
        }

        // When processing EnqueueUserMessage while idle.
        actor
            .handle(
                ActorEnvelope::Command(Command::EnqueueUserMessage(EnqueueUserMessage {
                    session_id: session_id.clone(),
                    text: "new message".into(),
                })),
                &ctx,
            )
            .await;

        // Then the emitted AssemblePrompt contains the session history entries
        // (2 pre-existing + the user's own message).
        let cmds = sink.commands();
        let cmd = cmds.iter().find_map(|c| match c {
            Command::AssemblePrompt(payload) => Some(payload.clone()),
            _ => None,
        });
        let prompt = cmd.expect("expected AssemblePrompt command");
        assert_eq!(prompt.history.len(), 3);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn assemble_prompt_includes_active_provider_as_model_name() {
        // Given a session actor with an active provider set.
        let (mut actor, state, sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();
        {
            let mut guard = state.write();
            guard
                .session_mut_or_create(&session_id)
                .set_model("lmstudio/my-model".to_owned());
        }

        // When processing EnqueueUserMessage while idle.
        actor
            .handle(
                ActorEnvelope::Command(Command::EnqueueUserMessage(EnqueueUserMessage {
                    session_id: session_id.clone(),
                    text: "hello".into(),
                })),
                &ctx,
            )
            .await;

        // Then the emitted AssemblePrompt has the active provider as model_name.
        let cmds = sink.commands();
        let cmd = cmds.iter().find_map(|c| match c {
            Command::AssemblePrompt(payload) => Some(payload.clone()),
            _ => None,
        });
        let prompt = cmd.expect("expected AssemblePrompt command");
        assert_eq!(prompt.model_name, "lmstudio/my-model");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn first_message_to_new_session_includes_user_entry_in_history() {
        // Given a session actor with a brand new session (no history).
        let (mut actor, _state, sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        // When processing EnqueueUserMessage while idle.
        actor
            .handle(
                ActorEnvelope::Command(Command::EnqueueUserMessage(EnqueueUserMessage {
                    session_id: session_id.clone(),
                    text: "hello".into(),
                })),
                &ctx,
            )
            .await;

        // Then the emitted AssemblePrompt contains the user's message in history.
        let cmds = sink.commands();
        let cmd = cmds.iter().find_map(|c| match c {
            Command::AssemblePrompt(payload) => Some(payload.clone()),
            _ => None,
        });
        let prompt = cmd.expect("expected AssemblePrompt command");
        assert_eq!(
            prompt.history.len(),
            1,
            "history must not be empty for first message"
        );
        assert!(matches!(
            &prompt.history[0].kind,
            ChatEntryKind::User(t) if t == "hello"
        ));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn enqueue_user_message_emits_chat_entry_submitted() {
        // Given a session actor with an idle session.
        let (mut actor, _state, sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        // When processing EnqueueUserMessage while idle.
        actor
            .handle(
                ActorEnvelope::Command(Command::EnqueueUserMessage(EnqueueUserMessage {
                    session_id: session_id.clone(),
                    text: "hello".into(),
                })),
                &ctx,
            )
            .await;

        // Then a ChatEntrySubmitted event was emitted.
        let events = sink.events();
        let found = events.iter().any(|e| match e {
            Event::ChatEntrySubmitted(payload) => {
                payload.session_id == session_id
                    && matches!(&payload.entry.kind, ChatEntryKind::User(t) if t == "hello")
            }
            _ => false,
        });
        assert!(found, "expected ChatEntrySubmitted event with user message");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn enqueue_user_message_queues_when_streaming() {
        // Given a session actor with a streaming session.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        // Set session to streaming.
        {
            let mut guard = state.write();
            let session = guard.session_mut_or_create(&session_id);
            session.begin_sending();
            session.begin_streaming();
        }

        // When processing EnqueueUserMessage while streaming.
        actor
            .handle(
                ActorEnvelope::Command(Command::EnqueueUserMessage(EnqueueUserMessage {
                    session_id: session_id.clone(),
                    text: "queued msg".into(),
                })),
                &ctx,
            )
            .await;

        // Then the message is queued.
        {
            let guard = state.read();
            let session = guard.session(&session_id);
            assert_eq!(session.queue_len(), 1);
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn enqueue_user_message_queues_when_sending() {
        // Given a session actor with a sending (but not streaming) session.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        // Set session to sending (dispatched but no tokens yet).
        {
            let mut guard = state.write();
            let session = guard.session_mut_or_create(&session_id);
            session.begin_sending();
        }

        // When processing EnqueueUserMessage while sending.
        actor
            .handle(
                ActorEnvelope::Command(Command::EnqueueUserMessage(EnqueueUserMessage {
                    session_id: session_id.clone(),
                    text: "busy msg".into(),
                })),
                &ctx,
            )
            .await;

        // Then the message is queued.
        {
            let guard = state.read();
            let session = guard.session(&session_id);
            assert_eq!(session.queue_len(), 1);
        }
    }

    // --- SetChatInputText ---

    #[rstest::rstest]
    #[tokio::test]
    async fn set_chat_input_text_updates_buffer() {
        // Given a session actor with a session.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        // When processing SetChatInputText.
        actor
            .handle(
                ActorEnvelope::Command(Command::SetChatInputText(SetChatInputText {
                    session_id: session_id.clone(),
                    text: "new text".into(),
                })),
                &ctx,
            )
            .await;

        // Then the input buffer has the new text.
        let guard = state.read();
        let session = guard.session(&session_id);
        assert_eq!(session.chat_input().text(), "new text");
    }

    // --- PushChatEntry ---

    #[rstest::rstest]
    #[tokio::test]
    async fn push_chat_entry_adds_to_history() {
        // Given a session actor with a session.
        let (mut actor, state, sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        // When processing PushChatEntry.
        let entry = ChatEntry::user("hello");
        actor
            .handle(
                ActorEnvelope::Command(Command::PushChatEntry(PushChatEntry {
                    session_id: session_id.clone(),
                    entry: entry.clone(),
                })),
                &ctx,
            )
            .await;

        // Then the session history has one entry.
        {
            let guard = state.read();
            let session = guard.session(&session_id);
            assert_eq!(session.history().len(), 1);
        }

        // And a ChatEntrySubmitted event was emitted.
        let events = sink.events();
        let found = events
            .iter()
            .any(|e| matches!(e, Event::ChatEntrySubmitted(..)));
        assert!(found, "expected ChatEntrySubmitted event");
    }

    // --- SendMessage ---

    #[rstest::rstest]
    #[tokio::test]
    async fn send_message_emits_enqueue_user_message() {
        // Given a session actor.
        let (mut actor, _state, sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        // When processing SendMessage.
        actor
            .handle(
                ActorEnvelope::Command(Command::SendMessage(SendMessage {
                    session_id: session_id.clone(),
                    text: "hello".into(),
                })),
                &ctx,
            )
            .await;

        // Then an EnqueueUserMessage command was emitted.
        let cmds = sink.commands();
        let found = cmds.iter().any(|c| match c {
            Command::EnqueueUserMessage(payload) => payload.text == "hello",
            _ => false,
        });
        assert!(found, "expected EnqueueUserMessage command");
    }

    // --- SessionLoadCompleted ---

    #[rstest::rstest]
    #[tokio::test]
    async fn session_load_completed_restores_history() {
        // Given a session actor.
        let (mut actor, state, sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        let mut loaded_session = ChatSessionState::new();
        loaded_session.set_session_id(session_id.clone());
        loaded_session.set_title("Test Session".to_owned());
        loaded_session.set_cwd(std::path::PathBuf::from("/tmp"));
        loaded_session.push_entry(ChatEntry::user("hello"));
        loaded_session.push_entry(ChatEntry::assistant("world"));

        // When processing SessionLoadCompleted.
        actor
            .handle(
                ActorEnvelope::Command(Command::SessionLoadCompleted(SessionLoadCompleted {
                    session: loaded_session,
                })),
                &ctx,
            )
            .await;

        // Then the session history is restored.
        {
            let guard = state.read();
            let session = guard.session(&session_id);
            assert_eq!(session.history().len(), 3);
        }

        // And the active session is set.
        {
            let guard = state.read();
            assert_eq!(guard.session.active_session, session_id);
        }

        // And session_loading is cleared.
        {
            let guard = state.read();
            assert!(!guard.session.session_loading);
            assert!(guard.session.session_load_started_at.is_none());
        }

        // And RestoreStrategyState and SwitchPromptStrategy were emitted.
        let cmds = sink.commands();
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Command::RestoreStrategyState(..)))
        );
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Command::SwitchPromptStrategy(..)))
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_load_completed_pushes_restored_system_message() {
        // Given a session actor.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        let mut loaded_session = ChatSessionState::new();
        loaded_session.set_session_id(session_id.clone());
        loaded_session.set_title("My Chat".to_owned());
        loaded_session.set_cwd(std::path::PathBuf::from("/tmp"));
        loaded_session.push_entry(ChatEntry::user("hello"));

        // When processing SessionLoadCompleted with a title.
        actor
            .handle(
                ActorEnvelope::Command(Command::SessionLoadCompleted(SessionLoadCompleted {
                    session: loaded_session,
                })),
                &ctx,
            )
            .await;

        // Then the last entry is a system message with the title.
        let guard = state.read();
        let session = guard.session(&session_id);
        let last = session.history().last().expect("should have entries");
        match &last.kind {
            ChatEntryKind::System(text) => {
                assert_eq!(text, "Session restored: My Chat");
            }
            other => panic!("expected System entry, got {other:?}"),
        }
    }

    // --- PromptAssembled (event) ---

    #[rstest::rstest]
    #[tokio::test]
    async fn prompt_assembled_transitions_to_streaming() {
        // Given a session actor with a sending session.
        let (mut actor, state, sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        {
            let mut guard = state.write();
            let session = guard.session_mut_or_create(&session_id);
            session.begin_sending();
        }

        // When processing PromptAssembled event.
        actor
            .handle(
                ActorEnvelope::Event(Event::PromptAssembled(
                    crate::feat::context::protocol::event::PromptAssembled {
                        session_id: session_id.clone(),
                        system_prompt: None,
                        messages: vec![crate::protocol::LlmMessage::User {
                            content: "hello".into(),
                        }],
                    },
                )),
                &ctx,
            )
            .await;

        // Then the session is streaming.
        {
            let guard = state.read();
            let session = guard.session(&session_id);
            assert!(session.is_streaming());
        }

        // And a SendToLlmProvider command was emitted.
        let cmds = sink.commands();
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Command::SendToLlmProvider(..)))
        );
    }

    // =========================================================
    // Token counting tests
    // =========================================================

    #[rstest::rstest]
    #[tokio::test]
    async fn prompt_assembled_records_input_tokens_in_ledger() {
        // Given a session actor with a sending session.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        {
            let mut guard = state.write();
            guard.session_mut_or_create(&session_id).begin_sending();
        }

        // When processing PromptAssembled with a user message.
        actor
            .handle(
                ActorEnvelope::Event(Event::PromptAssembled(
                    crate::feat::context::protocol::event::PromptAssembled {
                        session_id: session_id.clone(),
                        system_prompt: None,
                        messages: vec![crate::protocol::LlmMessage::User {
                            content: "hello world".into(),
                        }],
                    },
                )),
                &ctx,
            )
            .await;

        // Then the token ledger has one record with nonzero tokens_sent.
        let guard = state.read();
        let session = guard.session(&session_id);
        let ledger = session.token_ledger();
        assert_eq!(ledger.len(), 1);
        assert!(ledger[0].tokens_sent > 0, "tokens_sent should be nonzero");
        assert_eq!(ledger[0].tokens_received, 0, "tokens_received not yet set");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn prompt_assembled_caches_context_size() {
        // Given a session actor with a sending session.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        {
            let mut guard = state.write();
            guard.session_mut_or_create(&session_id).begin_sending();
        }

        // When processing PromptAssembled.
        actor
            .handle(
                ActorEnvelope::Event(Event::PromptAssembled(
                    crate::feat::context::protocol::event::PromptAssembled {
                        session_id: session_id.clone(),
                        system_prompt: None,
                        messages: vec![crate::protocol::LlmMessage::User {
                            content: "hello world".into(),
                        }],
                    },
                )),
                &ctx,
            )
            .await;

        // Then the context size is cached.
        let guard = state.read();
        let session = guard.session(&session_id);
        let ctx_size = session
            .context_size()
            .expect("context size should be cached");
        assert!(ctx_size > 0, "context size should be nonzero");
        // And it matches the recorded tokens_sent.
        assert_eq!(ctx_size, session.token_ledger()[0].tokens_sent);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn stream_completed_records_output_tokens() {
        // Given a session actor with a streaming session that has a token record.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        // Simulate prompt assembly recording input tokens.
        {
            let mut guard = state.write();
            let session = guard.session_mut_or_create(&session_id);
            session.begin_streaming();
            session.push_token_record(crate::feat::session::token_stats::TokenRecord {
                timestamp: jiff::Timestamp::now(),
                tokens_sent: 100,
                tokens_received: 0,
            });
        }

        // When processing StreamCompleted with assistant content.
        actor
            .handle(
                ActorEnvelope::Event(Event::StreamCompleted(StreamCompleted {
                    session_id: session_id.clone(),
                    reason: StreamCompletedReason::Finished,
                    assistant_content: Some("Hello world response".to_owned()),
                    tool_calls: None,
                })),
                &ctx,
            )
            .await;

        // Then the last token record has nonzero tokens_received.
        let guard = state.read();
        let session = guard.session(&session_id);
        let ledger = session.token_ledger();
        assert_eq!(ledger.len(), 1);
        assert_eq!(ledger[0].tokens_sent, 100);
        assert!(
            ledger[0].tokens_received > 0,
            "tokens_received should be nonzero"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn stream_completed_canceled_does_not_record_output_tokens() {
        // Given a session actor with a streaming session that has a token record.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        {
            let mut guard = state.write();
            let session = guard.session_mut_or_create(&session_id);
            session.begin_streaming();
            session.push_token_record(crate::feat::session::token_stats::TokenRecord {
                timestamp: jiff::Timestamp::now(),
                tokens_sent: 100,
                tokens_received: 0,
            });
        }

        // When processing StreamCompleted with Canceled reason.
        actor
            .handle(
                ActorEnvelope::Event(Event::StreamCompleted(StreamCompleted {
                    session_id: session_id.clone(),
                    reason: StreamCompletedReason::Canceled,
                    assistant_content: None,
                    tool_calls: None,
                })),
                &ctx,
            )
            .await;

        // Then the token record is NOT finalized (tokens_received stays 0).
        let guard = state.read();
        let session = guard.session(&session_id);
        assert_eq!(session.token_ledger()[0].tokens_received, 0);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn stream_completed_without_prior_assembly_does_not_panic() {
        // Given a session actor with a streaming session but NO token record.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        {
            let mut guard = state.write();
            guard.session_mut_or_create(&session_id).begin_streaming();
        }

        // When processing StreamCompleted with content (but no prior PromptAssembled).
        actor
            .handle(
                ActorEnvelope::Event(Event::StreamCompleted(StreamCompleted {
                    session_id: session_id.clone(),
                    reason: StreamCompletedReason::Finished,
                    assistant_content: Some("response".to_owned()),
                    tool_calls: None,
                })),
                &ctx,
            )
            .await;

        // Then no panic occurred and the ledger is still empty.
        let guard = state.read();
        let session = guard.session(&session_id);
        assert!(session.token_ledger().is_empty());
    }

    // =========================================================
    // Lifecycle tests — migrated from projector
    // =========================================================

    // --- StreamToken ---

    #[rstest::rstest]
    #[tokio::test]
    async fn stream_token_creates_assistant_entry() {
        // Given a session actor with default state.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        // When processing a StreamToken event.
        let event = Event::StreamToken(StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "Hello".to_owned(),
            is_thinking: false,
        });
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then the session has an Assistant entry with "Hello".
        let guard = state.read();
        let session = guard.session(&session_id);
        assert!(session.is_streaming());
        assert_eq!(session.history().len(), 1);
        match &session.history()[0].kind {
            ChatEntryKind::Assistant(text) => assert_eq!(text, "Hello"),
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn subsequent_stream_token_appends_to_existing_entry() {
        // Given a session actor with one token already processed.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        let first = Event::StreamToken(StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "Hello".to_owned(),
            is_thinking: false,
        });
        actor.handle(ActorEnvelope::Event(first), &ctx).await;

        // When processing a second StreamToken.
        let second = Event::StreamToken(StreamToken {
            session_id: session_id.clone(),
            index: 1,
            token: " world".to_owned(),
            is_thinking: false,
        });
        actor.handle(ActorEnvelope::Event(second), &ctx).await;

        // Then the text is "Hello world".
        let guard = state.read();
        let session = guard.session(&session_id);
        match &session.history()[0].kind {
            ChatEntryKind::Assistant(text) => assert_eq!(text, "Hello world"),
            other => panic!("expected Assistant, got {other:?}"),
        }
    }

    // --- StreamCompleted ---

    #[rstest::rstest]
    #[tokio::test]
    async fn stream_completed_stops_streaming() {
        // Given a session actor with a streaming session.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        let token = Event::StreamToken(StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "Hello".to_owned(),
            is_thinking: false,
        });
        actor.handle(ActorEnvelope::Event(token), &ctx).await;

        // When processing StreamCompleted.
        let completed = Event::StreamCompleted(StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: None,
            tool_calls: None,
        });
        actor.handle(ActorEnvelope::Event(completed), &ctx).await;

        // Then the session is no longer streaming.
        let guard = state.read();
        let session = guard.session(&session_id);
        assert!(!session.is_streaming());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn stream_completed_tool_use_transitions_to_sending() {
        // Given a session actor with a streaming session.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        let token = Event::StreamToken(StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "Let me check".to_owned(),
            is_thinking: false,
        });
        actor.handle(ActorEnvelope::Event(token), &ctx).await;

        // When processing StreamCompleted with ToolUse reason.
        let completed = Event::StreamCompleted(StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::ToolUse,
            assistant_content: Some("Let me check".to_owned()),
            tool_calls: None,
        });
        actor.handle(ActorEnvelope::Event(completed), &ctx).await;

        // Then the session is no longer streaming but is still sending.
        let guard = state.read();
        let session = guard.session(&session_id);
        assert!(!session.is_streaming());
        assert!(session.is_sending());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn stream_completed_cancelled_pushes_error_entry() {
        // Given a session actor with a streaming session.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        let token = Event::StreamToken(StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "Hello".to_owned(),
            is_thinking: false,
        });
        actor.handle(ActorEnvelope::Event(token), &ctx).await;

        // When processing StreamCompleted with Canceled reason.
        let completed = Event::StreamCompleted(StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Canceled,
            assistant_content: None,
            tool_calls: None,
        });
        actor.handle(ActorEnvelope::Event(completed), &ctx).await;

        // Then the session has an error entry with "Cancelled".
        let guard = state.read();
        let session = guard.session(&session_id);
        let has_cancelled = session
            .history()
            .iter()
            .any(|e| matches!(&e.kind, ChatEntryKind::Error(t) if t == "Cancelled"));
        assert!(has_cancelled, "expected an Error entry with 'Cancelled'");
        // And the session is no longer streaming.
        assert!(!session.is_streaming());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn stream_completed_drains_queued_messages() {
        // Given a session actor with a streaming session that has queued messages.
        let (mut actor, state, sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        // Start streaming.
        let token = Event::StreamToken(StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "Hello".to_owned(),
            is_thinking: false,
        });
        actor.handle(ActorEnvelope::Event(token), &ctx).await;

        // Queue messages.
        {
            let mut guard = state.write();
            let session = guard.session_mut_or_create(&session_id);
            session.enqueue_message("queued1".into());
            session.enqueue_message("queued2".into());
        }

        sink.clear();

        // When processing StreamCompleted with Finished reason.
        let completed = Event::StreamCompleted(StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: None,
            tool_calls: None,
        });
        actor.handle(ActorEnvelope::Event(completed), &ctx).await;

        // Then the queue is drained.
        {
            let guard = state.read();
            let session = guard.session(&session_id);
            assert_eq!(session.queue_len(), 0, "queue should be empty after drain");
        }

        // And an AssemblePrompt command was emitted.
        let cmds = sink.commands();
        let found = cmds
            .iter()
            .any(|c| matches!(c, Command::AssemblePrompt(..)));
        assert!(found, "expected AssemblePrompt after queue drain");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn stream_completed_drains_multiple_messages_as_separate_entries() {
        // Given a streaming session with 3 queued messages.
        let (mut actor, state, sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        let token = Event::StreamToken(StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "Hello".to_owned(),
            is_thinking: false,
        });
        actor.handle(ActorEnvelope::Event(token), &ctx).await;

        {
            let mut guard = state.write();
            let session = guard.session_mut_or_create(&session_id);
            session.enqueue_message("msg1".into());
            session.enqueue_message("msg2".into());
            session.enqueue_message("msg3".into());
        }

        sink.clear();

        // When processing StreamCompleted with Finished reason.
        let completed = Event::StreamCompleted(StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: None,
            tool_calls: None,
        });
        actor.handle(ActorEnvelope::Event(completed), &ctx).await;

        // Then the AssemblePrompt history contains the 3 new User entries.
        let cmds = sink.commands();
        let assemble_cmd = cmds.iter().find_map(|c| match c {
            Command::AssemblePrompt(payload) => Some(payload.clone()),
            _ => None,
        });
        let prompt = assemble_cmd.expect("expected AssemblePrompt");
        // 1 assistant entry (from stream) + 3 user entries (from queue drain).
        let user_entries: Vec<_> = prompt
            .history
            .iter()
            .filter(|e| matches!(&e.kind, ChatEntryKind::User(t) if t == "msg1" || t == "msg2" || t == "msg3"))
            .collect();
        assert_eq!(user_entries.len(), 3, "expected 3 separate user entries");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn stream_completed_does_not_drain_on_tool_use() {
        // Given a streaming session with queued messages.
        let (mut actor, state, sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        let token = Event::StreamToken(StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "Hello".to_owned(),
            is_thinking: false,
        });
        actor.handle(ActorEnvelope::Event(token), &ctx).await;

        {
            let mut guard = state.write();
            let session = guard.session_mut_or_create(&session_id);
            session.enqueue_message("queued".into());
        }

        sink.clear();

        // When processing StreamCompleted with ToolUse reason.
        let completed = Event::StreamCompleted(StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::ToolUse,
            assistant_content: Some("Hello".to_owned()),
            tool_calls: None,
        });
        actor.handle(ActorEnvelope::Event(completed), &ctx).await;

        // Then the queue still has the message.
        {
            let guard = state.read();
            let session = guard.session(&session_id);
            assert_eq!(
                session.queue_len(),
                1,
                "queue should NOT be drained on ToolUse"
            );
        }

        // And no AssemblePrompt was emitted.
        let cmds = sink.commands();
        let found = cmds
            .iter()
            .any(|c| matches!(c, Command::AssemblePrompt(..)));
        assert!(!found, "AssemblePrompt should NOT be emitted on ToolUse");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn stream_completed_drain_empty_queue_is_noop() {
        // Given a streaming session with NO queued messages.
        let (mut actor, state, sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        let token = Event::StreamToken(StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "Hello".to_owned(),
            is_thinking: false,
        });
        actor.handle(ActorEnvelope::Event(token), &ctx).await;

        sink.clear();

        // When processing StreamCompleted with Finished reason.
        let completed = Event::StreamCompleted(StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: None,
            tool_calls: None,
        });
        actor.handle(ActorEnvelope::Event(completed), &ctx).await;

        // Then the session is idle.
        {
            let guard = state.read();
            let session = guard.session(&session_id);
            assert!(
                session.is_idle(),
                "session should be idle after finished with empty queue"
            );
        }

        // And no AssemblePrompt was emitted.
        let cmds = sink.commands();
        let found = cmds
            .iter()
            .any(|c| matches!(c, Command::AssemblePrompt(..)));
        assert!(
            !found,
            "AssemblePrompt should not be emitted when queue is empty"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn stream_completed_drain_emits_chat_entry_submitted_for_each() {
        // Given a streaming session with 2 queued messages.
        let (mut actor, state, sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        let token = Event::StreamToken(StreamToken {
            session_id: session_id.clone(),
            index: 0,
            token: "Hello".to_owned(),
            is_thinking: false,
        });
        actor.handle(ActorEnvelope::Event(token), &ctx).await;

        {
            let mut guard = state.write();
            let session = guard.session_mut_or_create(&session_id);
            session.enqueue_message("q1".into());
            session.enqueue_message("q2".into());
        }

        sink.clear();

        // When processing StreamCompleted with Finished reason.
        let completed = Event::StreamCompleted(StreamCompleted {
            session_id: session_id.clone(),
            reason: StreamCompletedReason::Finished,
            assistant_content: None,
            tool_calls: None,
        });
        actor.handle(ActorEnvelope::Event(completed), &ctx).await;

        // Then ChatEntrySubmitted events were emitted for both queued messages.
        let events = sink.events();
        let submitted: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::ChatEntrySubmitted(payload) => Some(payload.entry.kind.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(submitted.len(), 2, "expected 2 ChatEntrySubmitted events");
        assert!(matches!(&submitted[0], ChatEntryKind::User(t) if t == "q1"));
        assert!(matches!(&submitted[1], ChatEntryKind::User(t) if t == "q2"));
    }

    // --- ToolCallReceived ---

    #[rstest::rstest]
    #[tokio::test]
    async fn tool_call_received_pushes_entry_when_no_prior_start() {
        // Given a session actor with no prior ToolUseStarted.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        // When processing a ToolCallReceived event without a prior ToolUseStarted.
        let event = Event::ToolCallReceived(ToolCallReceived {
            session_id: session_id.clone(),
            tool_call: ToolCall {
                id: "call_1".to_owned(),
                name: "read_file".to_owned(),
                arguments: r#"{"path":"/tmp"}"#.to_owned(),
            },
        });
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then the session has a ToolCall entry (finalize falls back to push).
        let guard = state.read();
        let session = guard.session(&session_id);
        assert_eq!(session.history().len(), 1);
        match &session.history()[0].kind {
            ChatEntryKind::ToolCall {
                id,
                name,
                arguments,
            } => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "read_file");
                assert_eq!(arguments, r#"{"path":"/tmp"}"#);
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn tool_call_received_finalizes_existing_entry() {
        // Given a session actor with a prior ToolUseStarted.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        let start_event = Event::ToolUseStarted(ToolUseStarted {
            session_id: session_id.clone(),
            index: 0,
            id: "call_1".to_owned(),
            name: "read_file".to_owned(),
        });
        actor.handle(ActorEnvelope::Event(start_event), &ctx).await;

        // When processing a ToolCallReceived event for the same tool call.
        let received_event = Event::ToolCallReceived(ToolCallReceived {
            session_id: session_id.clone(),
            tool_call: ToolCall {
                id: "call_1".to_owned(),
                name: "read_file".to_owned(),
                arguments: r#"{"path":"/tmp"}"#.to_owned(),
            },
        });
        actor
            .handle(ActorEnvelope::Event(received_event), &ctx)
            .await;

        // Then there is exactly one ToolCall entry (not duplicated).
        let guard = state.read();
        let session = guard.session(&session_id);
        assert_eq!(session.history().len(), 1);
        match &session.history()[0].kind {
            ChatEntryKind::ToolCall {
                id,
                name,
                arguments,
            } => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "read_file");
                assert_eq!(arguments, r#"{"path":"/tmp"}"#);
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    // --- ToolCallStreaming ---

    #[rstest::rstest]
    #[tokio::test]
    async fn tool_call_streaming_appends_delta() {
        // Given a session actor with a tool call started.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        // Start a tool call using the session directly (simulates ToolUseStarted).
        {
            let mut guard = state.write();
            let session = guard.session_mut_or_create(&session_id);
            session.begin_tool_call(0, "call_1", "read_file");
        }

        // When processing a ToolCallStreaming event.
        let event = Event::ToolCallStreaming(ToolCallStreaming {
            session_id: session_id.clone(),
            index: 0,
            partial_json: r#"{"path":"#.to_owned(),
        });
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then the tool call arguments have the delta appended.
        let guard = state.read();
        let session = guard.session(&session_id);
        match &session.history()[0].kind {
            ChatEntryKind::ToolCall { arguments, .. } => {
                assert_eq!(arguments, r#"{"path":"#);
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    // --- ToolExecutionCompleted ---

    #[rstest::rstest]
    #[tokio::test]
    async fn tool_execution_completed_pushes_tool_result_entry() {
        // Given a session actor.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        // When processing a ToolExecutionCompleted event.
        let event = Event::ToolExecutionCompleted(ToolExecutionCompleted {
            session_id: session_id.clone(),
            result: ToolResult {
                tool_call_id: "call_1".to_owned(),
                name: "read_file".to_owned(),
                content: "file contents here".to_owned(),
                success: true,
            },
        });
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then the session has a ToolResult entry.
        let guard = state.read();
        let session = guard.session(&session_id);
        assert_eq!(session.history().len(), 1);
        match &session.history()[0].kind {
            ChatEntryKind::ToolResult {
                id,
                name,
                content,
                success,
            } => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "read_file");
                assert_eq!(content, "file contents here");
                assert!(success);
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    // --- ModelsRefreshed ---

    #[rstest::rstest]
    #[tokio::test]
    async fn models_refreshed_pushes_success_table_entry() {
        // Given a session actor.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();

        let mut results = HashMap::new();
        results.insert(
            "lmstudio".to_owned(),
            vec!["model-a".to_owned(), "model-b".to_owned()],
        );

        // When processing ModelsRefreshed with results and no errors.
        let session_id = state.read().session.active_session.clone();
        actor
            .handle(
                ActorEnvelope::Event(Event::ModelsRefreshed(ModelsRefreshed {
                    session_id,
                    results,
                    errors: HashMap::new(),
                })),
                &ctx,
            )
            .await;

        // Then the active session has a table entry with the provider.
        let guard = state.read();
        let last = guard
            .active_session()
            .history()
            .last()
            .expect("should have entry");
        match &last.kind {
            ChatEntryKind::Table(data) => {
                assert_eq!(data.headers.len(), 3);
                assert_eq!(data.rows.len(), 1);
                assert_eq!(data.rows[0][0].content, "lmstudio");
                assert_eq!(data.rows[0][1].content, "2");
                assert!(data.rows[0][2].content.contains('\u{2705}'));
            }
            other => panic!("expected Table, got {other:?}"),
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn models_refreshed_pushes_error_table_entry() {
        // Given a session actor.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();

        let mut errors = HashMap::new();
        errors.insert(
            "ollama".to_owned(),
            "HTTP error: connection refused".to_owned(),
        );
        errors.insert("lmstudio".to_owned(), "timeout".to_owned());

        // When processing ModelsRefreshed with only errors.
        let session_id = state.read().session.active_session.clone();
        actor
            .handle(
                ActorEnvelope::Event(Event::ModelsRefreshed(ModelsRefreshed {
                    session_id,
                    results: HashMap::new(),
                    errors,
                })),
                &ctx,
            )
            .await;

        // Then the active session has a table entry with error rows (sorted alphabetically).
        let guard = state.read();
        let last = guard
            .active_session()
            .history()
            .last()
            .expect("should have entry");
        match &last.kind {
            ChatEntryKind::Table(data) => {
                assert_eq!(data.rows.len(), 2);
                // lmstudio comes first (alphabetical).
                assert_eq!(data.rows[0][0].content, "lmstudio");
                assert_eq!(data.rows[0][1].content, "0");
                assert!(data.rows[0][2].content.contains('\u{274c}'));
                assert!(data.rows[0][2].content.contains("timeout"));
                // ollama comes second.
                assert_eq!(data.rows[1][0].content, "ollama");
                assert!(data.rows[1][2].content.contains("connection refused"));
            }
            other => panic!("expected Table, got {other:?}"),
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn models_refreshed_pushes_partial_failure_table_entry() {
        // Given a session actor.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();

        let mut results = HashMap::new();
        results.insert("lmstudio".to_owned(), vec!["model-a".to_owned()]);
        let mut errors = HashMap::new();
        errors.insert("ollama".to_owned(), "connection refused".to_owned());

        // When processing ModelsRefreshed with both results and errors.
        let session_id = state.read().session.active_session.clone();
        actor
            .handle(
                ActorEnvelope::Event(Event::ModelsRefreshed(ModelsRefreshed {
                    session_id,
                    results,
                    errors,
                })),
                &ctx,
            )
            .await;

        // Then the active session has a table entry with both success and error rows.
        let guard = state.read();
        let last = guard
            .active_session()
            .history()
            .last()
            .expect("should have entry");
        match &last.kind {
            ChatEntryKind::Table(data) => {
                assert_eq!(data.rows.len(), 2);
                // lmstudio comes first (alphabetical) — success.
                assert_eq!(data.rows[0][0].content, "lmstudio");
                assert_eq!(data.rows[0][1].content, "1");
                assert!(data.rows[0][2].content.contains('\u{2705}'));
                // ollama comes second — error.
                assert_eq!(data.rows[1][0].content, "ollama");
                assert!(data.rows[1][2].content.contains('\u{274c}'));
                assert!(data.rows[1][2].content.contains("connection refused"));
            }
            other => panic!("expected Table, got {other:?}"),
        }
    }

    // --- CWD validation on session load ---

    #[rstest::rstest]
    #[tokio::test]
    async fn session_load_completed_falls_back_to_default_cwd_when_restored_cwd_is_empty() {
        // Given a session actor.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        let mut loaded_session = ChatSessionState::new();
        loaded_session.set_session_id(session_id.clone());
        loaded_session.set_title("Old Session".to_owned());
        // cwd is empty by default (simulates old snapshot without cwd field).
        loaded_session.push_entry(ChatEntry::user("hello"));

        // When processing SessionLoadCompleted with empty cwd.
        actor
            .handle(
                ActorEnvelope::Command(Command::SessionLoadCompleted(SessionLoadCompleted {
                    session: loaded_session,
                })),
                &ctx,
            )
            .await;

        // Then a warning entry is pushed about the missing CWD.
        {
            let guard = state.read();
            let session = guard.session(&session_id);
            let warning_entries: Vec<_> = session
                .history()
                .iter()
                .filter(|e| {
                    matches!(&e.kind, ChatEntryKind::System(t) if t.contains("Warning: working directory"))
                })
                .collect();
            assert_eq!(
                warning_entries.len(),
                1,
                "expected exactly one warning entry"
            );
            let warning_text = match &warning_entries[0].kind {
                ChatEntryKind::System(t) => t.clone(),
                other => panic!("expected System, got {other:?}"),
            };
            assert!(
                warning_text.contains("(empty)"),
                "warning should mention empty cwd: {warning_text}"
            );
            // And the CWD is set to default_cwd (which is "/" in test defaults).
            assert_eq!(session.cwd(), std::path::PathBuf::from("/"));
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_load_completed_falls_back_to_default_cwd_when_restored_cwd_does_not_exist() {
        // Given a session actor.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        let mut loaded_session = ChatSessionState::new();
        loaded_session.set_session_id(session_id.clone());
        loaded_session.set_title("Missing Dir".to_owned());
        loaded_session.set_cwd(std::path::PathBuf::from("/nonexistent/path/xyz/abc"));
        loaded_session.push_entry(ChatEntry::user("hello"));

        // When processing SessionLoadCompleted with a non-existent CWD.
        actor
            .handle(
                ActorEnvelope::Command(Command::SessionLoadCompleted(SessionLoadCompleted {
                    session: loaded_session,
                })),
                &ctx,
            )
            .await;

        // Then a warning entry is pushed about the non-existent directory.
        {
            let guard = state.read();
            let session = guard.session(&session_id);
            let warning_entries: Vec<_> = session
                .history()
                .iter()
                .filter(|e| {
                    matches!(&e.kind, ChatEntryKind::System(t) if t.contains("Warning: working directory"))
                })
                .collect();
            assert_eq!(
                warning_entries.len(),
                1,
                "expected exactly one warning entry"
            );
            let warning_text = match &warning_entries[0].kind {
                ChatEntryKind::System(t) => t.clone(),
                other => panic!("expected System, got {other:?}"),
            };
            assert!(
                warning_text.contains("/nonexistent/path/xyz/abc"),
                "warning should mention original cwd: {warning_text}"
            );
            // And the CWD is set to default_cwd.
            assert_eq!(session.cwd(), std::path::PathBuf::from("/"));
        }
    }
}
