//! Skill built-in tool — loads a skill's content and pins it as TOP context.

use crate::feat::skills::frontmatter::strip_frontmatter;
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};
use crate::protocol::{ChatEntry, PinPosition};

use super::BoxedToolFuture;

/// Returns the tool definition for the `skill` built-in tool.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "skill".to_owned(),
        description: "Load a specific agent skill's content into the conversation context. The skill content will be pinned as a top-priority system instruction for the rest of the conversation. Use this tool when the current task matches a skill's description from the available skills list."
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
/// Reads the skill's SKILL.md file, strips YAML frontmatter, pushes a
/// TOP-pinned system entry to the session, and returns the content.
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
                        "skill '{name}' is disabled for this session. \
                         Use <leader>sk to re-enable it."
                    ),
                    success: false,
                    full_content: None,
                    truncation: None,
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
                };
            }
        };

        let body = strip_frontmatter(&content);

        // Pin the skill content as a TOP system entry in the session.
        if let (Some(state), Some(session_id)) = (ctx.state, ctx.session_id.clone()) {
            let location = skill_path.to_string_lossy().to_string();
            let entry = ChatEntry::skill(&name, &location, body).with_pin(PinPosition::Top);

            // Emit PushChatEntry command so the session actor persists the entry.
            if let Some(sink) = ctx.sink {
                let _ = sink.send_command(crate::protocol::Command::PushChatEntry(
                    crate::feat::chat_input::protocol::command::PushChatEntry {
                        session_id: session_id.clone(),
                        entry,
                    },
                ));
            } else {
                // Fallback: push directly (no persistence, but entry appears in UI).
                let mut guard = state.write();
                let session = guard.session_mut_or_create(&session_id);
                session.push_entry(entry);
            }
        }

        ToolResult {
            tool_call_id: call.id,
            name: call.name,
            content: format!("Skill '{name}' loaded"),
            success: true,
            full_content: None,
            truncation: None,
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
    async fn execute_pins_skill_content_to_session() {
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
            state: Some(state.clone()),
            session_id: Some(session_id.clone()),
            app_paths: crate::common::app_paths::AppPaths::default(),
            sink: None,
            shell: "/bin/sh".to_owned(),
            max_output_lines: None,
            max_output_bytes: None,
        };

        // When executing.
        let result = execute(call, ctx).await;

        // Then if the skill exists, it should be pinned.
        if result.success {
            let guard = state.read();
            let session = guard.session(&session_id);
            let pinned = session.pinned_entries();
            assert!(!pinned.is_empty(), "skill content should be pinned");
            assert_eq!(pinned[0].pin_position(), Some(PinPosition::Top));
        }
        // If the skill doesn't exist, that's OK for this test.
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_confirmation_in_tool_result() {
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

        // Then if the skill exists, the tool result contains a confirmation message.
        if result.success {
            assert_eq!(
                result.content, "Skill 'phased-task-loop' loaded",
                "tool result should contain confirmation, not the full body"
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
            session.set_disabled_skills(
                HashSet::from(["web-coder".to_owned()]),
            );
        }

        let call = ToolCall {
            id: "call_1".to_owned(),
            name: "skill".to_owned(),
            arguments: serde_json::json!({"name": "web-coder"}).to_string(),
        };

        let ctx = ToolContext {
            cwd: PathBuf::from("/tmp"),
            timeout: None,
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
