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

use jiff::Timestamp;
use nullslop_actor::{Actor, ActorContext, ActorEnvelope, SystemMessage};
use nullslop_component::State;
use nullslop_protocol::chat_input::{
    ChatEntrySubmitted, EnqueueUserMessage, PushChatEntry, SetChatInputText,
};
use nullslop_protocol::context::{
    AssemblePrompt, PromptAssembled, RestoreStrategyState, SwitchPromptStrategy,
};
use nullslop_protocol::provider::{SendMessage, SendToLlmProvider, StreamCompleted, StreamToken};
use nullslop_protocol::session::SessionLoadCompleted;
use nullslop_protocol::tool::{
    PushToolResult, ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted, ToolUseStarted,
};
use nullslop_protocol::{
    ChatEntry, Command, Event, PromptStrategyId, SessionLoadRequested, SessionSaveRequested,
};
use nullslop_session::{PersistedSession, SessionStoreService};

/// Direct message type (unused — the actor only responds to bus commands/events).
pub enum SessionPersistenceDirectMsg {}

/// Decision returned after inspecting session state in `EnqueueUserMessage`.
enum EnqueueAction {
    /// Session is idle — dispatch prompt assembly.
    AssemblePrompt,
    /// Session is streaming — message was queued.
    Queued,
    /// Session is busy (sending or assembling) — put text back in the input box.
    SetInputText(String),
}

/// Session lifecycle and persistence actor.
///
/// Subscribes to session-related commands and events, mutates [`State`],
/// and emits new commands and events via the [`ActorContext`] message sink.
/// Also persists session snapshots to disk on `SessionSaveRequested` events.
pub struct SessionPersistenceActor {
    /// Shared application state.
    state: State,
    /// The session store service for writing session snapshots.
    store: Option<SessionStoreService>,
}

impl Actor for SessionPersistenceActor {
    type Message = SessionPersistenceDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        // Persistence subscriptions.
        ctx.subscribe_event::<SessionSaveRequested>();
        ctx.subscribe_command::<SessionLoadRequested>();

        // Session lifecycle subscriptions.
        ctx.subscribe_command::<EnqueueUserMessage>();
        ctx.subscribe_command::<SetChatInputText>();
        ctx.subscribe_command::<PushChatEntry>();
        ctx.subscribe_command::<PushToolResult>();
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
            Command::EnqueueUserMessage { payload } => {
                self.handle_enqueue_user_message(payload, ctx);
            }
            Command::SetChatInputText { payload } => self.handle_set_chat_input_text(payload),
            Command::PushChatEntry { payload } => self.handle_push_chat_entry(payload, ctx),
            Command::PushToolResult { payload } => self.handle_push_tool_result(payload),
            Command::SendMessage { payload } => self.handle_send_message(payload, ctx),
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
            | Command::LoadPickerEntries { .. }
            | Command::PinChatEntry { .. }
            | Command::UnpinChatEntry { .. }
            | Command::SwitchPromptStrategy { .. }
            | Command::RestoreStrategyState { .. } => {}
        }
    }

    // --- Persistence handlers ---

    /// Constructs a [`PersistedSession`] from the event payload and saves it.
    ///
    /// Errors are logged as warnings — persistence failure must not break
    /// the user experience.
    fn on_save_requested(&mut self, evt: &SessionSaveRequested) {
        let Some(store) = &self.store else {
            tracing::warn!("session-actor has no store — dropping save request");
            return;
        };

        let persisted = PersistedSession {
            session_id: evt.session_id.clone(),
            title: evt.title.clone(),
            updated_at: Timestamp::now(),
            history: evt.history.clone(),
            active_strategy: evt.active_strategy.clone(),
            blobs: evt.blobs.clone(),
        };

        if let Err(e) = store.save(&persisted) {
            tracing::warn!(
                session_id = ?evt.session_id,
                err = ?e,
                "failed to persist session"
            );
        }
    }

    /// Loads a full session from disk and sends back a `SessionLoadCompleted` command.
    fn on_load_requested(&mut self, evt: &SessionLoadRequested, ctx: &ActorContext) {
        use nullslop_protocol::session::SessionLoadCompleted as CompletedPayload;

        let Some(store) = &self.store else {
            tracing::warn!("session-actor has no store — dropping load request");
            return;
        };

        match store.load_full(evt.byte_offset) {
            Ok(Some(persisted)) => {
                let _ = ctx.send_command(Command::SessionLoadCompleted {
                    payload: CompletedPayload {
                        session_id: persisted.session_id,
                        title: persisted.title,
                        history: persisted.history,
                        active_strategy: persisted.active_strategy,
                        blobs: persisted.blobs,
                    },
                });
            }
            Ok(None) => {
                tracing::warn!(
                    byte_offset = evt.byte_offset,
                    "session load returned None at offset"
                );
                let _ = ctx.send_command(Command::SessionLoadCompleted {
                    payload: CompletedPayload {
                        session_id: evt.session_id.clone(),
                        title: String::new(),
                        history: vec![],
                        active_strategy: PromptStrategyId::passthrough(),
                        blobs: std::collections::HashMap::new(),
                    },
                });
            }
            Err(e) => {
                tracing::warn!(err = ?e, "failed to load session");
                let _ = ctx.send_command(Command::SessionLoadCompleted {
                    payload: CompletedPayload {
                        session_id: evt.session_id.clone(),
                        title: String::new(),
                        history: vec![],
                        active_strategy: PromptStrategyId::passthrough(),
                        blobs: std::collections::HashMap::new(),
                    },
                });
            }
        }
    }

    // --- Command handlers ---

    /// EnqueueUserMessage: if idle → assemble prompt; if streaming → queue;
    /// otherwise → set input text.
    fn handle_enqueue_user_message(&self, payload: &EnqueueUserMessage, ctx: &ActorContext) {
        let action = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            if session.is_idle() {
                session.push_entry(ChatEntry::user(&payload.text));
                session.begin_sending();
                EnqueueAction::AssemblePrompt
            } else if session.is_streaming() {
                session.enqueue_message(payload.text.clone());
                EnqueueAction::Queued
            } else {
                EnqueueAction::SetInputText(payload.text.clone())
            }
        };

        let (history, model_name) = match action {
            EnqueueAction::AssemblePrompt => {
                let state = self.state.read();
                let history = state.session(&payload.session_id).history().to_vec();
                let model_name = state.provider.active_provider.clone();
                (history, model_name)
            }
            EnqueueAction::Queued => (vec![], String::new()),
            EnqueueAction::SetInputText(_) => (vec![], String::new()),
        };

        match action {
            EnqueueAction::AssemblePrompt => {
                if let Err(e) = ctx.send_command(Command::AssemblePrompt {
                    payload: AssemblePrompt {
                        session_id: payload.session_id.clone(),
                        history,
                        tools: vec![],
                        model_name,
                    },
                }) {
                    tracing::warn!(err = ?e, "session-actor failed to emit AssemblePrompt");
                }

                if let Err(e) = ctx.send_event(Event::ChatEntrySubmitted {
                    payload: ChatEntrySubmitted {
                        session_id: payload.session_id.clone(),
                        entry: ChatEntry::user(&payload.text),
                    },
                }) {
                    tracing::warn!(err = ?e, "session-actor failed to emit ChatEntrySubmitted");
                }
            }
            EnqueueAction::Queued => {}
            EnqueueAction::SetInputText(text) => {
                if let Err(e) = ctx.send_command(Command::SetChatInputText {
                    payload: SetChatInputText {
                        session_id: payload.session_id.clone(),
                        text,
                    },
                }) {
                    tracing::warn!(err = ?e, "session-actor failed to emit SetChatInputText");
                }
            }
        }
    }

    /// SetChatInputText: update the session's input buffer.
    fn handle_set_chat_input_text(&self, payload: &SetChatInputText) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&payload.session_id);
        session.chat_input_mut().replace_all(payload.text.clone());
    }

    /// PushChatEntry: push entry to session history, emit ChatEntrySubmitted event.
    fn handle_push_chat_entry(&self, payload: &PushChatEntry, ctx: &ActorContext) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.push_entry(payload.entry.clone());
        }

        if let Err(e) = ctx.send_event(Event::ChatEntrySubmitted {
            payload: ChatEntrySubmitted {
                session_id: payload.session_id.clone(),
                entry: payload.entry.clone(),
            },
        }) {
            tracing::warn!(err = ?e, "session-actor failed to emit ChatEntrySubmitted");
        }
    }

    /// PushToolResult: add tool result to session history.
    fn handle_push_tool_result(&self, payload: &PushToolResult) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&payload.session_id);
        session.push_entry(ChatEntry::tool_result(
            &payload.result.tool_call_id,
            &payload.result.name,
            &payload.result.content,
            payload.result.success,
        ));
    }

    /// SendMessage: backward compat — emit EnqueueUserMessage.
    fn handle_send_message(&self, payload: &SendMessage, ctx: &ActorContext) {
        if let Err(e) = ctx.send_command(Command::EnqueueUserMessage {
            payload: EnqueueUserMessage {
                session_id: payload.session_id.clone(),
                text: payload.text.clone(),
            },
        }) {
            tracing::warn!(err = ?e, "session-actor failed to emit EnqueueUserMessage");
        }
    }

    /// SessionLoadCompleted: restore session state and emit follow-up commands.
    fn handle_session_load_completed(
        &self,
        payload: &SessionLoadCompleted,
        ctx: &ActorContext,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.restore_history(payload.history.clone());
            state.session.active_session = payload.session_id.clone();
            state.session.session_loading = false;
        }

        if let Err(e) = ctx.send_command(Command::RestoreStrategyState {
            payload: RestoreStrategyState {
                session_id: payload.session_id.clone(),
                strategy_id: payload.active_strategy.clone(),
                blob: payload
                    .blobs
                    .get(&payload.active_strategy.to_string())
                    .cloned()
                    .unwrap_or(serde_json::json!({})),
            },
        }) {
            tracing::warn!(err = ?e, "session-actor failed to emit RestoreStrategyState");
        }

        if let Err(e) = ctx.send_command(Command::SwitchPromptStrategy {
            payload: SwitchPromptStrategy {
                session_id: payload.session_id.clone(),
                strategy_id: payload.active_strategy.clone(),
            },
        }) {
            tracing::warn!(err = ?e, "session-actor failed to emit SwitchPromptStrategy");
        }
    }

    // --- Event handlers ---

    /// PromptAssembled (event): transition session from assembling to streaming,
    /// emit SendToLlmProvider.
    fn handle_prompt_assembled(&self, payload: &PromptAssembled, ctx: &ActorContext) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            if session.is_assembling() {
                session.finish_assembling();
            }
            if session.is_sending() {
                session.finish_sending();
            }
            session.begin_streaming();
        }

        if let Err(e) = ctx.send_command(Command::SendToLlmProvider {
            payload: SendToLlmProvider {
                session_id: payload.session_id.clone(),
                messages: payload.messages.clone(),
                provider_id: None,
            },
        }) {
            tracing::warn!(err = ?e, "session-actor failed to emit SendToLlmProvider");
        }
    }

    /// Appends a streaming token to the session's assistant entry.
    fn on_stream_token(&self, event: &StreamToken) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        if !session.is_streaming() {
            session.begin_streaming();
        }
        session.append_stream_token(&event.token);
    }

    /// Marks the session's stream as finished.
    fn on_stream_completed(&self, event: &StreamCompleted) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        session.finish_streaming();
    }

    /// Begins tracking a streaming tool call.
    fn on_tool_use_started(&self, event: &ToolUseStarted) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        session.begin_tool_call(event.index, &event.id, &event.name);
    }

    /// Pushes a tool call entry into the session history.
    fn on_tool_call_received(&self, event: &ToolCallReceived) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        session.push_entry(ChatEntry::tool_call(
            &event.tool_call.id,
            &event.tool_call.name,
            &event.tool_call.arguments,
        ));
    }

    /// Appends a partial JSON delta to a streaming tool call.
    fn on_tool_call_streaming(&self, event: &ToolCallStreaming) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        session.append_tool_call_delta(event.index, &event.partial_json);
    }

    /// Pushes a tool result entry into the session history.
    fn on_tool_execution_completed(&self, event: &ToolExecutionCompleted) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        session.push_entry(ChatEntry::tool_result(
            &event.result.tool_call_id,
            &event.result.name,
            &event.result.content,
            event.result.success,
        ));
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
    use nullslop_actor::Actor;
    use nullslop_actor::{ActorContext, ActorEnvelope, MessageSink};
    use nullslop_actor::RecordingSink;
    use nullslop_component::{AppState, State};
    use nullslop_protocol::chat_input::{EnqueueUserMessage, PushChatEntry, SetChatInputText};
    // no context imports needed in tests currently
    use nullslop_protocol::provider::{
        SendMessage, StreamCompleted, StreamCompletedReason, StreamToken,
    };
    use nullslop_protocol::session::SessionLoadCompleted;
    use nullslop_protocol::tool::{
        PushToolResult, ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted,
    };
    use nullslop_protocol::{
        ChatEntry, ChatEntryKind, Command, Event, PromptStrategyId, SessionId,
        SessionSaveRequested, ToolCall, ToolResult,
    };
    use nullslop_session::{JsonlSessionStore, SessionStoreService};
    use nullslop_services::Services;
    use tempfile::TempDir;

    use super::SessionPersistenceActor;

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
    fn create_lifecycle_actor() -> (SessionPersistenceActor, State, Arc<RecordingSink>, ActorContext) {
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
                    "strategy_state".to_owned(),
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
        assert_eq!(full.blobs["strategy_state"]["compaction_count"], 5);
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
            payload: nullslop_protocol::ActorStarted {
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
        assert!(cmds.iter().any(|c| matches!(c, Command::AssemblePrompt { .. })));
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
        assert_eq!(prompt.history.len(), 1, "history must not be empty for first message");
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
        let found = events.iter().any(|e| matches!(e, Event::ChatEntrySubmitted { .. }));
        assert!(found, "expected ChatEntrySubmitted event");
    }

    // --- PushToolResult ---

    #[rstest::rstest]
    #[tokio::test]
    async fn push_tool_result_adds_tool_result_entry() {
        // Given a session actor with a session.
        let (mut actor, state, _sink, ctx) = create_lifecycle_actor();
        let session_id = SessionId::new();

        // When processing PushToolResult.
        actor
            .handle(
                ActorEnvelope::Command(Command::PushToolResult {
                    payload: PushToolResult {
                        session_id: session_id.clone(),
                        result: ToolResult {
                            tool_call_id: "call_1".into(),
                            name: "echo".into(),
                            content: "hello".into(),
                            success: true,
                        },
                    },
                }),
                &ctx,
            )
            .await;

        // Then a tool result entry was added to the session history.
        {
            let guard = state.read();
            let session = guard.session(&session_id);
            assert_eq!(session.history().len(), 1);
            assert!(matches!(
                session.history()[0].kind,
                ChatEntryKind::ToolResult { .. }
            ));
        }
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
        assert!(cmds.iter().any(|c| matches!(c, Command::RestoreStrategyState { .. })));
        assert!(cmds.iter().any(|c| matches!(c, Command::SwitchPromptStrategy { .. })));
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
                    payload: nullslop_protocol::context::PromptAssembled {
                        session_id: session_id.clone(),
                        system_prompt: None,
                        messages: vec![nullslop_protocol::LlmMessage::User {
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
        assert!(cmds.iter().any(|c| matches!(c, Command::SendToLlmProvider { .. })));
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
            ChatEntryKind::ToolCall { id, name, arguments } => {
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
