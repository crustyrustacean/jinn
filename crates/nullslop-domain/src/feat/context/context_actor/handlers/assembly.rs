//! Assembly handler — prompt assembly with pin-aware history splitting.

use crate::common::actor::ActorContext;
use crate::feat::context::protocol::command::AssemblePrompt;
use crate::feat::context::protocol::event::PromptAssembled;
use crate::protocol::{
    ChatEntry, ChatEntryKind, Event, LlmMessage, PinPosition, entries_to_messages,
};

use crate::feat::context::env_context::{build_env_context, load_project_context_files};
use crate::feat::context::tool_prompt::build_tool_context_block;
use crate::feat::skills::format::format_skills_for_prompt;

use super::super::PromptAssemblyActor;

impl PromptAssemblyActor {
    /// Handles [`AssemblePrompt`] by assembling the prompt from history.
    ///
    /// The assembly pipeline:
    /// 1. Splits history into TOP/BOTTOM pins and working history.
    /// 2. Builds system prompt sections (skills, pinned system, env context, tools).
    /// 3. Re-injects pins and emits the assembled prompt.
    #[allow(clippy::too_many_lines)]
    pub(in crate::feat::context::context_actor) async fn on_assemble_prompt(
        &mut self,
        cmd: &AssemblePrompt,
        ctx: &ActorContext,
    ) {
        let session_id = cmd.session_id.clone();

        // Pre-processing: split history into TOP/BOTTOM pins and working history.
        // CPU-bound: clone + filter — offloaded to blocking thread to avoid
        // starving the tokio runtime during large history processing.
        let history = cmd.history.clone();
        let (top_pins, bottom_pins, working_history) = tokio::task::spawn_blocking(move || {
            let top_pins: Vec<ChatEntry> = history
                .iter()
                .filter(|e| {
                    e.pin_position() == Some(PinPosition::Top)
                        && !matches!(e.kind, ChatEntryKind::Thinking(_))
                })
                .cloned()
                .collect();

            let bottom_pins: Vec<ChatEntry> = history
                .iter()
                .filter(|e| {
                    e.pin_position() == Some(PinPosition::Bottom)
                        && !matches!(e.kind, ChatEntryKind::Thinking(_))
                })
                .cloned()
                .collect();

            let working_history: Vec<ChatEntry> = history
                .iter()
                .filter(|e| {
                    (e.pin_position().is_none() || e.pin_position() == Some(PinPosition::Relative))
                        && !matches!(e.kind, ChatEntryKind::Thinking(_))
                        && (!e.ignored || e.is_pinned())
                })
                .cloned()
                .collect();

            (top_pins, bottom_pins, working_history)
        })
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(err = ?e, "spawn_blocking panicked during prompt assembly");
            (vec![], vec![], vec![])
        });

        // Convert pin entries to messages (needed for system prompt assembly).
        let top_messages = entries_to_messages(&top_pins);
        let bottom_messages = entries_to_messages(&bottom_pins);

        // Extract pinned System entry contents and non-system top messages.
        let pinned_system_contents: Vec<String> = top_messages
            .iter()
            .filter_map(|m| match m {
                LlmMessage::System { content } => Some(content.clone()),
                _ => None,
            })
            .collect();
        let top_non_system: Vec<LlmMessage> = top_messages
            .into_iter()
            .filter(|m| !matches!(m, LlmMessage::System { .. }))
            .collect();

        // Build system prompt sections.
        let skills_block = {
            let guard = self.state.read();
            format_skills_for_prompt(&guard.context.skills)
        };

        let env_context = {
            let cwd = {
                let guard = self.state.read();
                guard
                    .session
                    .get(&session_id)
                    .map_or_else(|| std::path::PathBuf::from("."), |s| s.cwd().to_path_buf())
            };
            let context_files = load_project_context_files(&cwd).await;
            let guard = self.state.read();
            let persona = guard
                .session
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

        let tool_block = {
            let guard = self.state.read();
            build_tool_context_block(&guard.context.tool_definitions)
        };

        // Assemble system parts: skills → pinned system → env context → tool block.
        let mut system_parts: Vec<String> = Vec::new();
        if !skills_block.is_empty() {
            system_parts.push(skills_block);
        }
        system_parts.extend(pinned_system_contents);
        if !env_context.is_empty() {
            system_parts.push(env_context);
        }
        if let Some(block) = tool_block {
            system_parts.push(block);
        }

        // Convert working history to messages.
        let messages = entries_to_messages(&working_history);

        // Post-processing: re-inject pins and build final messages.
        let mut final_messages = Vec::new();

        // System message: concat system parts.
        let full_system = if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n\n"))
        };

        if let Some(content) = full_system {
            final_messages.push(LlmMessage::System { content });
        }
        final_messages.extend(top_non_system);
        final_messages.extend(messages);

        // Insert BOTTOM pins just before the last message.
        if final_messages.last().is_some() && !bottom_messages.is_empty() {
            let last = final_messages.pop().expect("just checked non-empty");
            final_messages.extend(bottom_messages);
            final_messages.push(last);
        } else {
            final_messages.extend(bottom_messages);
        }

        let _ = ctx.send_event(Event::PromptAssembled(PromptAssembled {
            session_id,
            system_prompt: None,
            messages: final_messages,
        }));
    }
}
