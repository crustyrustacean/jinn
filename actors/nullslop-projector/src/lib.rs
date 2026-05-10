//! Projector actor — pure event→state writer.
//!
//! Subscribes to domain events, writes [`AppState`] via shared [`State`],
//! and never emits commands or events. Zero side effects beyond state mutation.
//!
//! Each event handler acquires a write lock on [`State`], mutates the
//! corresponding field in [`AppState`], and releases immediately.
//! The projector is the single actor responsible for keeping `AppState`
//! in sync with the event stream.

use nullslop_actor::{Actor, ActorContext, ActorEnvelope, SystemMessage};
use nullslop_component::prompt_template::PromptTemplateStore;
use nullslop_component::State;
use nullslop_protocol::Event;
use nullslop_protocol::provider::{ModelsRefreshed, PromptTemplatesLoaded, ProviderSwitched, StreamCompleted, StreamToken};
use nullslop_protocol::tool::{ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted, ToolUseStarted, ToolsRegistered};
use nullslop_protocol::context::{PromptStrategySwitched, StrategyStateUpdated};

/// Direct message type (unused — the projector only responds to bus events).
pub enum ProjectorDirectMsg {}

/// Pure event→state projector.
///
/// Subscribes to domain events and writes shared [`AppState`].
/// Never emits commands or events.
pub struct ProjectorActor {
    state: State,
}

impl Actor for ProjectorActor {
    type Message = ProjectorDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_event::<StreamToken>();
        ctx.subscribe_event::<StreamCompleted>();
        ctx.subscribe_event::<ToolCallReceived>();
        ctx.subscribe_event::<ToolUseStarted>();
        ctx.subscribe_event::<ToolCallStreaming>();
        ctx.subscribe_event::<ToolExecutionCompleted>();
        ctx.subscribe_event::<ToolsRegistered>();
        ctx.subscribe_event::<ProviderSwitched>();
        ctx.subscribe_event::<ModelsRefreshed>();
        ctx.subscribe_event::<PromptTemplatesLoaded>();
        ctx.subscribe_event::<PromptStrategySwitched>();
        ctx.subscribe_event::<StrategyStateUpdated>();

        let state = ctx
            .take_data::<State>()
            .expect("State must be injected");

        Self { state }
    }

    async fn handle(&mut self, msg: ActorEnvelope<ProjectorDirectMsg>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::System(SystemMessage::ApplicationReady) => {
                ctx.announce_started();
            }
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::Event(event) => {
                self.handle_event(&event);
            }
            ActorEnvelope::Command(_) | ActorEnvelope::Direct(_) | ActorEnvelope::Shutdown => {}
        }
    }

    async fn shutdown(self) {}
}

impl ProjectorActor {
    /// Dispatches an event to the appropriate handler.
    fn handle_event(&self, event: &Event) {
        match event {
            Event::StreamToken { payload } => self.on_stream_token(payload),
            Event::StreamCompleted { payload } => self.on_stream_completed(payload),
            Event::ToolUseStarted { payload } => self.on_tool_use_started(payload),
            Event::ToolCallReceived { payload } => self.on_tool_call_received(payload),
            Event::ToolCallStreaming { payload } => self.on_tool_call_streaming(payload),
            Event::ToolExecutionCompleted { payload } => {
                self.on_tool_execution_completed(payload);
            }
            Event::ToolsRegistered { .. } => {
                // Tools are tracked by the tool orchestrator and LLM actor.
                // No AppState field to update currently.
            }
            Event::ProviderSwitched { payload } => self.on_provider_switched(payload),
            Event::ModelsRefreshed { payload } => self.on_models_refreshed(payload),
            Event::PromptTemplatesLoaded { payload } => self.on_prompt_templates_loaded(payload),
            Event::PromptStrategySwitched { payload } => {
                self.on_prompt_strategy_switched(payload);
            }
            Event::StrategyStateUpdated { payload } => self.on_strategy_state_updated(payload),
            _ => {}
        }
    }

    /// Appends a streaming token to the session's assistant entry.
    ///
    /// Begins streaming if the session is not already in a streaming state.
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
        session.push_entry(nullslop_protocol::ChatEntry::tool_call(
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
        session.push_entry(nullslop_protocol::ChatEntry::tool_result(
            &event.result.tool_call_id,
            &event.result.name,
            &event.result.content,
            event.result.success,
        ));
    }

    /// Updates the active provider name.
    fn on_provider_switched(&self, event: &ProviderSwitched) {
        let mut state = self.state.write();
        state.active_provider = event.provider_name.clone();
    }

    /// Refreshes the model cache and updates the last-refreshed timestamp.
    fn on_models_refreshed(&self, event: &ModelsRefreshed) {
        let now = jiff::Timestamp::now();
        let mut state = self.state.write();
        state.model_cache = Some(nullslop_providers::ModelCache {
            entries: event.results.clone(),
            last_updated_at: Some(now),
        });
        state.last_refreshed_at = Some(now);
    }

    /// Replaces the prompt template store with the loaded templates.
    fn on_prompt_templates_loaded(&self, event: &PromptTemplatesLoaded) {
        let mut state = self.state.write();
        state.prompt_templates =
            PromptTemplateStore::from_vec(event.templates.clone());
    }

    /// Switches the session's active prompt strategy.
    fn on_prompt_strategy_switched(&self, event: &PromptStrategySwitched) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        session.switch_strategy(event.strategy_id.clone());
    }

    /// Persists a strategy state blob in the session and the global strategy_state map.
    fn on_strategy_state_updated(&self, event: &StrategyStateUpdated) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&event.session_id);
        session.set_strategy_state(event.blob.clone());
        state.strategy_state.insert(
            (event.session_id.clone(), event.strategy_id.clone()),
            event.blob.clone(),
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::ProjectorActor;
    use nullslop_actor::{Actor, ActorContext, ActorEnvelope, MessageSink, SystemMessage};
    use nullslop_component::{AppState, State};
    use nullslop_protocol::{
        ChatEntryKind, Event, SessionId,
        ToolCall, ToolResult,
        provider::{StreamCompleted, StreamCompletedReason, StreamToken},
        tool::{ToolCallReceived, ToolCallStreaming, ToolExecutionCompleted},
        context::{PromptStrategySwitched, StrategyStateUpdated},
    };
    use std::sync::Arc;

    /// A no-op message sink for testing.
    #[derive(Debug)]
    struct NullSink;

    impl MessageSink for NullSink {
        fn send_command(
            &self,
            _command: nullslop_protocol::Command,
        ) -> nullslop_actor::SendResult {
            Ok(())
        }

        fn send_event(&self, _event: Event) -> nullslop_actor::SendResult {
            Ok(())
        }
    }

    fn make_actor() -> (State, ProjectorActor, ActorContext) {
        let state = State::new(AppState::default());
        let sink = Arc::new(NullSink) as Arc<dyn MessageSink>;
        let mut ctx = ActorContext::new("projector", sink);
        ctx.set_data(state.clone());
        let actor = ProjectorActor::activate(&mut ctx);
        (state, actor, ctx)
    }

    // --- StreamToken ---

    #[rstest::rstest]
    #[tokio::test]
    async fn stream_token_creates_assistant_entry() {
        // Given a ProjectorActor with default state.
        let (state, mut actor, ctx) = make_actor();
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
        // Given a ProjectorActor with one token already processed.
        let (state, mut actor, ctx) = make_actor();
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
        // Given a ProjectorActor with a streaming session.
        let (state, mut actor, ctx) = make_actor();
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
        // Given a ProjectorActor.
        let (state, mut actor, ctx) = make_actor();
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
        // Given a ProjectorActor with a tool call started (via begin_tool_call).
        let (state, mut actor, ctx) = make_actor();
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
        // Given a ProjectorActor.
        let (state, mut actor, ctx) = make_actor();
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
                assert!(*success);
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    // --- ProviderSwitched ---

    #[rstest::rstest]
    #[tokio::test]
    async fn provider_switched_updates_active_provider() {
        // Given a ProjectorActor.
        let (state, mut actor, ctx) = make_actor();

        // When processing a ProviderSwitched event.
        let event = Event::ProviderSwitched {
            payload: nullslop_protocol::provider::ProviderSwitched {
                provider_name: "Ollama".to_owned(),
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then the active provider is updated.
        let guard = state.read();
        assert_eq!(guard.active_provider, "Ollama");
    }

    // --- ModelsRefreshed ---

    #[rstest::rstest]
    #[tokio::test]
    async fn models_refreshed_updates_model_cache() {
        // Given a ProjectorActor.
        let (state, mut actor, ctx) = make_actor();

        let mut results = std::collections::HashMap::new();
        results.insert("Ollama".to_owned(), vec!["llama3".to_owned(), "mistral".to_owned()]);

        // When processing a ModelsRefreshed event.
        let event = Event::ModelsRefreshed {
            payload: nullslop_protocol::provider::ModelsRefreshed {
                results: results.clone(),
                errors: std::collections::HashMap::new(),
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then the model cache and last_refreshed_at are updated.
        let guard = state.read();
        let cache = guard.model_cache.as_ref().expect("model cache should be set");
        assert_eq!(cache.entries.get("Ollama").map(|v| v.len()), Some(2));
        assert!(guard.last_refreshed_at.is_some());
    }

    // --- PromptTemplatesLoaded ---

    #[rstest::rstest]
    #[tokio::test]
    async fn prompt_templates_loaded_updates_template_store() {
        // Given a ProjectorActor.
        let (state, mut actor, ctx) = make_actor();

        let templates = vec![nullslop_protocol::PromptTemplate {
            name: "greeting".to_owned(),
            description: "A greeting".to_owned(),
            body: "Hello!".to_owned(),
        }];

        // When processing a PromptTemplatesLoaded event.
        let event = Event::PromptTemplatesLoaded {
            payload: nullslop_protocol::provider::PromptTemplatesLoaded {
                templates: templates.clone(),
                error: None,
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then the prompt template store contains the templates.
        let guard = state.read();
        assert_eq!(guard.prompt_templates.len(), 1);
        assert_eq!(
            guard.prompt_templates.find_by_name("greeting").map(|t| &t.body),
            Some(&"Hello!".to_owned())
        );
    }

    // --- PromptStrategySwitched ---

    #[rstest::rstest]
    #[tokio::test]
    async fn prompt_strategy_switched_updates_session_strategy() {
        // Given a ProjectorActor.
        let (state, mut actor, ctx) = make_actor();
        let session_id = SessionId::new();

        // When processing a PromptStrategySwitched event.
        let event = Event::PromptStrategySwitched {
            payload: PromptStrategySwitched {
                session_id: session_id.clone(),
                strategy_id: nullslop_protocol::PromptStrategyId::sliding_window(),
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then the session's active strategy is updated.
        let guard = state.read();
        let session = guard.session(&session_id);
        assert_eq!(
            session.active_strategy(),
            &nullslop_protocol::PromptStrategyId::sliding_window()
        );
    }

    // --- StrategyStateUpdated ---

    #[rstest::rstest]
    #[tokio::test]
    async fn strategy_state_updated_stores_blob_in_session() {
        // Given a ProjectorActor.
        let (state, mut actor, ctx) = make_actor();
        let session_id = SessionId::new();

        // When processing a StrategyStateUpdated event.
        let blob = serde_json::json!({"compaction_count": 3});
        let event = Event::StrategyStateUpdated {
            payload: StrategyStateUpdated {
                session_id: session_id.clone(),
                strategy_id: nullslop_protocol::PromptStrategyId::compaction(),
                blob: blob.clone(),
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then the session has the blob.
        let guard = state.read();
        let session = guard.session(&session_id);
        let state_blob = session.strategy_state().expect("strategy state should be set");
        assert_eq!(state_blob["compaction_count"], 3);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn strategy_state_updated_stores_blob_in_global_map() {
        // Given a ProjectorActor.
        let (state, mut actor, ctx) = make_actor();
        let session_id = SessionId::new();
        let strategy_id = nullslop_protocol::PromptStrategyId::compaction();

        // When processing a StrategyStateUpdated event.
        let blob = serde_json::json!({"compaction_count": 3});
        let event = Event::StrategyStateUpdated {
            payload: StrategyStateUpdated {
                session_id: session_id.clone(),
                strategy_id: strategy_id.clone(),
                blob: blob.clone(),
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then the global strategy_state map has the blob keyed by (session_id, strategy_id).
        let guard = state.read();
        let stored = guard
            .strategy_state
            .get(&(session_id.clone(), strategy_id.clone()))
            .expect("blob should be in strategy_state map");
        assert_eq!(stored["compaction_count"], 3);
    }

    // --- Lifecycle ---

    #[rstest::rstest]
    #[tokio::test]
    async fn application_ready_announces_started() {
        // Given a ProjectorActor.
        let (_state, mut actor, ctx) = make_actor();

        // When processing ApplicationReady.
        // Then it completes without panic (announce_started is fire-and-forget).
        actor
            .handle(
                ActorEnvelope::System(SystemMessage::ApplicationReady),
                &ctx,
            )
            .await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn application_shutting_down_announces_shutdown_completed() {
        // Given a ProjectorActor.
        let (_state, mut actor, ctx) = make_actor();

        // When processing ApplicationShuttingDown.
        // Then it completes without panic (announce_shutdown_completed is fire-and-forget).
        actor
            .handle(
                ActorEnvelope::System(SystemMessage::ApplicationShuttingDown),
                &ctx,
            )
            .await;
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn unhandled_events_are_no_ops() {
        // Given a ProjectorActor.
        let (state, mut actor, ctx) = make_actor();

        // When processing an unhandled event (e.g., ActorStarting).
        let event = Event::ActorStarting {
            payload: nullslop_protocol::ActorStarting {
                name: "other-actor".to_owned(),
                description: None,
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then state is unchanged.
        let guard = state.read();
        assert_eq!(guard.active_provider, nullslop_component::NO_PROVIDER_ID);
    }
}
