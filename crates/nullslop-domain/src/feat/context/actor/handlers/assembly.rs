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

use super::super::PromptAssemblyActor;

impl PromptAssemblyActor {
    /// Lazily initializes a passthrough strategy for unknown sessions.
    pub(in crate::feat::context::actor) fn ensure_strategy(&mut self, session_id: &SessionId) {
        if !self.strategies.contains_key(session_id) {
            self.strategies
                .insert(session_id.clone(), Box::new(PassthroughStrategy));
        }
    }

    /// Handles [`AssemblePrompt`] by running the session's strategy.
    pub(in crate::feat::context::actor) async fn on_assemble_prompt(
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
}
