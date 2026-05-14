//! Assembly handlers — prompt assembly and strategy initialization.

use crate::common::actor::ActorContext;
use crate::feat::context::protocol::command::AssemblePrompt;
use crate::feat::context::protocol::event::PromptAssembled;
use crate::protocol::{
    ChatEntry, Event, LlmMessage, PinPosition, SessionId, ToolDefinition, entries_to_messages,
};

use crate::feat::context::{
    AssemblyContext, CharRatioEstimator, PassthroughStrategy, estimate_entry_tokens,
};

use crate::feat::context::env_context::{build_env_context, load_project_context_files};
use crate::feat::skills::format::format_skills_for_prompt;

use super::super::PromptAssemblyActor;

impl PromptAssemblyActor {
    /// Lazily initializes a passthrough strategy for unknown sessions.
    pub(in crate::feat::context::context_actor) fn ensure_strategy(
        &mut self,
        session_id: &SessionId,
    ) {
        if !self.strategies.contains_key(session_id) {
            self.strategies
                .insert(session_id.clone(), Box::new(PassthroughStrategy));
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

        // Prepend skills block (if any skills are loaded).
        let skills_block = {
            let guard = self.state.read();
            format_skills_for_prompt(&guard.context.skills)
        };

        // Prepend TOP pins.
        let mut final_messages = top_messages;
        final_messages.append(&mut messages);

        // Prepend skills block as a system message before everything else.
        if !skills_block.is_empty() {
            final_messages.insert(
                0,
                LlmMessage::System {
                    content: skills_block,
                },
            );
        }

        // Build and prepend environment context (persona, project files, date, CWD).
        let env_context = {
            let guard = self.state.read();
            let cwd = guard
                .session
                .sessions
                .get(&session_id)
                .map_or_else(|| std::path::PathBuf::from("."), |s| s.cwd().to_path_buf());
            let context_files = load_project_context_files(&cwd);
            let persona = guard.context.active_persona.as_ref();
            build_env_context(persona, &context_files, &cwd)
        };
        if !env_context.is_empty() {
            final_messages.insert(
                0,
                LlmMessage::System {
                    content: env_context,
                },
            );
        }

        let _ = ctx.send_event(Event::PromptAssembled(PromptAssembled {
            session_id,
            system_prompt: result.system_prompt,
            messages: final_messages,
        }));
    }
}
