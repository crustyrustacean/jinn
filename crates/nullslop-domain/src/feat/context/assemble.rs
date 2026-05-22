//! Prompt assembly — builds LLM-ready messages from session state.
//!
//! Produces [`AssembledPrompt`] via a single pure function call.
//! The assembly pipeline reads all context (skills, persona, context files,
//! tools, history) from [`AppState`] in one pass, splits pinned entries,
//! builds the system prompt, converts history to messages, and counts tokens.

use crate::common::app_state::AppState;
use crate::feat::context::env_context::build_env_context;
use crate::feat::context::strategy::token_estimator::TokenCounter;
use crate::feat::context::tool_prompt::build_tool_context_block;
use crate::feat::skills::format::format_skills_for_prompt;
use crate::protocol::{
    ChatEntry, ChatEntryKind, LlmMessage, PinPosition, SessionId, ToolDefinition, entries_to_messages,
};

/// Fully assembled LLM prompt — everything a provider needs to make a request.
///
/// Produced by [`assemble_prompt`]. Token count is computed at construction time
/// via the provided [`TokenCounter`]. Contains messages, tool definitions,
/// and the estimated token count.
#[derive(Debug, Clone)]
pub struct AssembledPrompt {
    /// The session this prompt was assembled for.
    pub session_id: SessionId,
    /// The assembled conversation messages ready for the LLM.
    pub messages: Vec<LlmMessage>,
    /// Tool definitions to include in the API request.
    pub tool_definitions: Vec<ToolDefinition>,
    /// Estimated token count (tiktoken o200k_base) of all messages.
    estimated_tokens: u32,
}

impl AssembledPrompt {
    /// Returns the estimated token count of this assembled prompt.
    #[must_use]
    pub fn estimated_tokens(&self) -> u32 {
        self.estimated_tokens
    }
}

/// Assembles a complete LLM prompt from session state in a single pass.
///
/// Reads all context (skills, persona, context files, tools, history) from
/// [`AppState`] in one read-lock scope, produces messages and counts tokens.
///
/// # Assembly pipeline
///
/// 1. Read skills, persona, context files, tools, history from state.
/// 2. Split history into TOP/BOTTOM pins and working history.
/// 3. Build system prompt sections (skills, pinned system, env context, tools).
/// 4. Convert working history to messages via [`entries_to_messages`].
/// 5. Re-inject pins in correct positions.
/// 6. Count tokens in all assembled messages.
/// 7. Return [`AssembledPrompt`].
///
/// # Panics
///
/// Panics if the given `session_id` does not exist in the session map.
#[must_use]
pub fn assemble_prompt(
    state: &AppState,
    session_id: &SessionId,
    counter: &dyn TokenCounter,
) -> AssembledPrompt {
    let session = state.session(session_id);

    // Gather all inputs in one scope.
    let skills_block = format_skills_for_prompt(&state.context.skills);
    let tool_defs: Vec<ToolDefinition> = state.context.tool_definitions.values().cloned().collect();
    let tool_block = build_tool_context_block(&state.context.tool_definitions);
    let cwd = session.cwd().to_path_buf();
    let context_files = &state.context.context_files;

    let persona = state
        .context
        .personas
        .iter()
        .find(|p| p.name == session.persona_name())
        .or_else(|| {
            state
                .context
                .personas
                .iter()
                .find(|p| p.name == "coding-assistant")
        });

    let history = session.history();

    // Split history into TOP/BOTTOM pins and working history.
    let (top_pins, bottom_pins, working_history) = split_history(history);

    // Convert pin entries to messages.
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

    // Build env context using cached context files.
    let env_context = build_env_context(persona, context_files, &cwd);

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

    // Build final message list.
    let mut final_messages = Vec::new();

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

    // Count tokens.
    let estimated_tokens = count_messages(&final_messages, counter);

    AssembledPrompt {
        session_id: session_id.clone(),
        messages: final_messages,
        tool_definitions: tool_defs,
        estimated_tokens,
    }
}

/// Splits history entries into TOP pins, BOTTOM pins, and working history.
///
/// Filters out thinking entries and ignored (non-pinned) entries.
fn split_history(history: &[ChatEntry]) -> (Vec<ChatEntry>, Vec<ChatEntry>, Vec<ChatEntry>) {
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
}

/// Counts tokens across all messages.
fn count_messages(messages: &[LlmMessage], counter: &dyn TokenCounter) -> u32 {
    messages
        .iter()
        .map(|msg| match msg {
            LlmMessage::System { content }
            | LlmMessage::User { content }
            | LlmMessage::Assistant { content, .. }
            | LlmMessage::Tool { content, .. } => counter.count(content),
        })
        .sum::<usize>() as u32
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::common::state::State;
    use crate::feat::context::env_context::ContextFile;
    use crate::feat::context::strategy::token_estimator::TiktokenCounter;
    use crate::feat::persona::Persona;
    use crate::feat::skills::Skill;
    use crate::feat::tools_actor::tool_types::ToolDefinition;
    use crate::protocol::{ChatEntry, SessionId};

    fn counter() -> TiktokenCounter {
        TiktokenCounter::o200k_base()
    }

    fn make_skill(name: &str) -> Skill {
        Skill {
            name: name.to_owned(),
            description: format!("{name} skill"),
            file_path: std::path::PathBuf::from(format!("/skills/{name}/SKILL.md")),
            base_dir: std::path::PathBuf::from(format!("/skills/{name}")),
        }
    }

    fn make_tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_owned(),
            description: format!("{name} tool"),
            parameters: serde_json::json!({"type": "object"}),
            prompt_snippet: Some(format!("{name} does things")),
            prompt_guidelines: vec![],
        }
    }

    fn state_with_history(entries: Vec<ChatEntry>) -> (State, SessionId) {
        let state = State::new(AppState::default());
        let session_id = {
            let mut guard = state.write();
            let session = guard.active_session_mut();
            for entry in entries {
                session.push_entry(entry);
            }
            guard.session.active_session_id().clone()
        };
        (state, session_id)
    }

    #[test]
    fn assemble_prompt_with_empty_history_produces_only_system() {
        // Given a state with skills but no history.
        let (state, session_id) = state_with_history(vec![]);
        {
            let mut guard = state.write();
            guard.context.skills = vec![make_skill("test-skill")];
        }

        // When assembling the prompt.
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter());

        // Then the first message is System and contains the skill.
        assert!(!result.messages.is_empty(), "should have at least a system message");
        match &result.messages[0] {
            LlmMessage::System { content } => {
                assert!(content.contains("test-skill"));
            }
            other => panic!("expected System message, got {other:?}"),
        }
    }

    #[test]
    fn assemble_prompt_with_no_skills_or_tools_still_has_env_context() {
        // Given a state with no skills, persona, tools, or context files, and no history.
        let (state, session_id) = state_with_history(vec![]);

        // When assembling the prompt.
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter());

        // Then the system message still has date and CWD from env context.
        assert!(!result.messages.is_empty());
        match &result.messages[0] {
            LlmMessage::System { content } => {
                assert!(content.contains("Current date:"));
                assert!(content.contains("Current working directory:"));
            }
            other => panic!("expected System message, got {other:?}"),
        }
    }

    #[test]
    fn assemble_prompt_places_bottom_pins_before_last_message() {
        // Given a session with a user message and a bottom-pinned entry.
        let user = ChatEntry::user("hello");
        let mut pinned = ChatEntry::assistant("pinned assistant");
        pinned.pin_position = Some(PinPosition::Bottom);

        let (state, session_id) = state_with_history(vec![user.clone(), pinned]);

        // When assembling the prompt.
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter());

        // Then the bottom pin appears just before the last message.
        assert!(result.messages.len() >= 2, "need at least 2 messages");
        let second_last = &result.messages[result.messages.len() - 2];
        match second_last {
            LlmMessage::Assistant { content, .. } => {
                assert_eq!(content, "pinned assistant");
            }
            other => panic!("expected Assistant as second-to-last, got {other:?}"),
        }
        let last = result.messages.last().expect("has last");
        match last {
            LlmMessage::User { content } => {
                assert_eq!(content, "hello");
            }
            other => panic!("expected User as last, got {other:?}"),
        }
    }

    #[test]
    fn assemble_prompt_excludes_thinking_entries() {
        // Given a session with a thinking entry and a user message.
        let thinking = ChatEntry::thinking("internal thoughts");
        let user = ChatEntry::user("hello");
        let (state, session_id) = state_with_history(vec![thinking, user]);

        // When assembling the prompt.
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter());

        // Then no message contains the thinking text.
        for msg in &result.messages {
            match msg {
                LlmMessage::System { content }
                | LlmMessage::User { content }
                | LlmMessage::Assistant { content, .. }
                | LlmMessage::Tool { content, .. } => {
                    assert!(
                        !content.contains("internal thoughts"),
                        "thinking entry should be excluded"
                    );
                }
            }
        }
    }

    #[test]
    fn assemble_prompt_token_count_is_accurate() {
        // Given a session with a user message.
        let (state, session_id) = state_with_history(vec![ChatEntry::user("hello world")]);

        // When assembling the prompt.
        let counter = counter();
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter);

        // Then token count is > 0 and matches manual count.
        assert!(
            result.estimated_tokens() > 0,
            "token count should be positive"
        );

        // Manual count.
        let manual: usize = result
            .messages
            .iter()
            .map(|m| match m {
                LlmMessage::System { content }
                | LlmMessage::User { content }
                | LlmMessage::Assistant { content, .. }
                | LlmMessage::Tool { content, .. } => counter.count(content),
            })
            .sum();
        assert_eq!(result.estimated_tokens(), manual as u32);
    }

    #[test]
    fn assemble_prompt_includes_tools() {
        // Given a state with tool definitions.
        let (state, session_id) = state_with_history(vec![ChatEntry::user("use tools")]);
        {
            let mut guard = state.write();
            guard.context.tool_definitions.insert(
                "bash".to_owned(),
                make_tool("bash"),
            );
        }

        // When assembling the prompt.
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter());

        // Then tool definitions are included.
        assert_eq!(result.tool_definitions.len(), 1);
        assert_eq!(result.tool_definitions[0].name, "bash");
    }

    #[test]
    fn assemble_prompt_includes_context_files_in_system_message() {
        // Given a state with cached context files.
        let (state, session_id) = state_with_history(vec![]);
        {
            let mut guard = state.write();
            guard.context.context_files = vec![ContextFile {
                path: std::path::PathBuf::from("/project/AGENTS.md"),
                content: "Use Rust.".to_owned(),
            }];
        }

        // When assembling the prompt.
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter());

        // Then the system message contains the context file content.
        assert!(!result.messages.is_empty());
        match &result.messages[0] {
            LlmMessage::System { content } => {
                assert!(content.contains("Use Rust."));
                assert!(content.contains("Project Context"));
            }
            other => panic!("expected System message, got {other:?}"),
        }
    }
}
