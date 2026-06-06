//! Prompt assembly - builds LLM-ready messages from session state.
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
use jinn_provider::ServerToolType;

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

/// Fully assembled LLM prompt - everything a provider needs to make a request.
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
#[allow(clippy::too_many_lines)]
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

    // Filter out disabled tools from the default tool list.
    // Override tool_definitions are not filtered (user-provided takes priority).
    let disabled = session.disabled_tools();
    if overrides.is_none_or(|o| o.tool_definitions.is_none()) {
        tool_defs.retain(|def| !disabled.contains(&def.name));
    }

    // Filter out server tools that don't match the active provider.
    // The model string format is "provider_name/model_name" - extract provider.
    let provider_name = session.model().split('/').next().unwrap_or("");
    if overrides.is_none_or(|o| o.tool_definitions.is_none()) {
        tool_defs.retain(|def| {
            match def.server_tool_type {
                Some(ServerToolType::OpenrouterWebSearch) => provider_name == "openrouter",
                _ => true, // Regular function tools always pass through.
            }
        });
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
        // Filter disabled tools from the tool context block.
        let filtered_map: HashMap<String, ToolDefinition> = state
            .context
            .tool_definitions
            .iter()
            .filter(|(name, _)| !disabled.contains(*name))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        build_tool_context_block(&filtered_map)
    };

    // Apply overrides: skills block.
    let skills_block = if overrides.is_some_and(|o| o.skip_skills) {
        String::new()
    } else {
        let disabled_skills = session.disabled_skills();
        let filtered: Vec<_> = state
            .context
            .skills
            .iter()
            .filter(|s| !disabled_skills.contains(&s.name))
            .cloned()
            .collect();
        format_skills_for_prompt(&filtered)
    };

    // Apply overrides: env context.
    let env_context = if overrides.is_some_and(|o| o.system_prompt.is_some()) {
        // System prompt override replaces everything - skip env context.
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
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
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
            body: String::new(),
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
            server_tool_type: None,
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
    fn assemble_prompt_drained_steering_sits_at_tail_preserving_tool_pairing() {
        // Given a tool_call/tool_result pair followed by a drained steering entry.
        let assistant = ChatEntry::assistant("using tool");
        let tool_result = ChatEntry::tool_result(
            "call-1",
            "bash",
            "ok",
            crate::feat::session::tool_result_status::ToolResultStatus::Success,
        );
        let steer = ChatEntry::user_expanded("stay at the foo part", "stay at the foo part");
        let (state, session_id) = state_with_history(vec![
            ChatEntry::user("initial"),
            assistant,
            tool_result,
            steer,
        ]);

        // When assembling.
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter(), None);

        // Then the steering entry sits at the tail of the messages.
        let last = result.messages.last().expect("has last message");
        match last {
            LlmMessage::User { content } => {
                assert_eq!(content, "stay at the foo part");
            }
            other => panic!("expected User (steering) at tail, got {other:?}"),
        }
    }

    #[test]
    fn assemble_prompt_steering_and_bottom_pin_coexist_at_respective_positions() {
        // Given a user-pinned entry and a tail steering entry.
        use crate::feat::session::chat_entry::PinPosition;
        let pinned = ChatEntry::user("pinned constraint").with_pin(PinPosition::Bottom);
        let middle = ChatEntry::user("middle");
        let assistant = ChatEntry::assistant("response");
        let steer = ChatEntry::user_expanded("steer msg", "steer msg");
        let entries = vec![pinned, middle, assistant, steer];
        let (state, session_id) = state_with_history(entries);

        // When assembling.
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter(), None);

        // Then both the pinned message and the steering message appear in the assembled prompt.
        let body = result
            .messages
            .iter()
            .map(|m| match m {
                LlmMessage::User { content } => content.as_str(),
                _ => "",
            })
            .collect::<Vec<_>>();
        assert!(
            body.iter().any(|s| s.contains("pinned constraint")),
            "pinned message must appear in prompt: {body:?}"
        );
        assert!(
            body.iter().any(|s| s.contains("steer msg")),
            "steering message must appear in prompt: {body:?}"
        );
        // Steering entry remains at the tail.
        assert_eq!(body.last().copied(), Some("steer msg"));
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

        // Then the system message is exactly the override - no skills, no context files.
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
                server_tool_type: None,
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
    fn assemble_prompt_excludes_disabled_tools_from_tool_definitions() {
        // Given a session with tools and some disabled.
        let (state, session_id) = state_with_history(vec![ChatEntry::user("use tools")]);
        {
            let mut guard = state.write();
            guard
                .context
                .tool_definitions
                .insert("bash".to_owned(), make_tool("bash"));
            guard
                .context
                .tool_definitions
                .insert("read".to_owned(), make_tool("read"));
            guard
                .context
                .tool_definitions
                .insert("write".to_owned(), make_tool("write"));
            // Disable bash and write.
            let mut disabled = std::collections::HashSet::new();
            disabled.insert("bash".to_owned());
            disabled.insert("write".to_owned());
            guard
                .session
                .get_mut(&session_id)
                .expect("session exists")
                .set_disabled_tools(disabled);
        }

        // When assembling the prompt.
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter(), None);

        // Then only enabled tools appear in tool definitions.
        let tool_names: Vec<&str> = result
            .tool_definitions
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert!(
            !tool_names.contains(&"bash"),
            "disabled bash should be excluded, got: {tool_names:?}"
        );
        assert!(
            !tool_names.contains(&"write"),
            "disabled write should be excluded, got: {tool_names:?}"
        );
        assert!(
            tool_names.contains(&"read"),
            "enabled read should be included, got: {tool_names:?}"
        );
    }

    #[test]
    fn assemble_prompt_excludes_disabled_tools_from_tool_context_block() {
        // Given a session with tools and some disabled.
        let (state, session_id) = state_with_history(vec![ChatEntry::user("use tools")]);
        {
            let mut guard = state.write();
            guard
                .context
                .tool_definitions
                .insert("bash".to_owned(), make_tool("bash"));
            guard
                .context
                .tool_definitions
                .insert("read".to_owned(), make_tool("read"));
            // Disable bash.
            let mut disabled = std::collections::HashSet::new();
            disabled.insert("bash".to_owned());
            guard
                .session
                .get_mut(&session_id)
                .expect("session exists")
                .set_disabled_tools(disabled);
        }

        // When assembling the prompt.
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter(), None);

        // Then the system message tool block excludes disabled tools.
        match &result.messages[0] {
            LlmMessage::System { content } => {
                assert!(
                    content.contains("read does things"),
                    "enabled tool should be in tool context block, got: {content}"
                );
                assert!(
                    !content.contains("bash does things"),
                    "disabled tool should be excluded from tool context block, got: {content}"
                );
            }
            other => panic!("expected System message, got {other:?}"),
        }
    }

    #[test]
    fn assemble_prompt_excludes_disabled_skills_from_skills_block() {
        // Given a session with skills and some disabled.
        let (state, session_id) = state_with_history(vec![ChatEntry::user("use skills")]);
        {
            let mut guard = state.write();
            guard.context.skills = vec![
                make_skill("phased-task-loop"),
                make_skill("web-coder"),
                make_skill("scream"),
            ];
            // Disable web-coder.
            guard
                .session
                .get_mut(&session_id)
                .expect("session exists")
                .set_disabled_skills(std::collections::HashSet::from(["web-coder".to_owned()]));
        }

        // When assembling the prompt.
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter(), None);

        // Then the system message skills block excludes disabled skills.
        match &result.messages[0] {
            LlmMessage::System { content } => {
                assert!(
                    content.contains("<name>phased-task-loop</name>"),
                    "enabled skill should be in skills block, got: {content}"
                );
                assert!(
                    content.contains("<name>scream</name>"),
                    "enabled skill should be in skills block, got: {content}"
                );
                assert!(
                    !content.contains("<name>web-coder</name>"),
                    "disabled skill should be excluded from skills block, got: {content}"
                );
            }
            other => panic!("expected System message, got {other:?}"),
        }
    }

    #[test]
    fn assemble_prompt_override_tool_definitions_not_filtered_by_disabled() {
        // Given a session with disabled tools AND an override providing specific tools.
        let (state, session_id) = state_with_history(vec![ChatEntry::user("use tools")]);
        {
            let mut guard = state.write();
            guard
                .context
                .tool_definitions
                .insert("bash".to_owned(), make_tool("bash"));
            // Disable bash.
            let mut disabled = std::collections::HashSet::new();
            disabled.insert("bash".to_owned());
            guard
                .session
                .get_mut(&session_id)
                .expect("session exists")
                .set_disabled_tools(disabled);
        }

        // When assembling with tool_definitions override that includes bash.
        let overrides = AssemblyOverrides {
            tool_definitions: Some(vec![ToolDefinition {
                name: "bash".to_owned(),
                description: "A workflow tool".to_owned(),
                parameters: serde_json::json!({"type": "object"}),
                prompt_snippet: Some("workflow bash".to_owned()),
                prompt_guidelines: vec![],
                server_tool_type: None,
            }]),
            ..Default::default()
        };
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter(), Some(&overrides));

        // Then the override tools are used as-is (not filtered by disabled set).
        assert_eq!(result.tool_definitions.len(), 1);
        assert_eq!(result.tool_definitions[0].name, "bash");
    }

    #[test]
    fn assemble_prompt_uses_matching_persona() {
        // Given a session with persona "custom" and a persona list containing "custom".
        let (state, session_id) = state_with_history(vec![ChatEntry::user("hello")]);
        {
            let mut guard = state.write();
            guard.context.personas.push(crate::feat::persona::Persona {
                name: "custom".to_owned(),
                description: "Custom persona".to_owned(),
                body: "You are a custom persona.".to_owned(),
                file_path: std::path::PathBuf::from("/custom.md"),
            });
            guard
                .session
                .get_mut(&session_id)
                .expect("session exists")
                .set_persona_name("custom".to_owned());
        }

        // When assembling the prompt.
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter(), None);

        // Then the system message contains the custom persona body.
        match &result.messages[0] {
            LlmMessage::System { content } => {
                assert!(
                    content.contains("You are a custom persona."),
                    "should contain custom persona body, got: {content}"
                );
            }
            other => panic!("expected System message, got {other:?}"),
        }
    }

    #[test]
    fn assemble_prompt_falls_back_to_coding_assistant_persona() {
        // Given a session with persona name "nonexistent" but "coding-assistant" is available.
        let (state, session_id) = state_with_history(vec![ChatEntry::user("hello")]);
        {
            let mut guard = state.write();
            guard.context.personas.push(crate::feat::persona::Persona {
                name: "coding-assistant".to_owned(),
                description: "Default".to_owned(),
                body: "You are a coding assistant.".to_owned(),
                file_path: std::path::PathBuf::from("/coding.md"),
            });
            guard
                .session
                .get_mut(&session_id)
                .expect("session exists")
                .set_persona_name("nonexistent".to_owned());
        }

        // When assembling the prompt.
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter(), None);

        // Then the system message contains the coding-assistant fallback.
        match &result.messages[0] {
            LlmMessage::System { content } => {
                assert!(
                    content.contains("You are a coding assistant."),
                    "should contain coding-assistant fallback, got: {content}"
                );
            }
            other => panic!("expected System message, got {other:?}"),
        }
    }

    #[test]
    fn assemble_prompt_includes_pinned_system_entry_in_system_message() {
        // Given a session with a top-pinned System entry.
        let mut sys_entry = ChatEntry::system("Custom system instructions");
        sys_entry.pin_position = Some(PinPosition::Top);
        let user = ChatEntry::user("hello");
        let (state, session_id) = state_with_history(vec![sys_entry, user]);

        // When assembling the prompt.
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter(), None);

        // Then the system message contains the pinned system content.
        match &result.messages[0] {
            LlmMessage::System { content } => {
                assert!(
                    content.contains("Custom system instructions"),
                    "should contain pinned system entry, got: {content}"
                );
            }
            other => panic!("expected System message, got {other:?}"),
        }
    }

    #[test]
    fn assemble_prompt_excludes_pinned_system_from_non_system_messages() {
        // Given a session with a top-pinned System entry and a top-pinned User entry.
        let mut sys_entry = ChatEntry::system("System stuff");
        sys_entry.pin_position = Some(PinPosition::Top);
        let mut user_pin = ChatEntry::user("pinned user");
        user_pin.pin_position = Some(PinPosition::Top);
        let (state, session_id) = state_with_history(vec![sys_entry, user_pin]);

        // When assembling the prompt.
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter(), None);

        // Then the system message contains the system entry content.
        let system_msg = result
            .messages
            .iter()
            .find(|m| matches!(m, LlmMessage::System { .. }));
        assert!(system_msg.is_some(), "should have a system message");
        if let LlmMessage::System { content } = system_msg.expect("checked") {
            assert!(content.contains("System stuff"));
        }

        // And the pinned user message appears as a separate User message (not merged into system).
        let user_msgs: Vec<_> = result
            .messages
            .iter()
            .filter(|m| matches!(m, LlmMessage::User { .. }))
            .collect();
        assert!(
            user_msgs.iter().any(|m| {
                if let LlmMessage::User { content } = m {
                    content == "pinned user"
                } else {
                    false
                }
            }),
            "pinned user message should appear as User: {user_msgs:?}"
        );
    }

    #[test]
    fn assemble_prompt_bottom_pins_placed_before_last_message() {
        // Given a session with multiple bottom-pinned entries and a user message.
        let mut bottom1 = ChatEntry::assistant("bottom-1");
        bottom1.pin_position = Some(PinPosition::Bottom);
        let mut bottom2 = ChatEntry::assistant("bottom-2");
        bottom2.pin_position = Some(PinPosition::Bottom);
        let user = ChatEntry::user("the last msg");
        let (state, session_id) = state_with_history(vec![bottom1, bottom2, user]);

        // When assembling the prompt.
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter(), None);

        // Then the last message is the user message (bottom pins are inserted before it).
        let last = result.messages.last().expect("has messages");
        if let LlmMessage::User { content } = last {
            assert_eq!(content, "the last msg");
        } else {
            panic!("expected User as last message, got {last:?}");
        }

        // And both bottom pins appear before the last message.
        let n = result.messages.len();
        let assistant_contents: Vec<&str> = result.messages[..n - 1]
            .iter()
            .filter_map(|m| match m {
                LlmMessage::Assistant { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            assistant_contents.contains(&"bottom-1"),
            "bottom-1 should appear before last msg"
        );
        assert!(
            assistant_contents.contains(&"bottom-2"),
            "bottom-2 should appear before last msg"
        );
    }

    #[test]
    fn assemble_prompt_top_pin_not_in_working_history() {
        // Given a session with a top-pinned entry that is NOT a system entry.
        let mut top_user = ChatEntry::user("top pinned user");
        top_user.pin_position = Some(PinPosition::Top);
        let working_user = ChatEntry::user("working user");
        let (state, session_id) = state_with_history(vec![top_user, working_user]);

        // When assembling the prompt.
        let guard = state.read();
        let result = assemble_prompt(&guard, &session_id, &counter(), None);

        // Then the top-pinned user message appears exactly once (not duplicated in working history).
        let user_msg_count = result
            .messages
            .iter()
            .filter(|m| {
                if let LlmMessage::User { content } = m {
                    content == "top pinned user"
                } else {
                    false
                }
            })
            .count();
        assert_eq!(
            user_msg_count, 1,
            "top pinned user should appear exactly once, appeared {user_msg_count}"
        );
    }
}
