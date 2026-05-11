//! Context actor — prompt assembly, strategy management, pinning, and templates.
//!
//! Owns the full context/prompt domain: assembles LLM-ready prompts from chat
//! history, manages prompt strategies, handles entry pinning, and loads prompt
//! templates. Subscribes to [`AssemblePrompt`], [`SwitchPromptStrategy`],
//! [`RestoreStrategyState`], [`PinChatEntry`], [`UnpinChatEntry`] commands and
//! [`PromptStrategySwitched`], [`ToolsRegistered`], [`PromptTemplatesLoaded`] events.
//!
//! Unknown sessions are automatically initialized with `PassthroughStrategy`.
//! Strategy switching uses a [`StrategyFactory`] injected via [`ActorContext`] data.

mod handlers;

use std::collections::HashMap;

use nsslice_context_protocol::{DefaultStrategyFactory, StrategyFactory};
use nullslop_actor::{Actor, ActorContext, ActorEnvelope, SystemMessage};
use nullslop_component::State;
use nullslop_protocol::context::{
    AssemblePrompt, PinChatEntry, PromptStrategySwitched, RestoreStrategyState,
    SwitchPromptStrategy, UnpinChatEntry,
};
use nullslop_protocol::provider::PromptTemplatesLoaded;
use nullslop_protocol::tool::ToolsRegistered;
use nullslop_protocol::{Command, Event, SessionId, ToolDefinition};

/// Direct message type for the prompt assembly actor (unused for now).
pub enum ContextDirectMsg {}

/// The context actor — handles prompt assembly, strategy management, pinning, and templates.
pub struct PromptAssemblyActor {
    /// Shared application state.
    pub(super) state: State,
    /// Per-session prompt assembly strategies.
    pub(super) strategies: HashMap<SessionId, Box<dyn nsslice_context_protocol::PromptAssembly>>,
    /// Cached tool definitions from [`ToolsRegistered`] events.
    pub(super) tool_definitions: HashMap<String, ToolDefinition>,
    /// Factory for creating new strategies on switch.
    pub(super) factory: Option<Box<dyn StrategyFactory>>,
}

impl Actor for PromptAssemblyActor {
    type Message = ContextDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        // Existing subscriptions (prompt assembly).
        ctx.subscribe_command::<AssemblePrompt>();
        ctx.subscribe_event::<PromptStrategySwitched>();
        ctx.subscribe_event::<ToolsRegistered>();

        // New subscriptions (strategy management, pinning, templates).
        ctx.subscribe_command::<SwitchPromptStrategy>();
        ctx.subscribe_command::<RestoreStrategyState>();
        ctx.subscribe_command::<PinChatEntry>();
        ctx.subscribe_command::<UnpinChatEntry>();
        ctx.subscribe_event::<PromptTemplatesLoaded>();

        ctx.set_description("Context assembly, strategy management, pinning, and templates");

        #[expect(clippy::expect_used, reason = "State is always injected at startup")]
        let state = ctx
            .take_data::<State>()
            .expect("PromptAssemblyActor requires State injection");
        let factory = ctx
            .take_data::<Box<dyn StrategyFactory>>()
            .unwrap_or_else(|| Box::new(DefaultStrategyFactory));
        Self {
            state,
            strategies: HashMap::new(),
            tool_definitions: HashMap::new(),
            factory: Some(factory),
        }
    }

    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext) {
        match msg {
            ActorEnvelope::Command(cmd) => {
                self.handle_command(&cmd, ctx).await;
            }
            ActorEnvelope::Event(evt) => {
                self.handle_event(&evt);
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

impl PromptAssemblyActor {
    /// Dispatches incoming commands to the appropriate handler.
    async fn handle_command(&mut self, cmd: &Command, ctx: &ActorContext) {
        match cmd {
            Command::AssemblePrompt { payload } => {
                self.on_assemble_prompt(payload, ctx).await;
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
            _ => {}
        }
    }

    /// Dispatches incoming events to the appropriate handler.
    fn handle_event(&mut self, evt: &Event) {
        match evt {
            Event::ToolsRegistered { payload } => {
                self.on_tools_registered(payload);
            }
            Event::PromptStrategySwitched { payload } => {
                self.on_prompt_strategy_switched(payload);
            }
            Event::PromptTemplatesLoaded { payload } => {
                self.on_prompt_templates_loaded(payload);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nullslop_actor::{ActorContext, MessageSink, RecordingSink};
    use nullslop_component::{AppState, State};
    use nullslop_protocol::ChatEntry;
    use nullslop_protocol::PinPosition;
    use nullslop_protocol::PromptStrategyId;
    use nullslop_protocol::context::PromptAssembled;

    use super::*;

    fn test_context(sink: Arc<RecordingSink>) -> ActorContext {
        let mut ctx = ActorContext::new("context", sink as Arc<dyn MessageSink>);
        ctx.set_data(State::new(AppState::default()));
        ctx
    }

    /// Creates a test actor with a fresh AppState (for handler tests).
    fn create_actor() -> (PromptAssemblyActor, State, Arc<RecordingSink>, ActorContext) {
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = ActorContext::new("context-actor", sink.clone() as Arc<dyn MessageSink>);
        let state = State::new(AppState::default());
        ctx.set_data(state.clone());
        ctx.set_data::<Box<dyn StrategyFactory>>(Box::new(DefaultStrategyFactory));
        let actor = PromptAssemblyActor::activate(&mut ctx);
        (actor, state, sink, ctx)
    }

    fn find_prompt_assembled(events: &[Event]) -> Option<PromptAssembled> {
        for evt in events {
            if let Event::PromptAssembled { payload } = evt {
                return Some(payload.clone());
            }
        }
        None
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn passthrough_assembly_emits_prompt_assembled() {
        // Given an actor with a fresh context.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        // When sending an AssemblePrompt with history.
        let session_id = SessionId::new();
        let history = vec![ChatEntry::user("hello"), ChatEntry::assistant("hi")];
        let cmd = nullslop_protocol::Command::AssemblePrompt {
            payload: AssemblePrompt {
                session_id: session_id.clone(),
                history,
                tools: vec![],
                model_name: "test".to_owned(),
            },
        };
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then a PromptAssembled event is emitted.
        let events = sink.events();
        let assembled = find_prompt_assembled(&events);
        assert!(assembled.is_some());
        let assembled = assembled.expect("should have PromptAssembled");
        assert_eq!(assembled.session_id, session_id);
        assert!(assembled.system_prompt.is_none());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn prompt_assembled_event_contains_messages() {
        // Given an actor with a fresh context.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        // When sending an AssemblePrompt with history.
        let session_id = SessionId::new();
        let history = vec![ChatEntry::user("hello"), ChatEntry::assistant("hi")];
        let cmd = nullslop_protocol::Command::AssemblePrompt {
            payload: AssemblePrompt {
                session_id: session_id.clone(),
                history,
                tools: vec![],
                model_name: "test".to_owned(),
            },
        };
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then the assembled event contains the expected messages.
        let events = sink.events();
        let assembled = find_prompt_assembled(&events).expect("should have PromptAssembled");
        assert_eq!(assembled.messages.len(), 2);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn unknown_session_gets_passthrough() {
        // Given an actor.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        // When sending an AssemblePrompt for a new session.
        let session_id = SessionId::new();
        let cmd = nullslop_protocol::Command::AssemblePrompt {
            payload: AssemblePrompt {
                session_id: session_id.clone(),
                history: vec![ChatEntry::user("test")],
                tools: vec![],
                model_name: "test".to_owned(),
            },
        };
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then assembly succeeds (auto-initialized with passthrough).
        let events = sink.events();
        assert!(find_prompt_assembled(&events).is_some());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn tools_registered_caches_definitions() {
        // Given an actor.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        // When receiving a ToolsRegistered event.
        let evt = Event::ToolsRegistered {
            payload: ToolsRegistered {
                provider: "echo-actor".to_owned(),
                definitions: vec![ToolDefinition {
                    name: "echo".to_owned(),
                    description: "echo tool".to_owned(),
                    parameters: serde_json::json!({}),
                }],
            },
        };
        actor.handle(ActorEnvelope::Event(evt), &ctx).await;

        // Then the tool definition is cached.
        assert!(actor.tool_definitions.contains_key("echo"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn prompt_strategy_switched_creates_strategy() {
        // Given an actor with an existing session.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        let session_id = SessionId::new();
        // Initialize the session with an assemble.
        let cmd = nullslop_protocol::Command::AssemblePrompt {
            payload: AssemblePrompt {
                session_id: session_id.clone(),
                history: vec![ChatEntry::user("hello")],
                tools: vec![],
                model_name: "test".to_owned(),
            },
        };
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;
        sink.clear();

        // When receiving a PromptStrategySwitched event.
        let event = Event::PromptStrategySwitched {
            payload: PromptStrategySwitched {
                session_id: session_id.clone(),
                strategy_id: PromptStrategyId::sliding_window(),
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then the strategy was created.
        let strategy = actor.strategies.get(&session_id).expect("should exist");
        assert_eq!(strategy.name(), "sliding_window");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn prompt_strategy_switched_unknown_id_is_ignored() {
        // Given an actor.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        // When receiving a PromptStrategySwitched event with an unknown strategy.
        let session_id = SessionId::new();
        let event = Event::PromptStrategySwitched {
            payload: PromptStrategySwitched {
                session_id: session_id.clone(),
                strategy_id: PromptStrategyId::new("nonexistent"),
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then no strategy is stored.
        assert!(!actor.strategies.contains_key(&session_id));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn sliding_window_strategy_limits_output() {
        // Given an actor with a session switched to sliding_window.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        let session_id = SessionId::new();

        // Switch to sliding window.
        let event = Event::PromptStrategySwitched {
            payload: PromptStrategySwitched {
                session_id: session_id.clone(),
                strategy_id: PromptStrategyId::sliding_window(),
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;
        sink.clear();

        // When assembling with more than 5 entries.
        let mut history = Vec::new();
        for i in 0..10 {
            history.push(ChatEntry::user(format!("msg {i}")));
        }
        let cmd = nullslop_protocol::Command::AssemblePrompt {
            payload: AssemblePrompt {
                session_id: session_id.clone(),
                history,
                tools: vec![],
                model_name: "test".to_owned(),
            },
        };
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then only the last 5 entries are in the output.
        let events = sink.events();
        let assembled = find_prompt_assembled(&events).expect("should have PromptAssembled");
        assert_eq!(assembled.messages.len(), 5);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn token_budget_strategy_limits_output() {
        // Given an actor with a session switched to token_budget.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        let session_id = SessionId::new();

        // Switch to token_budget.
        let event = Event::PromptStrategySwitched {
            payload: PromptStrategySwitched {
                session_id: session_id.clone(),
                strategy_id: PromptStrategyId::token_budget(),
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;
        sink.clear();

        // When assembling with many large entries that exceed the 8192 token budget.
        let mut history = Vec::new();
        for _ in 0..100 {
            // Each entry: 400 chars / 4 + 1 = 101 tokens. 100 entries = 10,100 tokens.
            history.push(ChatEntry::user("a".repeat(400)));
        }
        let cmd = nullslop_protocol::Command::AssemblePrompt {
            payload: AssemblePrompt {
                session_id: session_id.clone(),
                history,
                tools: vec![],
                model_name: "test".to_owned(),
            },
        };
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then the output is trimmed and a system prompt is set.
        let events = sink.events();
        let assembled = find_prompt_assembled(&events).expect("should have PromptAssembled");
        assert!(assembled.messages.len() < 100);
        assert!(assembled.system_prompt.is_some());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn compaction_strategy_limits_output() {
        // Given an actor with a session switched to compaction.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        let session_id = SessionId::new();

        // Switch to compaction.
        let event = Event::PromptStrategySwitched {
            payload: PromptStrategySwitched {
                session_id: session_id.clone(),
                strategy_id: PromptStrategyId::compaction(),
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;
        sink.clear();

        // When assembling with many entries that exceed the 8192 token budget.
        let mut history = Vec::new();
        for _ in 0..100 {
            history.push(ChatEntry::user("a".repeat(400)));
        }
        let cmd = nullslop_protocol::Command::AssemblePrompt {
            payload: AssemblePrompt {
                session_id: session_id.clone(),
                history,
                tools: vec![],
                model_name: "test".to_owned(),
            },
        };
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then the output is trimmed with a compaction system prompt.
        let events = sink.events();
        let assembled = find_prompt_assembled(&events).expect("should have PromptAssembled");
        assert!(assembled.messages.len() < 100);
        assert_eq!(
            assembled.system_prompt.as_deref(),
            Some(
                "Context was compacted to fit within the token budget. Earlier conversation history was summarized."
            )
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn top_pinned_entries_appear_first_in_assembled_messages() {
        // Given an actor and history with a TOP-pinned system entry plus regular entries.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        let session_id = SessionId::new();
        let history = vec![
            ChatEntry::system("important instruction").with_pin(PinPosition::Top),
            ChatEntry::user("hello"),
            ChatEntry::assistant("hi"),
        ];
        let cmd = nullslop_protocol::Command::AssemblePrompt {
            payload: AssemblePrompt {
                session_id,
                history,
                tools: vec![],
                model_name: "test".to_owned(),
            },
        };

        // When assembling.
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then the assembled messages start with the system message from the pinned entry.
        let events = sink.events();
        let assembled = find_prompt_assembled(&events).expect("should have PromptAssembled");
        assert!(!assembled.messages.is_empty());
        assert_eq!(
            assembled.messages[0],
            nullslop_protocol::LlmMessage::System {
                content: "important instruction".to_owned(),
            }
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn bottom_pin_produces_three_messages() {
        // Given history with BOTTOM-pinned entry plus regular entries.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        let session_id = SessionId::new();
        let history = vec![
            ChatEntry::user("hello"),
            ChatEntry::system("remember this").with_pin(PinPosition::Bottom),
            ChatEntry::user("what is 2+2?"),
        ];
        let cmd = nullslop_protocol::Command::AssemblePrompt {
            payload: AssemblePrompt {
                session_id,
                history,
                tools: vec![],
                model_name: "test".to_owned(),
            },
        };

        // When assembling.
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then three messages are produced.
        let events = sink.events();
        let assembled = find_prompt_assembled(&events).expect("should have PromptAssembled");
        assert_eq!(assembled.messages.len(), 3);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn bottom_pin_precedes_final_user_message() {
        // Given history with BOTTOM-pinned entry plus regular entries.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        let session_id = SessionId::new();
        let history = vec![
            ChatEntry::user("hello"),
            ChatEntry::system("remember this").with_pin(PinPosition::Bottom),
            ChatEntry::user("what is 2+2?"),
        ];
        let cmd = nullslop_protocol::Command::AssemblePrompt {
            payload: AssemblePrompt {
                session_id,
                history,
                tools: vec![],
                model_name: "test".to_owned(),
            },
        };

        // When assembling.
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then BOTTOM pin messages appear just before the final user message.
        let events = sink.events();
        let assembled = find_prompt_assembled(&events).expect("should have PromptAssembled");
        assert_eq!(
            assembled.messages[0],
            nullslop_protocol::LlmMessage::User {
                content: "hello".to_owned(),
            }
        );
        assert_eq!(
            assembled.messages[1],
            nullslop_protocol::LlmMessage::System {
                content: "remember this".to_owned(),
            }
        );
        assert_eq!(
            assembled.messages[2],
            nullslop_protocol::LlmMessage::User {
                content: "what is 2+2?".to_owned(),
            }
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn relative_pin_produces_three_messages() {
        // Given history with RELATIVE-pinned entries (no TOP/BOTTOM).
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        let session_id = SessionId::new();
        let history = vec![
            ChatEntry::user("hello"),
            ChatEntry::system("keep me").with_pin(PinPosition::Relative),
            ChatEntry::user("goodbye"),
        ];
        let cmd = nullslop_protocol::Command::AssemblePrompt {
            payload: AssemblePrompt {
                session_id,
                history,
                tools: vec![],
                model_name: "test".to_owned(),
            },
        };

        // When assembling.
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then three messages are produced.
        let events = sink.events();
        let assembled = find_prompt_assembled(&events).expect("should have PromptAssembled");
        assert_eq!(assembled.messages.len(), 3);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn relative_pin_stays_at_original_position() {
        // Given history with RELATIVE-pinned entries (no TOP/BOTTOM).
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        let session_id = SessionId::new();
        let history = vec![
            ChatEntry::user("hello"),
            ChatEntry::system("keep me").with_pin(PinPosition::Relative),
            ChatEntry::user("goodbye"),
        ];
        let cmd = nullslop_protocol::Command::AssemblePrompt {
            payload: AssemblePrompt {
                session_id,
                history,
                tools: vec![],
                model_name: "test".to_owned(),
            },
        };

        // When assembling.
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then RELATIVE-pinned entries appear at their original positions in the output.
        let events = sink.events();
        let assembled = find_prompt_assembled(&events).expect("should have PromptAssembled");
        assert_eq!(
            assembled.messages[0],
            nullslop_protocol::LlmMessage::User {
                content: "hello".to_owned(),
            }
        );
        assert_eq!(
            assembled.messages[1],
            nullslop_protocol::LlmMessage::System {
                content: "keep me".to_owned(),
            }
        );
        assert_eq!(
            assembled.messages[2],
            nullslop_protocol::LlmMessage::User {
                content: "goodbye".to_owned(),
            }
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn empty_working_history_with_only_pins() {
        // Given history with only TOP and BOTTOM pins, no working entries.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        let session_id = SessionId::new();
        let history = vec![
            ChatEntry::system("top instruction").with_pin(PinPosition::Top),
            ChatEntry::system("bottom reminder").with_pin(PinPosition::Bottom),
        ];
        let cmd = nullslop_protocol::Command::AssemblePrompt {
            payload: AssemblePrompt {
                session_id,
                history,
                tools: vec![],
                model_name: "test".to_owned(),
            },
        };

        // When assembling.
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then result is [top_messages] + [bottom_messages].
        let events = sink.events();
        let assembled = find_prompt_assembled(&events).expect("should have PromptAssembled");
        assert_eq!(assembled.messages.len(), 2);
        assert_eq!(
            assembled.messages[0],
            nullslop_protocol::LlmMessage::System {
                content: "top instruction".to_owned(),
            }
        );
        assert_eq!(
            assembled.messages[1],
            nullslop_protocol::LlmMessage::System {
                content: "bottom reminder".to_owned(),
            }
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn single_message_with_bottom_pins() {
        // Given one user message and BOTTOM pins.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        let session_id = SessionId::new();
        let history = vec![
            ChatEntry::system("reminder").with_pin(PinPosition::Bottom),
            ChatEntry::user("hello"),
        ];
        let cmd = nullslop_protocol::Command::AssemblePrompt {
            payload: AssemblePrompt {
                session_id,
                history,
                tools: vec![],
                model_name: "test".to_owned(),
            },
        };

        // When assembling.
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then result is [bottom_pins] + [user_message].
        let events = sink.events();
        let assembled = find_prompt_assembled(&events).expect("should have PromptAssembled");
        assert_eq!(assembled.messages.len(), 2);
        assert_eq!(
            assembled.messages[0],
            nullslop_protocol::LlmMessage::System {
                content: "reminder".to_owned(),
            }
        );
        assert_eq!(
            assembled.messages[1],
            nullslop_protocol::LlmMessage::User {
                content: "hello".to_owned(),
            }
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn top_pins_appear_first_in_output() {
        // Given history with TOP pin plus other entries.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        let session_id = SessionId::new();
        let history = vec![
            ChatEntry::system("top rule").with_pin(PinPosition::Top),
            ChatEntry::user("hello"),
            ChatEntry::system("relative note").with_pin(PinPosition::Relative),
            ChatEntry::user("middle"),
            ChatEntry::system("bottom reminder").with_pin(PinPosition::Bottom),
            ChatEntry::user("latest question"),
        ];
        let cmd = nullslop_protocol::Command::AssemblePrompt {
            payload: AssemblePrompt {
                session_id,
                history,
                tools: vec![],
                model_name: "test".to_owned(),
            },
        };

        // When assembling.
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then TOP pin appears as the first message.
        let events = sink.events();
        let assembled = find_prompt_assembled(&events).expect("should have PromptAssembled");
        assert_eq!(assembled.messages.len(), 6);
        assert_eq!(
            assembled.messages[0],
            nullslop_protocol::LlmMessage::System {
                content: "top rule".to_owned(),
            }
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn relative_pins_appear_in_strategy_output() {
        // Given history with RELATIVE pin plus other entries.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        let session_id = SessionId::new();
        let history = vec![
            ChatEntry::system("top rule").with_pin(PinPosition::Top),
            ChatEntry::user("hello"),
            ChatEntry::system("relative note").with_pin(PinPosition::Relative),
            ChatEntry::user("middle"),
            ChatEntry::system("bottom reminder").with_pin(PinPosition::Bottom),
            ChatEntry::user("latest question"),
        ];
        let cmd = nullslop_protocol::Command::AssemblePrompt {
            payload: AssemblePrompt {
                session_id,
                history,
                tools: vec![],
                model_name: "test".to_owned(),
            },
        };

        // When assembling.
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then RELATIVE pin is at its original position within the working history output.
        let events = sink.events();
        let assembled = find_prompt_assembled(&events).expect("should have PromptAssembled");
        assert_eq!(
            assembled.messages[2],
            nullslop_protocol::LlmMessage::System {
                content: "relative note".to_owned(),
            }
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn bottom_pins_appear_before_last_message() {
        // Given history with BOTTOM pin plus other entries.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        let session_id = SessionId::new();
        let history = vec![
            ChatEntry::system("top rule").with_pin(PinPosition::Top),
            ChatEntry::user("hello"),
            ChatEntry::system("relative note").with_pin(PinPosition::Relative),
            ChatEntry::user("middle"),
            ChatEntry::system("bottom reminder").with_pin(PinPosition::Bottom),
            ChatEntry::user("latest question"),
        ];
        let cmd = nullslop_protocol::Command::AssemblePrompt {
            payload: AssemblePrompt {
                session_id,
                history,
                tools: vec![],
                model_name: "test".to_owned(),
            },
        };

        // When assembling.
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then BOTTOM pin appears before the last message.
        let events = sink.events();
        let assembled = find_prompt_assembled(&events).expect("should have PromptAssembled");
        assert_eq!(
            assembled.messages[4],
            nullslop_protocol::LlmMessage::System {
                content: "bottom reminder".to_owned(),
            }
        );
        assert_eq!(
            assembled.messages[5],
            nullslop_protocol::LlmMessage::User {
                content: "latest question".to_owned(),
            }
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn no_pins_produces_correct_message_count() {
        // Given history with no pinned entries.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        let session_id = SessionId::new();
        let history = vec![
            ChatEntry::user("hello"),
            ChatEntry::assistant("hi"),
            ChatEntry::user("how are you?"),
        ];
        let cmd = nullslop_protocol::Command::AssemblePrompt {
            payload: AssemblePrompt {
                session_id,
                history,
                tools: vec![],
                model_name: "test".to_owned(),
            },
        };

        // When assembling.
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then output has no system prompt and 3 messages.
        let events = sink.events();
        let assembled = find_prompt_assembled(&events).expect("should have PromptAssembled");
        assert!(assembled.system_prompt.is_none());
        assert_eq!(assembled.messages.len(), 3);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn no_pins_produces_correct_message_content() {
        // Given history with no pinned entries.
        let sink = Arc::new(RecordingSink::new());
        let mut ctx = test_context(sink.clone());
        let mut actor = PromptAssemblyActor::activate(&mut ctx);

        let session_id = SessionId::new();
        let history = vec![
            ChatEntry::user("hello"),
            ChatEntry::assistant("hi"),
            ChatEntry::user("how are you?"),
        ];
        let cmd = nullslop_protocol::Command::AssemblePrompt {
            payload: AssemblePrompt {
                session_id,
                history,
                tools: vec![],
                model_name: "test".to_owned(),
            },
        };

        // When assembling.
        actor.handle(ActorEnvelope::Command(cmd), &ctx).await;

        // Then output is identical to pre-Phase 4 behavior (regression test).
        let events = sink.events();
        let assembled = find_prompt_assembled(&events).expect("should have PromptAssembled");
        assert_eq!(
            assembled.messages[0],
            nullslop_protocol::LlmMessage::User {
                content: "hello".to_owned(),
            }
        );
        assert_eq!(
            assembled.messages[1],
            nullslop_protocol::LlmMessage::Assistant {
                content: "hi".to_owned(),
                tool_calls: None,
            }
        );
        assert_eq!(
            assembled.messages[2],
            nullslop_protocol::LlmMessage::User {
                content: "how are you?".to_owned(),
            }
        );
    }

    // --- Pinning tests (migrated from coordinator) ---

    #[rstest::rstest]
    #[tokio::test]
    async fn pin_chat_entry_sets_pin_position() {
        // Given a context actor with a session that has a chat entry.
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

    #[rstest::rstest]
    #[tokio::test]
    async fn unpin_chat_entry_removes_pin() {
        // Given a context actor with a pinned entry.
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

    // --- Strategy tests (migrated from coordinator) ---

    #[rstest::rstest]
    #[tokio::test]
    async fn switch_prompt_strategy_updates_session_strategy() {
        // Given a context actor with a session.
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
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Command::RestoreStrategyState { .. }))
        );
        let events = sink.events();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::PromptStrategySwitched { .. }))
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn restore_strategy_state_stores_blob() {
        // Given a context actor with a session.
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
                .context
                .strategy_state
                .get(&(session_id.clone(), strategy_id.clone()));
            assert_eq!(stored, Some(&blob));
        }

        // And a StrategyStateUpdated event was emitted.
        let events = sink.events();
        let found = events
            .iter()
            .any(|e| matches!(e, Event::StrategyStateUpdated { .. }));
        assert!(found, "expected StrategyStateUpdated event");
    }

    // --- Template tests (migrated from projector) ---

    #[rstest::rstest]
    #[tokio::test]
    async fn prompt_templates_loaded_updates_template_store() {
        // Given a context actor.
        let (mut actor, state, _sink, ctx) = create_actor();

        let templates = vec![nullslop_protocol::PromptTemplate {
            name: "greeting".to_owned(),
            description: "A greeting".to_owned(),
            body: "Hello!".to_owned(),
        }];

        // When processing a PromptTemplatesLoaded event.
        let event = Event::PromptTemplatesLoaded {
            payload: PromptTemplatesLoaded {
                templates: templates.clone(),
                error: None,
            },
        };
        actor.handle(ActorEnvelope::Event(event), &ctx).await;

        // Then the prompt template store contains the templates.
        let guard = state.read();
        assert_eq!(guard.context.prompt_templates.len(), 1);
        assert_eq!(
            guard
                .context
                .prompt_templates
                .find_by_name("greeting")
                .map(|t| &t.body),
            Some(&"Hello!".to_owned())
        );
    }
}
