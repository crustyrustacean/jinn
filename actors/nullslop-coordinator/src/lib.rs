//! Coordinator actor — orchestrates domain workflows by subscribing to commands,
//! mutating shared state, and emitting new commands and events.
//!
//! The coordinator is the central orchestrator in the actor system. It handles
//! commands that require state mutation followed by side effects (emitting new
//! commands or events). Pure forwarding (where another actor handles a command
//! directly) is left to the actor host's routing.
//!
//! # Lock discipline
//!
//! All handlers follow the same pattern: acquire state lock → mutate → release →
//! then emit. Never hold the lock during emission.

use nullslop_actor::{Actor, ActorContext, ActorEnvelope, SystemMessage};
use nullslop_component::provider_picker::loader::load_provider_picker_items;
use nullslop_component::State;
use nullslop_protocol::chat_input::{EnqueueUserMessage, PushChatEntry, SetChatInputText};
use nullslop_protocol::context::{
    AssemblePrompt, PinChatEntry, PromptStrategySwitched, RestoreStrategyState,
    StrategyStateUpdated, SwitchPromptStrategy, UnpinChatEntry,
};
use nullslop_protocol::provider::{
    ProviderSwitch, ProviderSwitched, SendMessage, SendToLlmProvider,
};
use nullslop_protocol::session::SessionLoadCompleted;
use nullslop_protocol::tool::PushToolResult;
use nullslop_protocol::system::LoadPickerEntries;
use nullslop_protocol::{
    ChatEntry, Command, Event, PickerKind, PromptStrategyId,
};
use nullslop_services::Services;

/// Direct message type (unused — the coordinator only responds to bus commands).
pub enum CoordinatorDirectMsg {}

/// Decision returned after inspecting session state in `EnqueueUserMessage`.
enum EnqueueAction {
    /// Session is idle — dispatch prompt assembly.
    AssemblePrompt,
    /// Session is streaming — message was queued.
    Queued,
    /// Session is busy (sending or assembling) — put text back in the input box.
    SetInputText(String),
}

/// The coordinator actor.
///
/// Subscribes to domain commands, mutates [`State`], and emits new commands
/// and events via the [`ActorContext`] message sink.
pub struct CoordinatorActor {
    /// Shared application state.
    state: State,
    /// Runtime services (provider registry, session store, etc.).
    services: Services,
}

impl Actor for CoordinatorActor {
    type Message = CoordinatorDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        // Subscribe to commands the coordinator orchestrates.
        ctx.subscribe_command::<EnqueueUserMessage>();
        ctx.subscribe_command::<SetChatInputText>();
        ctx.subscribe_command::<PushChatEntry>();
        ctx.subscribe_command::<ProviderSwitch>();
        ctx.subscribe_command::<PinChatEntry>();
        ctx.subscribe_command::<UnpinChatEntry>();
        ctx.subscribe_command::<SwitchPromptStrategy>();
        ctx.subscribe_command::<RestoreStrategyState>();
        ctx.subscribe_command::<LoadPickerEntries>();
        ctx.subscribe_command::<SessionLoadCompleted>();
        ctx.subscribe_command::<PushToolResult>();
        ctx.subscribe_command::<SendMessage>();

        // Subscribe to events that trigger follow-up commands.
        ctx.subscribe_event::<nullslop_protocol::context::PromptAssembled>();
        ctx.subscribe_event::<nullslop_protocol::provider::ModelsRefreshed>();

        ctx.set_description("Orchestrates domain workflows: state mutation + side effects");

        // Extract injected State and Services.
        let state = ctx
            .take_data::<State>()
            .expect("CoordinatorActor requires State injection");
        let services = ctx
            .take_data::<Services>()
            .expect("CoordinatorActor requires Services injection");

        Self { state, services }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(cmd) => self.handle_command(&cmd, ctx),
            ActorEnvelope::Event(event) => {
                match event {
                    Event::PromptAssembled { ref payload } => {
                        self.handle_prompt_assembled(payload, ctx);
                    }
                    Event::ModelsRefreshed { .. } => {
                        self.handle_models_refreshed();
                    }
                    _ => {}
                }
            }
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

impl CoordinatorActor {
    /// Dispatches a command to the appropriate handler.
    fn handle_command(&mut self, cmd: &Command, ctx: &ActorContext) {
        match cmd {
            Command::EnqueueUserMessage { payload } => {
                self.handle_enqueue_user_message(payload, ctx);
            }
            Command::SetChatInputText { payload } => {
                self.handle_set_chat_input_text(payload);
            }
            Command::PushChatEntry { payload } => {
                self.handle_push_chat_entry(payload, ctx);
            }
            Command::ProviderSwitch { payload } => {
                self.handle_provider_switch(payload, ctx);
            }
            Command::PinChatEntry { payload } => {
                self.handle_pin_chat_entry(payload);
            }
            Command::UnpinChatEntry { payload } => {
                self.handle_unpin_chat_entry(payload);
            }
            Command::SwitchPromptStrategy { payload } => {
                self.handle_switch_prompt_strategy(payload, ctx);
            }
            Command::RestoreStrategyState { payload } => {
                self.handle_restore_strategy_state(payload, ctx);
            }
            Command::LoadPickerEntries { payload } => {
                self.handle_load_picker_entries(payload);
            }
            Command::SessionLoadCompleted { payload } => {
                self.handle_session_load_completed(payload, ctx);
            }
            Command::PushToolResult { payload } => {
                self.handle_push_tool_result(payload);
            }
            Command::SendMessage { payload } => {
                self.handle_send_message(payload, ctx);
            }
            // Commands NOT subscribed to — these should not arrive.
            Command::AssemblePrompt { .. }
            | Command::SendToLlmProvider { .. }
            | Command::ExecuteTool { .. }
            | Command::ProceedWithShutdown { .. }
            | Command::SessionLoadRequested { .. }
            | Command::CancelStream { .. }
            | Command::RefreshModels
            | Command::RescanPromptTemplates
            | Command::ExecuteToolBatch { .. }
            | Command::RegisterTools { .. } => {}
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
                session.begin_sending();
                EnqueueAction::AssemblePrompt
            } else if session.is_streaming() {
                session.enqueue_message(payload.text.clone());
                EnqueueAction::Queued
            } else {
                EnqueueAction::SetInputText(payload.text.clone())
            }
        };

        match action {
            EnqueueAction::AssemblePrompt => {
                if let Err(e) = ctx.send_command(Command::AssemblePrompt {
                    payload: AssemblePrompt {
                        session_id: payload.session_id.clone(),
                        history: vec![],
                        tools: vec![],
                        model_name: String::new(),
                    },
                }) {
                    tracing::warn!(err = ?e, "coordinator failed to emit AssemblePrompt");
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
                    tracing::warn!(err = ?e, "coordinator failed to emit SetChatInputText");
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
            payload: nullslop_protocol::chat_input::ChatEntrySubmitted {
                session_id: payload.session_id.clone(),
                entry: payload.entry.clone(),
            },
        }) {
            tracing::warn!(err = ?e, "coordinator failed to emit ChatEntrySubmitted");
        }
    }

    /// ProviderSwitch: update active provider, emit ProviderSwitched event.
    fn handle_provider_switch(&self, payload: &ProviderSwitch, ctx: &ActorContext) {
        {
            let mut state = self.state.write();
            state.active_provider = payload.provider_id.clone();
        }

        if let Err(e) = ctx.send_event(Event::ProviderSwitched {
            payload: ProviderSwitched {
                provider_name: payload.provider_id.clone(),
            },
        }) {
            tracing::warn!(err = ?e, "coordinator failed to emit ProviderSwitched");
        }
    }

    /// PinChatEntry: pin entry in session.
    fn handle_pin_chat_entry(&self, payload: &PinChatEntry) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&payload.session_id);
        session.pin_entry(&payload.entry_id, payload.position);
    }

    /// UnpinChatEntry: unpin entry in session.
    fn handle_unpin_chat_entry(&self, payload: &UnpinChatEntry) {
        let mut state = self.state.write();
        let session = state.session_mut_or_create(&payload.session_id);
        session.unpin_entry(&payload.entry_id);
    }

    /// SwitchPromptStrategy: switch strategy, emit RestoreStrategyState + PromptStrategySwitched.
    fn handle_switch_prompt_strategy(
        &self,
        payload: &SwitchPromptStrategy,
        ctx: &ActorContext,
    ) {
        let blob = {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.switch_strategy(payload.strategy_id.clone());
            state
                .strategy_state
                .get(&(payload.session_id.clone(), payload.strategy_id.clone()))
                .cloned()
                .unwrap_or(serde_json::json!({}))
        };

        if let Err(e) = ctx.send_command(Command::RestoreStrategyState {
            payload: RestoreStrategyState {
                session_id: payload.session_id.clone(),
                strategy_id: payload.strategy_id.clone(),
                blob,
            },
        }) {
            tracing::warn!(err = ?e, "coordinator failed to emit RestoreStrategyState");
        }

        if let Err(e) = ctx.send_event(Event::PromptStrategySwitched {
            payload: PromptStrategySwitched {
                session_id: payload.session_id.clone(),
                strategy_id: payload.strategy_id.clone(),
            },
        }) {
            tracing::warn!(
                err = ?e,
                "coordinator failed to emit PromptStrategySwitched"
            );
        }
    }

    /// RestoreStrategyState: set strategy blob on session, emit StrategyStateUpdated.
    fn handle_restore_strategy_state(
        &self,
        payload: &RestoreStrategyState,
        ctx: &ActorContext,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            session.set_strategy_state(payload.blob.clone());
            state.strategy_state.insert(
                (payload.session_id.clone(), payload.strategy_id.clone()),
                payload.blob.clone(),
            );
        }

        if let Err(e) = ctx.send_event(Event::StrategyStateUpdated {
            payload: StrategyStateUpdated {
                session_id: payload.session_id.clone(),
                strategy_id: payload.strategy_id.clone(),
                blob: payload.blob.clone(),
            },
        }) {
            tracing::warn!(err = ?e, "coordinator failed to emit StrategyStateUpdated");
        }
    }

    /// LoadPickerEntries: load entries based on picker kind.
    fn handle_load_picker_entries(&self, payload: &LoadPickerEntries) {
        match payload.kind {
            PickerKind::Provider => {
                let mut state = self.state.write();
                load_provider_picker_items(&self.services, &mut state);
            }
            PickerKind::ContextAssembly
            | PickerKind::Session
            | PickerKind::Keymap => {
                // Future: load from services or state as appropriate.
                // For now, a no-op — entries will be populated when needed.
            }
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
            session.switch_strategy(payload.active_strategy.clone());
            state.active_session = payload.session_id.clone();
            state.session_loading = false;
            // Store blobs from the payload into strategy_state.
            // Blobs are keyed by string (strategy ID as raw string).
            for (key, blob) in &payload.blobs {
                let strat_id = PromptStrategyId::new(key.as_str());
                state
                    .strategy_state
                    .insert((payload.session_id.clone(), strat_id), blob.clone());
            }
        }

        // Emit RestoreStrategyState for the active strategy.
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
            tracing::warn!(
                err = ?e,
                "coordinator failed to emit RestoreStrategyState"
            );
        }

        // Emit SwitchPromptStrategy for the active strategy.
        if let Err(e) = ctx.send_command(Command::SwitchPromptStrategy {
            payload: SwitchPromptStrategy {
                session_id: payload.session_id.clone(),
                strategy_id: payload.active_strategy.clone(),
            },
        }) {
            tracing::warn!(
                err = ?e,
                "coordinator failed to emit SwitchPromptStrategy"
            );
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
            tracing::warn!(err = ?e, "coordinator failed to emit EnqueueUserMessage");
        }
    }

    /// PromptAssembled (event): transition session from assembling to streaming,
    /// emit SendToLlmProvider.
    fn handle_prompt_assembled(
        &self,
        payload: &nullslop_protocol::context::PromptAssembled,
        ctx: &ActorContext,
    ) {
        {
            let mut state = self.state.write();
            let session = state.session_mut_or_create(&payload.session_id);
            // Session may be in assembling or sending state.
            if session.is_assembling() {
                session.finish_assembling();
            }
            // finish_sending is needed if in sending state (no tokens yet).
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
            tracing::warn!(
                err = ?e,
                "coordinator failed to emit SendToLlmProvider"
            );
        }
    }
    /// ModelsRefreshed: reload provider picker entries from updated model cache.
    fn handle_models_refreshed(&self) {
        let mut state = self.state.write();
        load_provider_picker_items(&self.services, &mut state);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nullslop_actor::{Actor, ActorContext, ActorEnvelope, MessageSink, RecordingSink};
    use nullslop_component::{AppState, State};
    use nullslop_protocol::chat_input::{EnqueueUserMessage, PushChatEntry, SetChatInputText};
    use nullslop_protocol::context::{
        PinChatEntry, RestoreStrategyState, SwitchPromptStrategy, UnpinChatEntry,
    };
    use nullslop_protocol::provider::{ProviderSwitch, SendMessage};
    use nullslop_protocol::session::SessionLoadCompleted;
    use nullslop_protocol::tool::PushToolResult;
    use nullslop_protocol::system::LoadPickerEntries;
    use nullslop_protocol::{
        ChatEntry, ChatEntryKind, Command, Event, PickerKind, PinPosition, PromptStrategyId,
        SessionId, ToolResult,
    };
    use nullslop_services::Services;

    use super::CoordinatorActor;

    /// Creates a test actor with a fresh AppState and fake services.
    fn create_actor() -> (CoordinatorActor, State, Arc<RecordingSink>, ActorContext) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("coordinator", sink.clone() as Arc<dyn MessageSink>);
        let state = State::new(AppState::default());
        let services = Services::new();
        ctx.set_data(state.clone());
        ctx.set_data(services);
        let actor = CoordinatorActor::activate(&mut ctx);
        (actor, state, sink, ctx)
    }

    // --- EnqueueUserMessage ---

    #[rstest::rstest]
    #[tokio::test]
    async fn enqueue_user_message_dispatches_assemble_prompt_when_idle() {
        // Given a coordinator with an idle session.
        let (mut actor, state, sink, ctx) = create_actor();
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
    async fn enqueue_user_message_queues_when_streaming() {
        // Given a coordinator with a streaming session.
        let (mut actor, state, _sink, ctx) = create_actor();
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
        // Given a coordinator with a sending (but not streaming) session.
        let (mut actor, state, sink, ctx) = create_actor();
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
        // Given a coordinator with a session.
        let (mut actor, state, _sink, ctx) = create_actor();
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
        // Given a coordinator with a session.
        let (mut actor, state, sink, ctx) = create_actor();
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

    // --- ProviderSwitch ---

    #[rstest::rstest]
    #[tokio::test]
    async fn provider_switch_updates_active_provider() {
        // Given a coordinator.
        let (mut actor, state, sink, ctx) = create_actor();

        // When processing ProviderSwitch.
        actor
            .handle(
                ActorEnvelope::Command(Command::ProviderSwitch {
                    payload: ProviderSwitch {
                        provider_id: "ollama".into(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then the active provider is updated.
        {
            let guard = state.read();
            assert_eq!(guard.active_provider, "ollama");
        }

        // And a ProviderSwitched event was emitted.
        let events = sink.events();
        let found = events.iter().any(|e| matches!(e, Event::ProviderSwitched { .. }));
        assert!(found, "expected ProviderSwitched event");
    }

    // --- PinChatEntry ---

    #[rstest::rstest]
    #[tokio::test]
    async fn pin_chat_entry_sets_pin_position() {
        // Given a coordinator with a session that has a chat entry.
        let (mut actor, state, _sink, ctx) = create_actor();
        let session_id = SessionId::new();
        let entry_id = {
            let mut guard = state.write();
            let session = guard.session_mut_or_create(&session_id);
            let index = session.push_entry(ChatEntry::user("pin me"));
            session.history()[index].id.clone()
        };

        // When processing PinChatEntry.
        actor
            .handle(
                ActorEnvelope::Command(Command::PinChatEntry {
                    payload: PinChatEntry {
                        session_id: session_id.clone(),
                        entry_id: entry_id.clone(),
                        position: PinPosition::Top,
                    },
                }),
                &ctx,
            )
            .await;

        // Then the entry is pinned.
        {
            let guard = state.read();
            let session = guard.session(&session_id);
            assert!(session.pinned_entries().iter().any(|e| e.id == entry_id));
        }
    }

    // --- UnpinChatEntry ---

    #[rstest::rstest]
    #[tokio::test]
    async fn unpin_chat_entry_removes_pin() {
        // Given a coordinator with a pinned entry.
        let (mut actor, state, _sink, ctx) = create_actor();
        let session_id = SessionId::new();
        let entry_id = {
            let mut guard = state.write();
            let session = guard.session_mut_or_create(&session_id);
            let entry = ChatEntry::user("pin me").with_pin(PinPosition::Top);
            let index = session.push_entry(entry);
            session.history()[index].id.clone()
        };

        // When processing UnpinChatEntry.
        actor
            .handle(
                ActorEnvelope::Command(Command::UnpinChatEntry {
                    payload: UnpinChatEntry {
                        session_id: session_id.clone(),
                        entry_id: entry_id.clone(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then the entry is no longer pinned.
        {
            let guard = state.read();
            let session = guard.session(&session_id);
            assert!(session.pinned_entries().is_empty());
        }
    }

    // --- SwitchPromptStrategy ---

    #[rstest::rstest]
    #[tokio::test]
    async fn switch_prompt_strategy_updates_session_strategy() {
        // Given a coordinator with a session.
        let (mut actor, state, sink, ctx) = create_actor();
        let session_id = SessionId::new();
        let new_strategy = PromptStrategyId::sliding_window();

        // When processing SwitchPromptStrategy.
        actor
            .handle(
                ActorEnvelope::Command(Command::SwitchPromptStrategy {
                    payload: SwitchPromptStrategy {
                        session_id: session_id.clone(),
                        strategy_id: new_strategy.clone(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then the session strategy is updated.
        {
            let guard = state.read();
            let session = guard.session(&session_id);
            assert_eq!(session.active_strategy(), &new_strategy);
        }

        // And RestoreStrategyState and PromptStrategySwitched were emitted.
        let cmds = sink.commands();
        assert!(cmds.iter().any(|c| matches!(c, Command::RestoreStrategyState { .. })));
        let events = sink.events();
        assert!(events.iter().any(|e| matches!(e, Event::PromptStrategySwitched { .. })));
    }

    // --- RestoreStrategyState ---

    #[rstest::rstest]
    #[tokio::test]
    async fn restore_strategy_state_stores_blob() {
        // Given a coordinator with a session.
        let (mut actor, state, sink, ctx) = create_actor();
        let session_id = SessionId::new();
        let strategy_id = PromptStrategyId::compaction();
        let blob = serde_json::json!({"compaction_count": 5});

        // When processing RestoreStrategyState.
        actor
            .handle(
                ActorEnvelope::Command(Command::RestoreStrategyState {
                    payload: RestoreStrategyState {
                        session_id: session_id.clone(),
                        strategy_id: strategy_id.clone(),
                        blob: blob.clone(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then the blob is stored in strategy_state.
        {
            let guard = state.read();
            let stored = guard
                .strategy_state
                .get(&(session_id.clone(), strategy_id.clone()));
            assert_eq!(stored, Some(&blob));
        }

        // And a StrategyStateUpdated event was emitted.
        let events = sink.events();
        let found = events.iter().any(|e| matches!(e, Event::StrategyStateUpdated { .. }));
        assert!(found, "expected StrategyStateUpdated event");
    }

    // --- LoadPickerEntries (Provider) ---

    #[rstest::rstest]
    #[tokio::test]
    async fn load_picker_entries_provider_does_not_panic() {
        // Given a coordinator with fake services (empty registry).
        let (mut actor, _state, _sink, ctx) = create_actor();

        // When processing LoadPickerEntries for Provider kind.
        actor
            .handle(
                ActorEnvelope::Command(Command::LoadPickerEntries {
                    payload: LoadPickerEntries {
                        kind: PickerKind::Provider,
                    },
                }),
                &ctx,
            )
            .await;

        // Then the handler completes without panic.
        // (Provider entries loaded from fake services — no models available.)
    }

    // --- ModelsRefreshed (event) ---

    #[rstest::rstest]
    #[tokio::test]
    async fn models_refreshed_reloads_provider_picker_entries() {
        // Given a coordinator with fake services (empty registry).
        let (mut actor, _state, _sink, ctx) = create_actor();

        // When processing ModelsRefreshed event.
        actor
            .handle(
                ActorEnvelope::Event(Event::ModelsRefreshed {
                    payload: nullslop_protocol::provider::ModelsRefreshed {
                        results: std::collections::HashMap::new(),
                        errors: std::collections::HashMap::new(),
                    },
                }),
                &ctx,
            )
            .await;

        // Then the handler completes without panic (picker entries reloaded).
    }

    // --- SessionLoadCompleted ---

    #[rstest::rstest]
    #[tokio::test]
    async fn session_load_completed_restores_history() {
        // Given a coordinator.
        let (mut actor, state, sink, ctx) = create_actor();
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
                        blobs: std::collections::HashMap::new(),
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
            assert_eq!(guard.active_session, session_id);
        }

        // And session_loading is cleared.
        {
            let guard = state.read();
            assert!(!guard.session_loading);
        }

        // And RestoreStrategyState and SwitchPromptStrategy were emitted.
        let cmds = sink.commands();
        assert!(cmds.iter().any(|c| matches!(c, Command::RestoreStrategyState { .. })));
        assert!(cmds.iter().any(|c| matches!(c, Command::SwitchPromptStrategy { .. })));
    }

    // --- PushToolResult ---

    #[rstest::rstest]
    #[tokio::test]
    async fn push_tool_result_adds_tool_result_entry() {
        // Given a coordinator with a session.
        let (mut actor, state, _sink, ctx) = create_actor();
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
        // Given a coordinator.
        let (mut actor, _state, sink, ctx) = create_actor();
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

    // --- PromptAssembled (event) ---

    #[rstest::rstest]
    #[tokio::test]
    async fn prompt_assembled_transitions_to_streaming() {
        // Given a coordinator with an assembling session.
        let (mut actor, state, sink, ctx) = create_actor();
        let session_id = SessionId::new();

        {
            let mut guard = state.write();
            let session = guard.session_mut_or_create(&session_id);
            session.begin_sending();
        }

        // Simulate the flow: context actor emits PromptAssembled event.
        // Session is in 'sending' state at this point (not assembling).
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
}
