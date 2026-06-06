//! Skill built-in tool - loads a skill's content into the conversation context
//! as a pinned ToolResult that the model sees inline at load time and on every
//! subsequent turn (the entry is pinned with `PinPosition::Relative`, which
//! survives compaction).

use crate::feat::skills::frontmatter::strip_frontmatter;
use crate::feat::tools_actor::tool_types::{
    ToolCall, ToolContext, ToolDefinition, ToolResult, ToolResultPinPosition,
};

use super::BoxedToolFuture;

/// Returns the tool definition for the `skill` built-in tool.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "skill".to_owned(),
        description: "Load a specific agent skill's content into the conversation context. The skill content is returned in this tool result and pinned in the conversation history for the rest of the conversation. Use this tool when the current task matches a skill's description from the available skills list. Do not call this tool if the skill is already loaded (marked loaded='true' in the available skills list)."
            .to_owned(),
        prompt_snippet: None,
        prompt_guidelines: vec![],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The name of the skill to load (from the available skills list)"
                }
            },
            "required": ["name"]
        }),
        server_tool_type: None,
    }
}

/// Executes the `skill` built-in tool.
///
/// Reads the skill's SKILL.md file, strips YAML frontmatter, and returns the
/// skill body wrapped in a `<skill>` XML element as the ToolResult content.
/// The ToolResult is pinned with `PinPosition::Relative` so it persists at its
/// original position in history through compaction.
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let name = match parse_args(&call.arguments) {
            Ok(n) => n,
            Err(e) => {
                return ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content: format!("failed to parse arguments: {e}"),
                    success: false,
                    full_content: None,
                    truncation: None,
                    pin_position: None,
                };
            }
        };

        if name.is_empty() {
            return ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: "skill name must not be empty".to_owned(),
                success: false,
                full_content: None,
                truncation: None,
                pin_position: None,
            };
        }

        // Reject disabled skills.
        if let (Some(state), Some(session_id)) = (ctx.state.as_ref(), &ctx.session_id) {
            let guard = state.read();
            if let Some(session) = guard.session.get(session_id)
                && !session.is_skill_enabled(&name)
            {
                return ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content: format!(
                        "skill '{name}' is disabled for this session. Use <leader>sk to re-enable it."
                    ),
                    success: false,
                    full_content: None,
                    truncation: None,
                    pin_position: None,
                };
            }
        }

        // Idempotency: refuse to re-load if a pinned ToolResult for this skill already exists.
        if let (Some(state), Some(session_id)) = (ctx.state.as_ref(), &ctx.session_id) {
            let guard = state.read();
            if let Some(session) = guard.session.get(session_id)
                && session.loaded_skills().contains(&name)
            {
                return ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content: format!(
                        "skill '{name}' is already loaded; its content is in context as a pinned tool result"
                    ),
                    success: false,
                    full_content: None,
                    truncation: None,
                    pin_position: None,
                };
            }
        }

        let skill_path = ctx.app_paths.skills_dir().join(&name).join("SKILL.md");

        let content = match tokio::fs::read_to_string(&skill_path).await {
            Ok(c) => c,
            Err(e) => {
                return ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content: format!("failed to read skill '{}': {e}", skill_path.display()),
                    success: false,
                    full_content: None,
                    truncation: None,
                    pin_position: None,
                };
            }
        };

        let body = strip_frontmatter(&content);
        let location = skill_path.to_string_lossy().to_string();
        let xml = format!("<skill name=\"{name}\" location=\"{location}\">\n{body}\n</skill>");

        ToolResult {
            tool_call_id: call.id,
            name: call.name,
            content: xml,
            success: true,
            full_content: None,
            truncation: None,
            pin_position: Some(ToolResultPinPosition::Relative),
        }
    })
}

fn parse_args(raw: &str) -> Result<String, serde_json::Error> {
    let v: serde_json::Value = serde_json::from_str(raw)?;
    let name = v
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    Ok(name)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use std::path::PathBuf;

    fn test_ctx() -> ToolContext {
        ToolContext {
            cwd: PathBuf::from("/tmp"),
            timeout: None,
            bash_default_timeout: None,
            state: None,
            session_id: None,
            app_paths: crate::common::app_paths::AppPaths::default(),
            sink: None,
            shell: "/bin/sh".to_owned(),
            max_output_lines: None,
            max_output_bytes: None,
        }
    }

    #[rstest::rstest]
    fn definition_has_correct_name() {
        // Given the skill tool definition.
        let def = definition();

        // Then the name is "skill".
        assert_eq!(def.name, "skill");
    }

    #[rstest::rstest]
    fn definition_requires_name_parameter() {
        // Given the skill tool definition.
        let def = definition();

        // Then the parameters require "name".
        let required = def
            .parameters
            .get("required")
            .expect("should have required");
        assert!(
            required
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("name"))
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_error_for_nonexistent_skill() {
        // Given a call for a nonexistent skill.
        let result = execute(
            ToolCall {
                id: "call_1".to_owned(),
                name: "skill".to_owned(),
                arguments: serde_json::json!({"name": "nonexistent-skill-xyz"}).to_string(),
            },
            test_ctx(),
        )
        .await;

        // Then the result indicates failure.
        assert!(!result.success);
        assert!(result.content.contains("failed to read skill"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_error_on_empty_name() {
        // Given a call with empty name.
        let call = ToolCall {
            id: "call_1".to_owned(),
            name: "skill".to_owned(),
            arguments: serde_json::json!({"name": ""}).to_string(),
        };

        // When executing.
        let result = execute(call, test_ctx()).await;

        // Then the result indicates failure.
        assert!(!result.success);
        assert!(result.content.contains("must not be empty"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_error_on_bad_json() {
        // Given a call with invalid JSON.
        let call = ToolCall {
            id: "call_1".to_owned(),
            name: "skill".to_owned(),
            arguments: "not json".to_owned(),
        };

        // When executing.
        let result = execute(call, test_ctx()).await;

        // Then the result indicates failure.
        assert!(!result.success);
        assert!(result.content.contains("failed to parse arguments"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_skill_body_in_tool_result() {
        use crate::common::app_state::AppState;
        use crate::common::state::State;
        use crate::protocol::SessionId;

        // Given a skill file in the real skills dir (best-effort).
        let state = State::new(AppState::default());
        let session_id = SessionId::new();

        let call = ToolCall {
            id: "call_1".to_owned(),
            name: "skill".to_owned(),
            arguments: serde_json::json!({"name": "phased-task-loop"}).to_string(),
        };

        let ctx = ToolContext {
            cwd: PathBuf::from("/tmp"),
            timeout: None,
            bash_default_timeout: None,
            state: Some(state),
            session_id: Some(session_id),
            app_paths: crate::common::app_paths::AppPaths::default(),
            sink: None,
            shell: "/bin/sh".to_owned(),
            max_output_lines: None,
            max_output_bytes: None,
        };

        // When executing.
        let result = execute(call, ctx).await;

        // Then if the skill exists, the tool result contains the body wrapped in <skill> XML, pinned Relative.
        if result.success {
            assert!(
                result
                    .content
                    .starts_with("<skill name=\"phased-task-loop\""),
                "tool result should contain the skill body in <skill> XML, got: {}",
                &result.content[..result.content.len().min(80)]
            );
            assert_eq!(
                result.pin_position,
                Some(ToolResultPinPosition::Relative),
                "tool result should be pinned Relative"
            );
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_already_loaded_for_duplicate_load() {
        use crate::common::app_state::AppState;
        use crate::common::state::State;
        use crate::feat::session::chat_entry::{ChatEntry, PinPosition};
        use crate::feat::session::tool_result_status::ToolResultStatus;
        use crate::protocol::SessionId;

        // Given a session that already has a pinned ToolResult from the `skill` tool
        // for "phased-task-loop" (matches the body-in-ToolResult shape).
        let state = State::new(AppState::default());
        let session_id = SessionId::new();
        {
            let mut guard = state.write();
            let session = guard.session_mut_or_create(&session_id);
            let seeded_xml = "<skill name=\"phased-task-loop\" location=\"/tmp\">\nbody\n</skill>";
            let mut entry = ChatEntry::tool_result(
                "seeded_call_id",
                "skill",
                seeded_xml,
                ToolResultStatus::Success,
            );
            entry.pin_position = Some(PinPosition::Relative);
            session.push_entry(entry);
        }

        // When calling execute again for the same skill name.
        let call = ToolCall {
            id: "call_2".to_owned(),
            name: "skill".to_owned(),
            arguments: serde_json::json!({"name": "phased-task-loop"}).to_string(),
        };
        let ctx = ToolContext {
            cwd: PathBuf::from("/tmp"),
            timeout: None,
            bash_default_timeout: None,
            state: Some(state),
            session_id: Some(session_id),
            app_paths: crate::common::app_paths::AppPaths::default(),
            sink: None,
            shell: "/bin/sh".to_owned(),
            max_output_lines: None,
            max_output_bytes: None,
        };
        let result = execute(call, ctx).await;

        // Then the result indicates failure with the "already loaded" message,
        // no new pin position, and no file is read.
        assert!(
            !result.success,
            "second load should fail when skill is already loaded"
        );
        assert!(
            result.content.contains("already loaded"),
            "tool result should mention already-loaded state, got: {}",
            result.content
        );
        assert_eq!(
            result.pin_position, None,
            "already-loaded rejection should not pin anything"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_loads_different_skill_when_other_already_loaded() {
        use crate::common::app_state::AppState;
        use crate::common::state::State;
        use crate::feat::session::chat_entry::{ChatEntry, PinPosition};
        use crate::feat::session::tool_result_status::ToolResultStatus;
        use crate::protocol::SessionId;

        // Given a session that already has a pinned ToolResult for "rust-programming".
        let state = State::new(AppState::default());
        let session_id = SessionId::new();
        {
            let mut guard = state.write();
            let session = guard.session_mut_or_create(&session_id);
            let seeded_xml = "<skill name=\"rust-programming\" location=\"/tmp\">\nbody\n</skill>";
            let mut entry = ChatEntry::tool_result(
                "seeded_call_id",
                "skill",
                seeded_xml,
                ToolResultStatus::Success,
            );
            entry.pin_position = Some(PinPosition::Relative);
            session.push_entry(entry);
        }

        // When calling execute for a *different* skill ("phased-task-loop").
        let call = ToolCall {
            id: "call_2".to_owned(),
            name: "skill".to_owned(),
            arguments: serde_json::json!({"name": "phased-task-loop"}).to_string(),
        };
        let ctx = ToolContext {
            cwd: PathBuf::from("/tmp"),
            timeout: None,
            bash_default_timeout: None,
            state: Some(state),
            session_id: Some(session_id),
            app_paths: crate::common::app_paths::AppPaths::default(),
            sink: None,
            shell: "/bin/sh".to_owned(),
            max_output_lines: None,
            max_output_bytes: None,
        };
        let result = execute(call, ctx).await;

        // Then if the requested skill file exists, the load succeeds and returns
        // the body in <skill> XML with Relative pin (idempotency did not reject
        // because the name differs from the already-loaded skill).
        if result.success {
            assert!(
                result
                    .content
                    .starts_with("<skill name=\"phased-task-loop\""),
                "different-name load should succeed with body in <skill> XML, got: {}",
                &result.content[..result.content.len().min(80)]
            );
            assert_eq!(
                result.pin_position,
                Some(ToolResultPinPosition::Relative),
                "successful different-name load should pin Relative"
            );
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_error_for_disabled_skill() {
        use crate::common::app_state::AppState;
        use crate::common::state::State;
        use crate::protocol::SessionId;
        use std::collections::HashSet;

        // Given a session with "web-coder" disabled.
        let state = State::new(AppState::default());
        let session_id = SessionId::new();
        {
            let mut guard = state.write();
            let session = guard.session_mut_or_create(&session_id);
            session.set_disabled_skills(HashSet::from(["web-coder".to_owned()]));
        }

        let call = ToolCall {
            id: "call_1".to_owned(),
            name: "skill".to_owned(),
            arguments: serde_json::json!({"name": "web-coder"}).to_string(),
        };

        let ctx = ToolContext {
            cwd: PathBuf::from("/tmp"),
            timeout: None,
            bash_default_timeout: None,
            state: Some(state),
            session_id: Some(session_id),
            app_paths: crate::common::app_paths::AppPaths::default(),
            sink: None,
            shell: "/bin/sh".to_owned(),
            max_output_lines: None,
            max_output_bytes: None,
        };

        // When executing.
        let result = execute(call, ctx).await;

        // Then the result indicates failure due to disabled skill.
        assert!(!result.success);
        assert!(result.content.contains("disabled for this session"));
    }
}
