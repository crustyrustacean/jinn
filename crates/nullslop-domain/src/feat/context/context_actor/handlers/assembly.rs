//! Assembly handlers — prompt assembly and strategy initialization.

use crate::common::actor::ActorContext;
use crate::feat::context::protocol::command::AssemblePrompt;
use crate::feat::context::protocol::event::PromptAssembled;
use crate::protocol::{
    ChatEntry, ChatEntryKind, Event, LlmMessage, PinPosition, PromptStrategyId, SessionId,
    ToolDefinition, entries_to_messages,
};

use crate::feat::context::strategy::types::StrategyConfig;
use crate::feat::context::{
    AssemblyContext, CharRatioEstimator, PassthroughStrategy, estimate_entry_tokens,
};

use crate::feat::context::env_context::{build_env_context, load_project_context_files};
use crate::feat::context::tool_prompt::build_tool_context_block;
use crate::feat::skills::format::format_skills_for_prompt;

use super::super::PromptAssemblyActor;

impl PromptAssemblyActor {
    /// Lazily initializes a strategy for sessions not yet tracked by this actor.
    ///
    /// Reads the session's active strategy ID and token budget from state,
    /// then creates the appropriate strategy via the factory.
    /// Falls back to passthrough if no factory is available.
    pub(in crate::feat::context::context_actor) fn ensure_strategy(
        &mut self,
        session_id: &SessionId,
    ) {
        if self.strategies.contains_key(session_id) {
            return;
        }
        let Some(factory) = self.factory.as_ref() else {
            self.strategies
                .insert(session_id.clone(), Box::new(PassthroughStrategy));
            return;
        };
        let (strategy_id, config) = {
            let guard = self.state.read();
            match guard.session.sessions.get(session_id) {
                Some(session) => {
                    let sid = session.active_strategy().clone();
                    let cfg = if sid == PromptStrategyId::sliding_window() {
                        StrategyConfig::SlidingWindow {
                            // TODO: Phase 2 — read from session.profile().sliding_window_size
                            window_size: 5,
                        }
                    } else if sid == PromptStrategyId::token_budget() {
                        StrategyConfig::TokenBudget {
                            budget: session.profile().token_budget,
                        }
                    } else if sid == PromptStrategyId::compaction() {
                        StrategyConfig::Compaction {
                            budget: session.profile().token_budget,
                        }
                    } else {
                        StrategyConfig::Passthrough
                    };
                    (sid, cfg)
                }
                None => (
                    PromptStrategyId::passthrough(),
                    StrategyConfig::Passthrough,
                ),
            }
        };
        match factory.create(&strategy_id, &config) {
            Ok(strategy) => {
                self.strategies.insert(session_id.clone(), strategy);
            }
            Err(e) => {
                tracing::error!("ensure_strategy failed: {e:?}");
                self.strategies
                    .insert(session_id.clone(), Box::new(PassthroughStrategy));
            }
        }
    }

    /// Handles [`AssemblePrompt`] by running the session's strategy.
    #[allow(clippy::too_many_lines)]
    pub(in crate::feat::context::context_actor) async fn on_assemble_prompt(
        &mut self,
        cmd: &AssemblePrompt,
        ctx: &ActorContext,
    ) {
        let session_id = cmd.session_id.clone();
        self.ensure_strategy(&session_id);
        let tools: Vec<ToolDefinition> = cmd
            .tools
            .iter()
            .cloned()
            .chain({
                let guard = self.state.read();
                guard
                    .context
                    .tool_definitions
                    .values()
                    .filter(|td| !cmd.tools.iter().any(|t| t.name == td.name))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .collect();

        // Pre-processing: split history into TOP/BOTTOM pins and working history.
        let top_pins: Vec<ChatEntry> = cmd
            .history
            .iter()
            .filter(|e| {
                e.pin_position() == Some(PinPosition::Top)
                    && !matches!(e.kind, ChatEntryKind::Thinking(_))
            })
            .cloned()
            .collect();

        let bottom_pins: Vec<ChatEntry> = cmd
            .history
            .iter()
            .filter(|e| {
                e.pin_position() == Some(PinPosition::Bottom)
                    && !matches!(e.kind, ChatEntryKind::Thinking(_))
            })
            .cloned()
            .collect();

        let working_history: Vec<ChatEntry> = cmd
            .history
            .iter()
            .filter(|e| {
                (e.pin_position().is_none() || e.pin_position() == Some(PinPosition::Relative))
                    && !matches!(e.kind, ChatEntryKind::Thinking(_))
                    && (!e.ignored || e.is_pinned())
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
        let mut messages: Vec<LlmMessage> = result.messages;

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

        // Build system prompt sections.
        let skills_block = {
            let guard = self.state.read();
            format_skills_for_prompt(&guard.context.skills)
        };

        // Extract pinned System entry contents from top_messages.
        let pinned_system_contents: Vec<String> = top_messages
            .iter()
            .filter_map(|m| match m {
                LlmMessage::System { content } => Some(content.clone()),
                _ => None,
            })
            .collect();

        // Remove System messages from top_messages — they'll go into the system prompt.
        let top_non_system: Vec<LlmMessage> = top_messages
            .into_iter()
            .filter(|m| !matches!(m, LlmMessage::System { .. }))
            .collect();

        let env_context = {
            // Extract CWD and load context files (async I/O).
            let cwd = {
                let guard = self.state.read();
                guard
                    .session
                    .sessions
                    .get(&session_id)
                    .map_or_else(|| std::path::PathBuf::from("."), |s| s.cwd().to_path_buf())
            };
            let context_files = load_project_context_files(&cwd).await;
            // Re-acquire lock for persona lookup.
            let guard = self.state.read();
            // Look up persona from this session's profile, not the global default.
            // Falls back to coding-assistant if the session's persona name is not found.
            let persona = guard
                .session
                .sessions
                .get(&session_id)
                .and_then(|s| {
                    let name = s.persona_name();
                    guard.context.personas.iter().find(|p| p.name == name)
                })
                .or_else(|| {
                    guard
                        .context
                        .personas
                        .iter()
                        .find(|p| p.name == "coding-assistant")
                });
            build_env_context(persona, &context_files, &cwd)
        };

        // Concatenate all system sections into one message.
        // Order: skills (lowest priority) → pinned System entries → env_context (highest priority, closest to conversation).
        let mut system_parts: Vec<String> = Vec::new();
        if !skills_block.is_empty() {
            system_parts.push(skills_block);
        }
        system_parts.extend(pinned_system_contents);
        if !env_context.is_empty() {
            system_parts.push(env_context);
        }

        // Tool context block (available tools + tool guidelines).
        let tool_block = {
            let guard = self.state.read();
            build_tool_context_block(&guard.context.tool_definitions)
        };
        if let Some(block) = tool_block {
            system_parts.push(block);
        }

        // Assemble final messages: single system message + top pins (non-System) + working history + bottom pins.
        let mut final_messages = Vec::new();

        if !system_parts.is_empty() {
            final_messages.push(LlmMessage::System {
                content: system_parts.join("\n\n"),
            });
        }

        final_messages.extend(top_non_system);
        final_messages.append(&mut messages);

        let _ = ctx.send_event(Event::PromptAssembled(PromptAssembled {
            session_id,
            system_prompt: result.system_prompt,
            messages: final_messages,
        }));
    }
}
