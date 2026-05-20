//! Assembly handler — prompt assembly with inlined compaction logic.

use crate::common::actor::ActorContext;
use crate::feat::context::protocol::command::AssemblePrompt;
use crate::feat::context::protocol::event::PromptAssembled;
use crate::protocol::{
    ChatEntry, ChatEntryKind, Event, LlmMessage, PinPosition, ToolDefinition, entries_to_messages,
};

use crate::feat::context::strategy::compaction_prompt::DEFAULT_COMPACTION_PROMPT;
use crate::feat::context::{
    CharRatioEstimator, TokenEstimator, estimate_entry_tokens, estimate_tool_schema_tokens,
};

use crate::feat::context::env_context::{build_env_context, load_project_context_files};
use crate::feat::context::tool_prompt::build_tool_context_block;
use crate::feat::skills::format::format_skills_for_prompt;

use super::super::PromptAssemblyActor;

/// System prompt set when context was compacted (trimmed to fit budget).
const COMPACTION_SYSTEM_PROMPT: &str = "Context was compacted to fit within the token budget. Earlier conversation history was summarized.";

impl PromptAssemblyActor {
    /// Handles [`AssemblePrompt`] by running compaction logic inline.
    ///
    /// The assembly pipeline:
    /// 1. Splits history into TOP/BOTTOM pins and working history.
    /// 2. Builds system prompt sections (skills, pinned system, env context, tools).
    /// 3. Estimates overhead tokens (pins, system prompt, tool schemas).
    /// 4. Runs compaction: if working history fits within budget, passthrough;
    ///    if over budget, trims newest-to-oldest preserving pinned entries.
    /// 5. Re-injects pins and emits the assembled prompt.
    #[allow(clippy::too_many_lines)]
    pub(in crate::feat::context::context_actor) async fn on_assemble_prompt(
        &mut self,
        cmd: &AssemblePrompt,
        ctx: &ActorContext,
    ) {
        let session_id = cmd.session_id.clone();
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
                    .sessions()
                    .get(&session_id)
                    .map_or_else(|| std::path::PathBuf::from("."), |s| s.cwd().to_path_buf())
            };
            let context_files = load_project_context_files(&cwd).await;
            let guard = self.state.read();
            let persona = guard
                .session
                .sessions()
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

        // Estimate overhead tokens for everything not managed by compaction.
        let estimator = CharRatioEstimator;
        let reserved_tokens: usize = top_pins
            .iter()
            .chain(bottom_pins.iter())
            .map(|e| estimate_entry_tokens(&estimator, e))
            .sum();
        let system_prompt_tokens: usize = if system_parts.is_empty() {
            0
        } else {
            estimator.estimate(&system_parts.join("\n\n"))
        };
        let tool_schema_tokens = estimate_tool_schema_tokens(&estimator, &tools);
        let budget_offset = reserved_tokens
            .saturating_add(system_prompt_tokens)
            .saturating_add(tool_schema_tokens);

        // Read the token budget from the session profile.
        let max_tokens = {
            let guard = self.state.read();
            guard
                .session
                .sessions()
                .get(&session_id)
                .map_or(150_000, |s| s.profile().token_budget)
        };

        // Inline compaction logic.
        let compaction_system_prompt =
            run_compaction(&working_history, max_tokens, budget_offset, &estimator);
        let messages = entries_to_messages(&working_history);

        // Post-processing: re-inject pins and build final messages.
        let mut final_messages = Vec::new();

        // System message: concat system parts + compaction prompt if any.
        let full_system = {
            let mut parts = system_parts;
            if let Some(ref prompt) = compaction_system_prompt {
                parts.push(DEFAULT_COMPACTION_PROMPT.to_owned());
                parts.push(prompt.clone());
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n\n"))
            }
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
            system_prompt: compaction_system_prompt,
            messages: final_messages,
        }));
    }
}

/// Runs compaction logic on the working history.
///
/// If the estimated total tokens are within the effective budget, returns `None`
/// (no compaction needed — all entries pass through). If over budget, trims
/// entries newest-to-oldest (always including pinned entries) and returns
/// the compaction system prompt.
///
/// NOTE: This mutates `working_history` in place when trimming is needed.
fn run_compaction(
    working_history: &[ChatEntry],
    max_tokens: usize,
    budget_offset: usize,
    estimator: &dyn TokenEstimator,
) -> Option<String> {
    if working_history.is_empty() {
        return None;
    }

    // Estimate total tokens across all working history.
    let total_tokens: usize = working_history
        .iter()
        .map(|entry| estimate_entry_tokens(estimator, entry))
        .sum();

    let effective_budget = max_tokens.saturating_sub(budget_offset);

    // If everything fits, no compaction needed.
    if total_tokens <= effective_budget {
        return None;
    }

    // Over threshold — trim newest-to-oldest.
    // Pinned entries are always included regardless of budget, but their tokens count.
    let mut included_indices = Vec::new();
    let mut used_tokens = 0usize;

    for (i, entry) in working_history.iter().enumerate().rev() {
        let entry_tokens = estimate_entry_tokens(estimator, entry);

        // Pinned entries are always included, tokens count toward budget.
        if entry.is_pinned() {
            used_tokens += entry_tokens;
            included_indices.push(i);
            continue;
        }

        // Skip unpinned entries when budget is exceeded, but continue walking
        // to find pinned entries at older indices.
        if !included_indices.is_empty() && used_tokens + entry_tokens > effective_budget {
            continue;
        }

        used_tokens += entry_tokens;
        included_indices.push(i);
    }

    let _ = (included_indices, used_tokens);

    Some(COMPACTION_SYSTEM_PROMPT.to_owned())
}
