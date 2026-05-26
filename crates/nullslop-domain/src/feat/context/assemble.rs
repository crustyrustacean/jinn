//! Prompt assembly — builds LLM-ready messages from session state.
//!
//! Produces [`AssembledPrompt`] via a single pure function call.
//! The assembly pipeline reads all context (skills, persona, context files,
//! tools, history) from [`AppState`] in one pass, splits pinned entries,
//! builds the system prompt, converts history to messages, and counts tokens.

use std::collections::HashMap;

use crate::common::app_state::AppState;
use crate::feat::context::env_context::build_env_context;
use crate::feat::context::strategy::token_estimator::TokenCounter;
use crate::feat::context::tool_prompt::build_tool_context_block;
use crate::feat::skills::format::format_skills_for_prompt;
use crate::protocol::{
    ChatEntry, LlmMessage, PinPosition, SessionId, ToolDefinition, entries_to_messages,
};

/// Overrides for [`assemble_prompt`]. When provided, these replace the default
/// sources for system prompt, tools, skills, and context files.
///
/// Used by workflow sessions to control the LLM prompt independently of global state.
#[derive(Debug, Clone, Default)]
pub struct AssemblyOverrides {
    /// If set, replaces the entire system prompt (persona, skills, env context, context files).
    /// Pinned system entries from history are still included.
    pub system_prompt: Option<String>,
    /// If set, replaces global tool definitions in both the assembled prompt's
    /// `tool_definitions` field and the tool context block.
    pub tool_definitions: Option<Vec<ToolDefinition>>,
    /// If true, skip the skills block in the system prompt.
    pub skip_skills: bool,
    /// If true, skip context files in the env context.
    pub skip_context_files: bool,
}

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
    overrides: Option<&AssemblyOverrides>,
) -> AssembledPrompt {
    let session = state.session(session_id);
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

    // Apply overrides: tool definitions.
    let mut tool_defs: Vec<ToolDefinition> = overrides
        .and_then(|o| o.tool_definitions.clone())
        .unwrap_or_else(|| state.context.tool_definitions.values().cloned().collect());

    // Inject judge-specific tools for judge sessions.
    if session.judge().is_some() {
        tool_defs.extend(crate::feat::judge::judge_tool_definitions());
    }

    // Apply overrides: tool context block.
    let tool_block = if overrides.is_some_and(|o| o.tool_definitions.is_some()) {
        let defs = overrides
            .expect("checked")
            .tool_definitions
            .as_ref()
            .expect("checked");
        let map: HashMap<String, ToolDefinition> =
            defs.iter().map(|d| (d.name.clone(), d.clone())).collect();
        build_tool_context_block(&map)
    } else {
        build_tool_context_block(&state.context.tool_definitions)
    };

    // Apply overrides: skills block.
    let skills_block = if overrides.is_some_and(|o| o.skip_skills) {
        String::new()
    } else {
        format_skills_for_prompt(&state.context.skills)
    };

    // Apply overrides: env context.
    let env_context = if overrides.is_some_and(|o| o.system_prompt.is_some()) {
        // System prompt override replaces everything — skip env context.
        String::new()
    } else if overrides.is_some_and(|o| o.skip_context_files) {
        build_env_context(persona, &[], &cwd)
    } else {
        build_env_context(persona, context_files, &cwd)
    };

    // Check for system prompt override.
    let forced_system = overrides.and_then(|o| o.system_prompt.clone());

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

    // Assemble system parts.
    let full_system = if let Some(content) = forced_system {
        // Override replaces all generated system parts.
        // Still include pinned system entries from history below.
        Some(content)
    } else {
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
        if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n\n"))
        }
    };

    // Convert working history to messages.
    let messages = entries_to_messages(&working_history);

    // Build final message list.
    let mut final_messages = Vec::new();

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
/// Filters out entries not in context (per `is_in_context()`).
/// Pinned entries are always in context regardless of kind.
fn split_history(history: &[ChatEntry]) -> (Vec<ChatEntry>, Vec<ChatEntry>, Vec<ChatEntry>) {
    let top_pins: Vec<ChatEntry> = history
        .iter()
        .filter(|e| e.pin_position() == Some(PinPosition::Top))
        .cloned()
        .collect();

    let bottom_pins: Vec<ChatEntry> = history
        .iter()
        .filter(|e| e.pin_position() == Some(PinPosition::Bottom))
        .cloned()
        .collect();

    let working_history: Vec<ChatEntry> = history
        .iter()
        .filter(|e| {
            (e.pin_position().is_none() || e.pin_position() == Some(PinPosition::Relative))
                && e.is_in_context()
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
        let result = assemble_prompt(&guard, &session_id, &counter(), None);

        // Then the first message is System and contains the skill.
        assert!(
            !result.messages.is_empty(),
            "should have at least a system message"
        );
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
        let result = assemble_prompt(&guard, &session_id, &counter(), None);

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
        let result = assemble_prompt(&guard, &session_id, &counter(), None);

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
        let result = assemble_prompt(&guard, &session_id, &counter(), None);

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
        let result = assemble_prompt(&guard, &session_id, &counter, None);

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
            guard
                .context
                .tool_definitions
                .insert("bash".to_owned(), make_tool("bash"));
        }

        // When assembling the prompt.
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter(), None);

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
        let result = assemble_prompt(&guard, &session_id, &counter(), None);

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

    #[test]
    fn assemble_prompt_with_system_prompt_override_replaces_system_message() {
        // Given a state with skills and context files.
        let (state, session_id) = state_with_history(vec![ChatEntry::user("hello")]);
        {
            let mut guard = state.write();
            guard.context.skills = vec![make_skill("test-skill")];
            guard.context.context_files = vec![ContextFile {
                path: std::path::PathBuf::from("/project/AGENTS.md"),
                content: "Use Rust.".to_owned(),
            }];
        }

        // When assembling with system_prompt override.
        let overrides = AssemblyOverrides {
            system_prompt: Some("You are a workflow assistant.".to_owned()),
            ..Default::default()
        };
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter(), Some(&overrides));

        // Then the system message is exactly the override — no skills, no context files.
        match &result.messages[0] {
            LlmMessage::System { content } => {
                assert_eq!(content, "You are a workflow assistant.");
                assert!(!content.contains("test-skill"));
                assert!(!content.contains("Use Rust."));
            }
            other => panic!("expected System message, got {other:?}"),
        }
    }

    #[test]
    fn assemble_prompt_with_tool_definitions_override_replaces_tools() {
        // Given a state with global tools.
        let (state, session_id) = state_with_history(vec![ChatEntry::user("use tools")]);
        {
            let mut guard = state.write();
            guard
                .context
                .tool_definitions
                .insert("global_tool".to_owned(), make_tool("global_tool"));
        }

        // When assembling with tool_definitions override.
        let overrides = AssemblyOverrides {
            tool_definitions: Some(vec![ToolDefinition {
                name: "workflow_tool".to_owned(),
                description: "A workflow tool".to_owned(),
                parameters: serde_json::json!({"type": "object"}),
                prompt_snippet: Some("workflow tool does things".to_owned()),
                prompt_guidelines: vec![],
            }]),
            ..Default::default()
        };
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter(), Some(&overrides));

        // Then tool definitions are the override ones, not global.
        assert_eq!(result.tool_definitions.len(), 1);
        assert_eq!(result.tool_definitions[0].name, "workflow_tool");
    }

    #[test]
    fn assemble_prompt_with_skip_skills_excludes_skills_block() {
        // Given a state with skills.
        let (state, session_id) = state_with_history(vec![ChatEntry::user("hello")]);
        {
            let mut guard = state.write();
            guard.context.skills = vec![make_skill("test-skill")];
        }

        // When assembling with skip_skills.
        let overrides = AssemblyOverrides {
            skip_skills: true,
            ..Default::default()
        };
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter(), Some(&overrides));

        // Then the system message does NOT contain the skill.
        match &result.messages[0] {
            LlmMessage::System { content } => {
                assert!(!content.contains("test-skill"));
            }
            other => panic!("expected System message, got {other:?}"),
        }
    }

    #[test]
    fn assemble_prompt_with_skip_context_files_excludes_files() {
        // Given a state with context files.
        let (state, session_id) = state_with_history(vec![ChatEntry::user("hello")]);
        {
            let mut guard = state.write();
            guard.context.context_files = vec![ContextFile {
                path: std::path::PathBuf::from("/project/AGENTS.md"),
                content: "Use Rust.".to_owned(),
            }];
        }

        // When assembling with skip_context_files.
        let overrides = AssemblyOverrides {
            skip_context_files: true,
            ..Default::default()
        };
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter(), Some(&overrides));

        // Then the system message does NOT contain the context file content.
        match &result.messages[0] {
            LlmMessage::System { content } => {
                assert!(!content.contains("Use Rust."));
                assert!(!content.contains("Project Context"));
                // But still has env context (date, CWD).
                assert!(content.contains("Current date:"));
            }
            other => panic!("expected System message, got {other:?}"),
        }
    }

    #[test]
    fn assemble_prompt_with_none_overrides_is_identical_to_no_overrides() {
        // Given a state with skills and tools.
        let (state, session_id) = state_with_history(vec![ChatEntry::user("hello")]);
        {
            let mut guard = state.write();
            guard.context.skills = vec![make_skill("test-skill")];
            guard
                .context
                .tool_definitions
                .insert("bash".to_owned(), make_tool("bash"));
        }

        // When assembling with None overrides.
        let guard = state.read();
        let result_none = assemble_prompt(&guard, &session_id, &counter(), None);

        // Then the result matches what we'd get from the original behavior.
        assert!(!result_none.messages.is_empty());
        match &result_none.messages[0] {
            LlmMessage::System { content } => {
                assert!(content.contains("test-skill"));
            }
            other => panic!("expected System message, got {other:?}"),
        }
        assert_eq!(result_none.tool_definitions.len(), 1);
        assert_eq!(result_none.tool_definitions[0].name, "bash");
    }

    #[test]
    fn assemble_prompt_includes_judge_tools_for_judge_session() {
        // Given a judge session (session with judge metadata).
        use crate::feat::judge::JudgeMeta;
        let (state, session_id) = state_with_history(vec![ChatEntry::user("evaluate")]);
        {
            let mut guard = state.write();
            // Create a new session as the judge.
            let mut judge_session = crate::feat::session::chat_session::ChatSessionState::new();
            let _judge_id = judge_session.session_id().clone();
            judge_session.set_judge(JudgeMeta {
                origin_session: session_id,
                is_attached: true,
                judge_name: "test-judge".to_owned(),
            });
            guard.session.insert(judge_session);
        }

        // Get the judge session ID.
        let judge_id = {
            let guard = state.read();
            guard
                .session
                .iter()
                .find(|(_, s)| s.is_judge())
                .map(|(id, _)| id.clone())
                .expect("judge session exists")
        };

        // When assembling the prompt for the judge session.
        let guard = state.read();
        let result = assemble_prompt(&guard, &judge_id, &counter(), None);

        // Then the tool definitions include the judge tools.
        let tool_names: Vec<&str> = result
            .tool_definitions
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert!(
            tool_names.contains(&"session_query"),
            "judge tools should include session_query, got: {tool_names:?}"
        );
        assert!(
            tool_names.contains(&"session_query_recent"),
            "judge tools should include session_query_recent, got: {tool_names:?}"
        );
        assert!(
            tool_names.contains(&"task_complete"),
            "judge tools should include task_complete, got: {tool_names:?}"
        );
        assert!(
            tool_names.contains(&"task_incomplete"),
            "judge tools should include task_incomplete, got: {tool_names:?}"
        );
    }

    #[test]
    fn assemble_prompt_judge_session_includes_regular_and_judge_tools() {
        use crate::feat::judge::JudgeMeta;

        // Given a state with regular tools in the global map (simulating post-filter state).
        let (state, origin_id) = state_with_history(vec![ChatEntry::user("evaluate")]);
        {
            let mut guard = state.write();
            guard.context.tool_definitions.insert(
                "bash".to_owned(),
                make_tool("bash"),
            );
            guard.context.tool_definitions.insert(
                "read".to_owned(),
                make_tool("read"),
            );
        }

        // And a judge session.
        let judge_id = {
            let mut guard = state.write();
            let mut judge_session = crate::feat::session::chat_session::ChatSessionState::new();
            judge_session.set_judge(JudgeMeta {
                origin_session: origin_id,
                is_attached: true,
                judge_name: "test-judge".to_owned(),
            });
            let id = judge_session.session_id().clone();
            guard.session.insert(judge_session);
            id
        };

        // When assembling the prompt for the judge session.
        let guard = state.read();
        let result = assemble_prompt(&guard, &judge_id, &counter(), None);

        // Then the tool definitions include regular tools.
        let tool_names: Vec<&str> = result
            .tool_definitions
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert!(
            tool_names.contains(&"bash"),
            "judge session should have bash, got: {tool_names:?}"
        );
        assert!(
            tool_names.contains(&"read"),
            "judge session should have read, got: {tool_names:?}"
        );

        // And the tool definitions include judge tools.
        assert!(
            tool_names.contains(&"session_query"),
            "judge session should have session_query, got: {tool_names:?}"
        );
        assert!(
            tool_names.contains(&"session_query_recent"),
            "judge session should have session_query_recent, got: {tool_names:?}"
        );
        assert!(
            tool_names.contains(&"task_complete"),
            "judge session should have task_complete, got: {tool_names:?}"
        );
        assert!(
            tool_names.contains(&"task_incomplete"),
            "judge session should have task_incomplete, got: {tool_names:?}"
        );
    }

    #[test]
    fn assemble_prompt_excludes_judge_tools_for_regular_session() {
        // Given a regular (non-judge) session.
        let (state, session_id) = state_with_history(vec![ChatEntry::user("hello")]);

        // When assembling the prompt.
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter(), None);

        // Then the tool definitions do NOT include judge tools.
        let tool_names: Vec<&str> = result
            .tool_definitions
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert!(
            !tool_names.contains(&"session_query"),
            "regular sessions should not have session_query, got: {tool_names:?}"
        );
        assert!(
            !tool_names.contains(&"task_complete"),
            "regular sessions should not have task_complete, got: {tool_names:?}"
        );
    }
}
