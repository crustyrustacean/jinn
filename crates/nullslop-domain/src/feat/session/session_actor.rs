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
use std::sync::Arc;

use crate::common::actor::{
    Actor, ActorContext, ActorEnvelope, ActorRef, MessageSink, SystemMessage,
};
use crate::common::actor_host::{ActorSpawnResult, spawn_actor};
use crate::common::state::State;
use crate::feat::chat_input::protocol::command::{
    EnqueueUserMessage, PushChatEntry, SetChatInputText,
};
use crate::feat::context::protocol::event::PromptAssembled;
use crate::feat::provider::protocol::command::SendMessage;
use crate::feat::provider::protocol::event::{StreamCompleted, StreamToken};
use crate::feat::session::protocol::load_session_picker_entries::LoadSessionPickerEntries;
use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
use crate::feat::tools_actor::protocol::event::{
    ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted, ToolUseStarted,
};
use crate::protocol::{Command, Event};
use crate::{SessionLoadRequested, SessionSaveRequested};

use super::entries::load_session_picker_items_from_store;

/// Direct message type (unused — the actor only responds to bus commands/events).
pub enum SessionPersistenceDirectMsg {}

/// Session lifecycle and persistence actor.
///
/// Subscribes to session-related commands and events, mutates [`State`],
/// and emits new commands and events via the [`ActorContext`] message sink.
/// Also persists session snapshots to disk on `SessionSaveRequested` events.
pub struct SessionPersistenceActor {
    /// Shared application state.
    pub(super) state: State,
    /// The session store service for writing session snapshots.
    pub(super) store: Option<SessionStoreService>,
}

impl Actor for SessionPersistenceActor {
    type Message = SessionPersistenceDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        // Persistence subscriptions.
        ctx.subscribe_event::<SessionSaveRequested>();
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

        ctx.set_description("Session lifecycle and persistence");

        #[expect(clippy::expect_used, reason = "State is always injected at startup")]
        let state = ctx
            .take_data::<State>()
            .expect("SessionPersistenceActor requires State injection");
        let store = ctx.take_data::<SessionStoreService>();

        Self { state, store }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Event(event) => self.handle_event(&event, ctx),
            ActorEnvelope::Command(cmd) => self.handle_command(&cmd, ctx),
            ActorEnvelope::System(SystemMessage::ApplicationReady) => {
                ctx.announce_started();
            }
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::Direct(_) | ActorEnvelope::Shutdown => {}
        }
    }

    async fn shutdown(self) {}
}

impl SessionPersistenceActor {
    /// Dispatches a bus event to the appropriate handler.
    fn handle_event(&mut self, event: &Event, ctx: &ActorContext) {
        match event {
            Event::SessionSaveRequested { payload } => self.on_save_requested(payload),
            Event::PromptAssembled { payload } => self.handle_prompt_assembled(payload, ctx),
            Event::StreamToken { payload } => self.on_stream_token(payload),
            Event::StreamCompleted { payload } => self.on_stream_completed(payload),
            Event::ToolUseStarted { payload } => self.on_tool_use_started(payload),
            Event::ToolCallReceived { payload } => self.on_tool_call_received(payload),
            Event::ToolCallStreaming { payload } => self.on_tool_call_streaming(payload),
            Event::ToolExecutionCompleted { payload } => {
                self.on_tool_execution_completed(payload);
            }
            _ => {}
        }
    }

    /// Dispatches a command to the appropriate handler.
    fn handle_command(&mut self, cmd: &Command, ctx: &ActorContext) {
        match cmd {
            Command::SessionLoadRequested { payload } => self.on_load_requested(payload, ctx),
            Command::LoadSessionPickerEntries { payload } => {
                self.handle_load_session_picker_entries(payload);
            }
            Command::EnqueueUserMessage { payload } => {
                self.handle_enqueue_user_message(payload, ctx);
            }
            Command::SetChatInputText { payload } => self.handle_set_chat_input_text(payload),
            Command::PushChatEntry { payload } => self.handle_push_chat_entry(payload, ctx),
            Command::SendMessage { payload } => Self::handle_send_message(payload, ctx),
            Command::SessionLoadCompleted { payload } => {
                self.handle_session_load_completed(payload, ctx);
            }
            // Commands NOT subscribed to — these should not arrive.
            Command::AssemblePrompt { .. }
            | Command::SendToLlmProvider { .. }
            | Command::ExecuteTool { .. }
            | Command::ProceedWithShutdown { .. }
            | Command::CancelStream { .. }
            | Command::RefreshModels
            | Command::RescanPromptTemplates
            | Command::ExecuteToolBatch { .. }
            | Command::RegisterTools { .. }
            | Command::ProviderSwitch { .. }
            | Command::LoadProviderPickerEntries { .. }
            | Command::LoadContextStrategyPickerEntries { .. }
            | Command::PinChatEntry { .. }
            | Command::UnpinChatEntry { .. }
            | Command::SwitchPromptStrategy { .. }
            | Command::RestoreStrategyState { .. }
            | Command::ScanSkills => {}
        }
    }

    /// Loads session picker entries from the session store into `AppState`.
    fn handle_load_session_picker_entries(&self, _payload: &LoadSessionPickerEntries) {
        if let Some(ref store) = self.store {
            let mut state = self.state.write();
            load_session_picker_items_from_store(store, &mut state);
        }
    }
}

pub fn spawn_session_actor(
    state: crate::common::state::State,
    session_store: super::SessionStoreService,
    sink: Arc<dyn MessageSink>,
    handle: &tokio::runtime::Handle,
) -> (ActorRef<SessionPersistenceDirectMsg>, ActorSpawnResult) {
    let (tx, rx) = kanal::unbounded::<ActorEnvelope<SessionPersistenceDirectMsg>>();
    let actor_ref = ActorRef::new(tx);
    let mut ctx = ActorContext::new("session-persistence", sink);
    ctx.set_description("Persists session data to disk");
    ctx.set_data(state);
    ctx.set_data(session_store);
    let actor = SessionPersistenceActor::activate(&mut ctx);
    let result = spawn_actor("session-persistence", actor, &actor_ref, rx, ctx, handle);
    (actor_ref, result)
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
    use super::super::session_store::{JsonlSessionStore, SessionStoreService};
    use crate::SessionSaveRequested;
    use crate::common::services::Services;
    use crate::feat::provider::protocol::command::SendMessage;
    use crate::feat::provider::protocol::event::{
        StreamCompleted, StreamCompletedReason, StreamToken,
    };
    use crate::feat::session::protocol::session_load_completed::SessionLoadCompleted;
    use crate::feat::tools_actor::protocol::event::{
        ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted,
    };
    use crate::protocol::{
        ChatEntry, ChatEntryKind, Command, Event, PromptStrategyId, SessionId, ToolCall, ToolResult,
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
        let store = JsonlSessionStore::new_in(dir.path().to_path_buf());
        let service = SessionStoreService::new(Arc::new(store));
        (dir, service)
    }

    /// Creates a save event for testing persistence.
    fn make_save_event(session_id: &SessionId, title: &str) -> Event {
        Event::SessionSaveRequested {
            payload: SessionSaveRequested {
                session_id: session_id.clone(),
                title: title.to_owned(),
                history: vec![ChatEntry::user("hello"), ChatEntry::assistant("world")],
                active_strategy: PromptStrategyId::passthrough(),
                blobs: HashMap::new(),
            },
        }
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

    // =========================================================
    // Persistence tests
    // =========================================================

    #[rstest::rstest]
    #[tokio::test]
    async fn save_event_creates_summary() {
        // Given a SessionPersistenceActor with a JsonlSessionStore in a temp directory.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        // When a SessionSaveRequested event is sent to the actor.
        let session_id = SessionId::new();
        let event = make_save_event(&session_id, "Test Session");
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then load_summaries returns the session.
        let summaries = store_service.load_summaries().expect("load_summaries");
        assert_eq!(summaries.len(), 1);
        let (id, summary, _offset) = &summaries[0];
        assert_eq!(id, &session_id);
        assert_eq!(summary.title, "Test Session");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn save_event_preserves_full_data() {
        // Given a SessionPersistenceActor with a JsonlSessionStore in a temp directory.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        // When a SessionSaveRequested event is sent to the actor.
        let session_id = SessionId::new();
        let event = make_save_event(&session_id, "Test Session");
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // And load_full returns matching data.
        let summaries = store_service.load_summaries().expect("load_summaries");
        let full = store_service
            .load_full(summaries[0].2)
            .expect("load_full")
            .expect("should have session");
        assert_eq!(full.session_id, session_id);
        assert_eq!(full.history.len(), 2);
        assert_eq!(full.title, "Test Session");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_persistence_actor_saves_with_blobs() {
        // Given a SessionPersistenceActor with a store.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        // When a SessionSaveRequested event with blobs is sent.
        let session_id = SessionId::new();
        let event = Event::SessionSaveRequested {
            payload: SessionSaveRequested {
                session_id: session_id.clone(),
                title: "Blob Session".to_owned(),
                history: vec![ChatEntry::user("test")],
                active_strategy: PromptStrategyId::sliding_window(),
                blobs: HashMap::from([(
                    "strategy-state".to_owned(),
                    serde_json::json!({"compaction_count": 5}),
                )]),
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then load_full returns the session with blobs preserved.
        let summaries = store_service.load_summaries().expect("load_summaries");
        let full = store_service
            .load_full(summaries[0].2)
            .expect("load_full")
            .expect("should have session");
        assert_eq!(full.active_strategy, PromptStrategyId::sliding_window());
        assert_eq!(full.blobs["strategy-state"]["compaction_count"], 5);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_persistence_actor_ignores_other_events() {
        // Given a SessionPersistenceActor with a store.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        // When a non-SessionSaveRequested event is sent.
        let event = Event::ActorStarted {
            payload: crate::common::actor::protocol::event::ActorStarted {
                name: "test".to_owned(),
                description: None,
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then no session is saved.
        let summaries = store_service.load_summaries().expect("load_summaries");
        assert!(summaries.is_empty());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_persistence_actor_saves_multiple_sessions() {
        // Given a SessionPersistenceActor with a store.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        // When saving two different sessions.
        let id_a = SessionId::new();
        let id_b = SessionId::new();
        actor
            .handle(ActorEnvelope::Event(make_save_event(&id_a, "A")), &ctx)
            .await;
        actor
            .handle(ActorEnvelope::Event(make_save_event(&id_b, "B")), &ctx)
            .await;

        // Then both sessions are in the store.
        let summaries = store_service.load_summaries().expect("load_summaries");
        assert_eq!(summaries.len(), 2);
        let titles: Vec<&str> = summaries.iter().map(|(_, s, _)| s.title.as_str()).collect();
        assert!(titles.contains(&"A"));
        assert!(titles.contains(&"B"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_save_creates_summary() {
        // Given a SessionPersistenceActor with a store.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        // When saving a session with history and blobs.
        let session_id = SessionId::new();
        let event = Event::SessionSaveRequested {
            payload: SessionSaveRequested {
                session_id: session_id.clone(),
                title: "Round Trip".to_owned(),
                history: vec![
                    ChatEntry::user("first message"),
                    ChatEntry::assistant("first response"),
                    ChatEntry::user("second message"),
                ],
                active_strategy: PromptStrategyId::passthrough(),
                blobs: HashMap::from([(
                    "test_blob".to_owned(),
                    serde_json::json!({"key": "value"}),
                )]),
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then loading summaries shows the saved session.
        let summaries = store_service.load_summaries().expect("load_summaries");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].1.title, "Round Trip");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn session_load_restores_full_data() {
        // Given a SessionPersistenceActor with a store.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let (_dir, store_service) = make_store();
        ctx.set_data(store_service.clone());
        let mut actor = SessionPersistenceActor::activate(&mut ctx);

        // When saving a session with history and blobs.
        let session_id = SessionId::new();
        let event = Event::SessionSaveRequested {
            payload: SessionSaveRequested {
                session_id: session_id.clone(),
                title: "Round Trip".to_owned(),
                history: vec![
                    ChatEntry::user("first message"),
                    ChatEntry::assistant("first response"),
                    ChatEntry::user("second message"),
                ],
                active_strategy: PromptStrategyId::passthrough(),
                blobs: HashMap::from([(
                    "test_blob".to_owned(),
                    serde_json::json!({"key": "value"}),
                )]),
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then loading the full session restores all data.
        let summaries = store_service.load_summaries().expect("load_summaries");
        let full = store_service
            .load_full(summaries[0].2)
            .expect("load_full")
            .expect("should have session");

        assert_eq!(full.session_id, session_id);
        assert_eq!(full.title, "Round Trip");
        assert_eq!(full.history.len(), 3);
        assert_eq!(full.blobs["test_blob"]["key"], "value");
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

        // Save a session to the store.
        let session_id = SessionId::new();
        let event = make_save_event(&session_id, "Test Session");
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // When processing LoadSessionPickerEntries.
        actor
            .handle(
                ActorEnvelope::Command(Command::LoadSessionPickerEntries {
                    payload: LoadSessionPickerEntries,
                }),
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
                ActorEnvelope::Command(Command::EnqueueUserMessage {
                    payload: EnqueueUserMessage {
                        session_id: session_id.clone(),
                        text: "hello".into(),
                    },
                }),
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
                .any(|c| matches!(c, Command::AssemblePrompt { .. }))
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
                ActorEnvelope::Command(Command::EnqueueUserMessage {
                    payload: EnqueueUserMessage {
                        session_id: session_id.clone(),
                        text: "new message".into(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then the emitted AssemblePrompt contains the session history entries
        // (2 pre-existing + the user's own message).
        let cmds = sink.commands();
        let cmd = cmds.iter().find_map(|c| match c {
            Command::AssemblePrompt { payload } => Some(payload.clone()),
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
            guard.provider.active_provider = "lmstudio/my-model".to_owned();
        }

        // When processing EnqueueUserMessage while idle.
        actor
            .handle(
                ActorEnvelope::Command(Command::EnqueueUserMessage {
                    payload: EnqueueUserMessage {
                        session_id: session_id.clone(),
                        text: "hello".into(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then the emitted AssemblePrompt has the active provider as model_name.
        let cmds = sink.commands();
        let cmd = cmds.iter().find_map(|c| match c {
            Command::AssemblePrompt { payload } => Some(payload.clone()),
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
                ActorEnvelope::Command(Command::EnqueueUserMessage {
                    payload: EnqueueUserMessage {
                        session_id: session_id.clone(),
                        text: "hello".into(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then the emitted AssemblePrompt contains the user's message in history.
        let cmds = sink.commands();
        let cmd = cmds.iter().find_map(|c| match c {
            Command::AssemblePrompt { payload } => Some(payload.clone()),
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
                ActorEnvelope::Command(Command::EnqueueUserMessage {
                    payload: EnqueueUserMessage {
                        session_id: session_id.clone(),
                        text: "hello".into(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then a ChatEntrySubmitted event was emitted.
        let events = sink.events();
        let found = events.iter().any(|e| match e {
            Event::ChatEntrySubmitted { payload } => {
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
                ActorEnvelope::Command(Command::EnqueueUserMessage {
                    payload: EnqueueUserMessage {
                        session_id: session_id.clone(),
                        text: "queued msg".into(),
                    },
                }),
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
    async fn enqueue_user_message_sets_input_text_when_busy() {
        // Given a session actor with a sending (but not streaming) session.
        let (mut actor, state, sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        // Set session to sending (dispatched but no tokens yet).
        {
            let mut guard = state.write();
            let session = guard.session_mut_or_create(&session_id);
            session.begin_sending();
        }

        // When processing EnqueueUserMessage while busy.
        actor
            .handle(
                ActorEnvelope::Command(Command::EnqueueUserMessage {
                    payload: EnqueueUserMessage {
                        session_id: session_id.clone(),
                        text: "busy msg".into(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then a SetChatInputText command was emitted with the text.
        let cmds = sink.commands();
        let found = cmds.iter().any(|c| match c {
            Command::SetChatInputText { payload } => payload.text == "busy msg",
            _ => false,
        });
        assert!(found, "expected SetChatInputText with 'busy msg'");
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
                ActorEnvelope::Command(Command::SetChatInputText {
                    payload: SetChatInputText {
                        session_id: session_id.clone(),
                        text: "new text".into(),
                    },
                }),
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
                ActorEnvelope::Command(Command::PushChatEntry {
                    payload: PushChatEntry {
                        session_id: session_id.clone(),
                        entry: entry.clone(),
                    },
                }),
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
            .any(|e| matches!(e, Event::ChatEntrySubmitted { .. }));
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
                ActorEnvelope::Command(Command::SendMessage {
                    payload: SendMessage {
                        session_id: session_id.clone(),
                        text: "hello".into(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then an EnqueueUserMessage command was emitted.
        let cmds = sink.commands();
        let found = cmds.iter().any(|c| match c {
            Command::EnqueueUserMessage { payload } => payload.text == "hello",
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

        // When processing SessionLoadCompleted.
        actor
            .handle(
                ActorEnvelope::Command(Command::SessionLoadCompleted {
                    payload: SessionLoadCompleted {
                        session_id: session_id.clone(),
                        title: "Test Session".into(),
                        history: vec![ChatEntry::user("hello"), ChatEntry::assistant("world")],
                        active_strategy: PromptStrategyId::passthrough(),
                        blobs: HashMap::new(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then the session history is restored.
        {
            let guard = state.read();
            let session = guard.session(&session_id);
            assert_eq!(session.history().len(), 2);
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
        }

        // And RestoreStrategyState and SwitchPromptStrategy were emitted.
        let cmds = sink.commands();
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Command::RestoreStrategyState { .. }))
        );
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Command::SwitchPromptStrategy { .. }))
        );
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
                ActorEnvelope::Event(Event::PromptAssembled {
                    payload: crate::feat::context::protocol::event::PromptAssembled {
                        session_id: session_id.clone(),
                        system_prompt: None,
                        messages: vec![crate::protocol::LlmMessage::User {
                            content: "hello".into(),
                        }],
                    },
                }),
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
                .any(|c| matches!(c, Command::SendToLlmProvider { .. }))
        );
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
        let event = Event::StreamToken {
            payload: StreamToken {
                session_id: session_id.clone(),
                index: 0,
                token: "Hello".to_owned(),
            },
        };
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

        let first = Event::StreamToken {
            payload: StreamToken {
                session_id: session_id.clone(),
                index: 0,
                token: "Hello".to_owned(),
            },
        };
        actor.handle(ActorEnvelope::Event(first), &ctx).await;

        // When processing a second StreamToken.
        let second = Event::StreamToken {
            payload: StreamToken {
                session_id: session_id.clone(),
                index: 1,
                token: " world".to_owned(),
            },
        };
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

        let token = Event::StreamToken {
            payload: StreamToken {
                session_id: session_id.clone(),
                index: 0,
                token: "Hello".to_owned(),
            },
        };
        actor.handle(ActorEnvelope::Event(token), &ctx).await;

        // When processing StreamCompleted.
        let completed = Event::StreamCompleted {
            payload: StreamCompleted {
                session_id: session_id.clone(),
                reason: StreamCompletedReason::Finished,
                assistant_content: None,
                tool_calls: None,
            },
        };
        actor.handle(ActorEnvelope::Event(completed), &ctx).await;

        // Then the session is no longer streaming.
        let guard = state.read();
        let session = guard.session(&session_id);
        assert!(!session.is_streaming());
    }

    // --- ToolCallReceived ---

    #[rstest::rstest]
    #[tokio::test]
    async fn tool_call_received_pushes_tool_call_entry() {
        // Given a session actor.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        // When processing a ToolCallReceived event.
        let event = Event::ToolCallReceived {
            payload: ToolCallReceived {
                session_id: session_id.clone(),
                tool_call: ToolCall {
                    id: "call_1".to_owned(),
                    name: "read_file".to_owned(),
                    arguments: r#"{"path":"/tmp"}"#.to_owned(),
                },
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then the session has a ToolCall entry.
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
        let event = Event::ToolCallStreaming {
            payload: ToolCallStreaming {
                session_id: session_id.clone(),
                index: 0,
                partial_json: r#"{"path":"#.to_owned(),
            },
        };
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
        let event = Event::ToolExecutionCompleted {
            payload: ToolExecutionCompleted {
                session_id: session_id.clone(),
                result: ToolResult {
                    tool_call_id: "call_1".to_owned(),
                    name: "read_file".to_owned(),
                    content: "file contents here".to_owned(),
                    success: true,
                },
            },
        };
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
}
