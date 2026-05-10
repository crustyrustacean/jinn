//! Prompt assembly actor — assembles LLM-ready prompts from chat history.
//!
//! Subscribes to [`AssemblePrompt`] commands and [`PromptStrategySwitched`] events,
//! runs the configured strategy for each session, and emits [`PromptAssembled`]
//! events when complete.
//!
//! Unknown sessions are automatically initialized with `PassthroughStrategy`.
//! Strategy switching uses a [`StrategyFactory`] injected via [`ActorContext`] data.

use std::collections::HashMap;

use nullslop_actor::{Actor, ActorContext, ActorEnvelope, SystemMessage};
use nullslop_context::{
    AssemblyContext, CharRatioEstimator, DefaultStrategyFactory, PromptAssembly, StrategyFactory,
    estimate_entry_tokens,
};
use nullslop_protocol::context::{
    AssemblePrompt, PromptAssembled, PromptStrategySwitched,
};
use nullslop_protocol::tool::ToolsRegistered;
use nullslop_protocol::{
    ChatEntry, Event, PinPosition, SessionId, ToolDefinition, entries_to_messages,
};

/// Direct message type for the prompt assembly actor (unused for now).
pub enum ContextDirectMsg {}

/// The prompt assembly actor.
pub struct PromptAssemblyActor {
    /// Per-session prompt assembly strategies.
    strategies: HashMap<SessionId, Box<dyn PromptAssembly>>,
    /// Cached tool definitions from [`ToolsRegistered`] events.
    tool_definitions: HashMap<String, ToolDefinition>,
    /// Factory for creating new strategies on switch.
    factory: Option<Box<dyn StrategyFactory>>,
}

impl Actor for PromptAssemblyActor {
    type Message = ContextDirectMsg;

    fn activate(ctx: &mut ActorContext) -> Self {
        ctx.subscribe_command::<AssemblePrompt>();
        ctx.subscribe_event::<PromptStrategySwitched>();
        ctx.subscribe_event::<ToolsRegistered>();
        let factory = ctx
            .take_data::<Box<dyn StrategyFactory>>()
            .unwrap_or_else(|| Box::new(DefaultStrategyFactory));
        Self {
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
    async fn handle_command(&mut self, cmd: &nullslop_protocol::Command, ctx: &ActorContext) {
        match cmd {
            nullslop_protocol::Command::AssemblePrompt { payload } => {
                self.on_assemble_prompt(payload, ctx).await;
            }

            _ => {}
        }
    }

    /// Dispatches incoming events to the appropriate handler.
    fn handle_event(&mut self, evt: &nullslop_protocol::Event) {
        match evt {
            Event::ToolsRegistered { payload } => {
                self.on_tools_registered(payload);
            }
            Event::PromptStrategySwitched { payload } => {
                self.on_prompt_strategy_switched(payload);
            }
            _ => {}
        }
    }

    /// Lazily initializes a passthrough strategy for unknown sessions.
    fn ensure_strategy(&mut self, session_id: &SessionId) {
        if !self.strategies.contains_key(session_id) {
            self.strategies.insert(
                session_id.clone(),
                Box::new(nullslop_context::PassthroughStrategy),
            );
        }
    }

    /// Handles [`AssemblePrompt`] by running the session's strategy.
    async fn on_assemble_prompt(&mut self, cmd: &AssemblePrompt, ctx: &ActorContext) {
        let session_id = cmd.session_id.clone();
        self.ensure_strategy(&session_id);
        let tools: Vec<ToolDefinition> = cmd
            .tools
            .iter()
            .cloned()
            .chain(
                self.tool_definitions
                    .values()
                    .filter(|td| !cmd.tools.iter().any(|t| t.name == td.name))
                    .cloned(),
            )
            .collect();

        // Pre-processing: split history into TOP/BOTTOM pins and working history.
        let top_pins: Vec<ChatEntry> = cmd
            .history
            .iter()
            .filter(|e| e.pin_position() == Some(PinPosition::Top))
            .cloned()
            .collect();

        let bottom_pins: Vec<ChatEntry> = cmd
            .history
            .iter()
            .filter(|e| e.pin_position() == Some(PinPosition::Bottom))
            .cloned()
            .collect();

        let working_history: Vec<ChatEntry> = cmd
            .history
            .iter()
            .filter(|e| {
                e.pin_position().is_none() || e.pin_position() == Some(PinPosition::Relative)
            })
            .cloned()
            .collect();

        // Estimate reserved tokens for TOP/BOTTOM pins.
        let estimator = CharRatioEstimator;
        let reserved_tokens: usize = top_pins
            .iter()
            .chain(bottom_pins.iter())
            .map(|e| estimate_entry_tokens(&estimator, e))
            .sum();

        #[expect(
            clippy::expect_used,
            reason = "strategy was just ensured by ensure_strategy above"
        )]
        let strategy = self
            .strategies
            .get(&session_id)
            .expect("strategy was just ensured");
        let context = AssemblyContext {
            history: &working_history,
            tools: &tools,
            model_name: &cmd.model_name,
            session_id: &session_id,
            budget_offset: reserved_tokens,
        };
        let result = match strategy.assemble(&context).await {
            Ok(assembled) => assembled,
            Err(e) => {
                tracing::error!("prompt assembly failed: {e:?}");
                return;
            }
        };

        // Post-processing: re-inject TOP and BOTTOM pins.
        let mut messages = result.messages;

        // Convert pin entries to messages.
        let top_messages = entries_to_messages(&top_pins);
        let bottom_messages = entries_to_messages(&bottom_pins);

        // Insert BOTTOM pins just before the last message.
        if messages.last().is_some() {
            #[expect(clippy::expect_used, reason = "just checked non-empty")]
            let last = messages.pop().expect("just checked non-empty");
            messages.extend(bottom_messages);
            messages.push(last);
        } else {
            messages.extend(bottom_messages);
        }

        // Prepend TOP pins.
        let mut final_messages = top_messages;
        final_messages.append(&mut messages);

        let _ = ctx.send_event(Event::PromptAssembled {
            payload: PromptAssembled {
                session_id,
                system_prompt: result.system_prompt,
                messages: final_messages,
            },
        });
    }

    /// Handles [`PromptStrategySwitched`] by creating a new strategy via the factory.
    fn on_prompt_strategy_switched(&mut self, evt: &PromptStrategySwitched) {
        let Some(factory) = self.factory.as_ref() else {
            tracing::error!("no strategy factory available");
            return;
        };
        match factory.create(&evt.strategy_id) {
            Ok(new_strategy) => {
                self.strategies.insert(evt.session_id.clone(), new_strategy);
            }
            Err(e) => {
                tracing::error!("failed to create strategy '{}': {e:?}", evt.strategy_id);
            }
        }
    }

    /// Caches tool definitions from a [`ToolsRegistered`] event.
    fn on_tools_registered(&mut self, evt: &ToolsRegistered) {
        for def in &evt.definitions {
            self.tool_definitions.insert(def.name.clone(), def.clone());
        }
    }


}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nullslop_actor::{ActorContext, MessageSink, RecordingSink};
    use nullslop_protocol::ChatEntry;
    use nullslop_protocol::PromptStrategyId;

    use super::*;

    fn test_context(sink: Arc<RecordingSink>) -> ActorContext {
        ActorContext::new("context", sink as Arc<dyn MessageSink>)
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
    async fn prompt_strategy_switched_updates_strategy_name() {
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

        // And the strategy is now sliding_window.
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
}
