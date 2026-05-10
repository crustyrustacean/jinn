//! Coordinator actor — handles pinning and strategy management.
//!
//! This actor is being progressively gutted as handlers migrate to domain-specific
//! actors. After Phase 2, only pinning and strategy handlers remain.
//! Phase 3 will migrate these to the context actor and this crate will be deleted.
//!
//! # Lock discipline
//!
//! All handlers follow the same pattern: acquire state lock → mutate → release →
//! then emit. Never hold the lock during emission.

use nullslop_actor::{Actor, ActorContext, ActorEnvelope, SystemMessage};
use nullslop_component::State;
use nullslop_protocol::context::{
    PinChatEntry, PromptStrategySwitched, RestoreStrategyState, StrategyStateUpdated,
    SwitchPromptStrategy, UnpinChatEntry,
};
use nullslop_protocol::{Command, Event};
use nullslop_services::Services;

/// Direct message type (unused — the coordinator only responds to bus commands).
pub enum CoordinatorDirectMsg {}

/// The coordinator actor.
///
/// Subscribes to domain commands, mutates [`State`], and emits new commands
/// and events via the [`ActorContext`] message sink.
pub struct CoordinatorActor {
    /// Shared application state.
    state: State,
    /// Runtime services — unused after Phase 2 migration, removed in Phase 3.
    #[expect(dead_code, reason = "still injected by app.rs; removed in Phase 3")]
    services: Services,
}

impl Actor for CoordinatorActor {
    type Message = CoordinatorDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        // Subscribe to remaining commands.
        ctx.subscribe_command::<PinChatEntry>();
        ctx.subscribe_command::<UnpinChatEntry>();
        ctx.subscribe_command::<SwitchPromptStrategy>();
        ctx.subscribe_command::<RestoreStrategyState>();

        ctx.set_description("Pinning and strategy management (migrating to context actor)");

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
            ActorEnvelope::System(SystemMessage::ApplicationReady) => {
                ctx.announce_started();
            }
            ActorEnvelope::System(SystemMessage::ApplicationShuttingDown) => {
                ctx.announce_shutdown_completed();
            }
            ActorEnvelope::Event(_) | ActorEnvelope::Direct(_) | ActorEnvelope::Shutdown => {}
        }
    }

    async fn shutdown(self) {}
}

impl CoordinatorActor {
    /// Dispatches a command to the appropriate handler.
    fn handle_command(&mut self, cmd: &Command, ctx: &ActorContext) {
        match cmd {
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
            // Commands NOT subscribed to — these should not arrive.
            Command::EnqueueUserMessage { .. }
            | Command::SetChatInputText { .. }
            | Command::PushChatEntry { .. }
            | Command::ProviderSwitch { .. }
            | Command::LoadPickerEntries { .. }
            | Command::SessionLoadCompleted { .. }
            | Command::PushToolResult { .. }
            | Command::SendMessage { .. }
            | Command::AssemblePrompt { .. }
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
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nullslop_actor::{Actor, ActorContext, ActorEnvelope, MessageSink, RecordingSink};
    use nullslop_component::{AppState, State};
    use nullslop_protocol::context::{
        PinChatEntry, RestoreStrategyState, SwitchPromptStrategy, UnpinChatEntry,
    };
    use nullslop_protocol::{
        ChatEntry, Command, Event, PinPosition, PromptStrategyId, SessionId,
    };
    use nullslop_services::Services;

    use super::CoordinatorActor;

    /// Creates a test actor with a fresh AppState and fake services.
    fn create_actor() -> (CoordinatorActor, State, Arc<RecordingSink>, ActorContext) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("coordinator", sink.clone() as Arc<dyn MessageSink>);
        let state = State::new(AppState::default());
        ctx.set_data(state.clone());
        ctx.set_data(Services::new());
        let actor = CoordinatorActor::activate(&mut ctx);
        (actor, state, sink, ctx)
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
}
