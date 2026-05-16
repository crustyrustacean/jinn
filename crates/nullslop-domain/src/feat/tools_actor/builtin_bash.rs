//! Bash built-in tool — executes shell commands.
//!
//! Spawns a shell process, captures stdout + stderr, and returns the combined
//! output. Supports optional timeout and CWD resolution.

use std::fmt::Write as _;

use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};

use super::BoxedToolFuture;

/// Returns the tool definition for the `bash` built-in tool.
pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "bash".to_owned(),
        description: "Execute a bash command in the current working directory. \
            Returns stdout and stderr. Output is truncated to last 2000 lines or 50KB \
            (whichever is hit first). Optionally provide a timeout in seconds."
            .to_owned(),
        prompt_snippet: Some("Execute bash commands (ls, grep, find, etc.)".to_owned()),
        prompt_guidelines: vec![
            "Use bash for file operations like ls, rg, find".to_owned(),
        ],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Bash command to execute"
                },
                "timeout": {
                    "type": "number",
                    "description": "Timeout in seconds (optional, no default timeout)"
                }
            },
            "required": ["command"]
        }),
    }
}

/// Detects the user's shell from the SHELL env var, falling back to /bin/sh.
fn shell_path() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned())
}

/// Parses the arguments from the tool call JSON.
fn parse_args(raw: &str) -> Result<(String, Option<u64>), serde_json::Error> {
    let v: serde_json::Value = serde_json::from_str(raw)?;
    let command = v
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let timeout = v.get("timeout").and_then(serde_json::Value::as_u64);
    Ok((command, timeout))
}

/// Executes the `bash` built-in tool.
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let (command, per_call_timeout) = match parse_args(&call.arguments) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content: format!("failed to parse arguments: {e}"),
                    success: false,
                };
            }
        };

        if command.is_empty() {
            return ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: "command is empty".to_owned(),
                success: false,
            };
        }

        let shell = shell_path();
        let cwd = ctx.cwd.clone();

        let output = tokio::process::Command::new(&shell)
            .arg("-c")
            .arg(&command)
            .current_dir(&cwd)
            .output();

        // Use the per-call timeout if provided, otherwise fall back to context timeout.
        let timeout_dur = per_call_timeout
            .map(std::time::Duration::from_secs)
            .or(ctx.timeout);

        let result = match timeout_dur {
            Some(dur) => match tokio::time::timeout(dur, output).await {
                Ok(Ok(out)) => Ok(out),
                Ok(Err(e)) => Err(format!("failed to execute command: {e}")),
                Err(_) => Err(format!("command timed out after {}s", dur.as_secs())),
            },
            None => match output.await {
                Ok(out) => Ok(out),
                Err(e) => Err(format!("failed to execute command: {e}")),
            },
        };

        match result {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let mut content = String::new();
                if !stdout.is_empty() {
                    content.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    if !content.is_empty() && !content.ends_with('\n') {
                        content.push('\n');
                    }
                    content.push_str(&stderr);
                }

                let success = out.status.success();
                if !success {
                    if !content.is_empty() && !content.ends_with('\n') {
                        content.push('\n');
                    }
                    let _ = write!(
                        content,
                        "Command exited with code {}",
                        out.status
                            .code()
                            .map_or_else(|| "unknown (signal)".to_owned(), |c| c.to_string())
                    );
                }

                ToolResult {
                    tool_call_id: call.id,
                    name: call.name,
                    content,
                    success,
                }
            }
            Err(msg) => ToolResult {
                tool_call_id: call.id,
                name: call.name,
                content: msg,
                success: false,
            },
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_ctx() -> ToolContext {
        ToolContext {
            cwd: PathBuf::from("/tmp"),
            timeout: None,
            state: None,
            session_id: None,
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_stdout() {
        // Given a bash tool call that echoes text.
        let call = ToolCall {
            id: "call_1".to_owned(),
            name: "bash".to_owned(),
            arguments: serde_json::json!({
                "command": "echo hello world"
            })
            .to_string(),
        };

        // When executing the bash tool.
        let result = execute(call, test_ctx()).await;

        // Then the result contains the echoed output.
        assert_eq!(result.tool_call_id, "call_1");
        assert!(result.success);
        assert!(result.content.contains("hello world"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_captures_stderr() {
        // Given a bash tool call that writes to stderr.
        let call = ToolCall {
            id: "call_2".to_owned(),
            name: "bash".to_owned(),
            arguments: serde_json::json!({
                "command": "echo error >&2"
            })
            .to_string(),
        };

        // When executing the bash tool.
        let result = execute(call, test_ctx()).await;

        // Then the result contains the stderr output.
        assert!(result.success);
        assert!(result.content.contains("error"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_reports_nonzero_exit() {
        // Given a bash tool call that exits with code 1.
        let call = ToolCall {
            id: "call_3".to_owned(),
            name: "bash".to_owned(),
            arguments: serde_json::json!({
                "command": "exit 1"
            })
            .to_string(),
        };

        // When executing the bash tool.
        let result = execute(call, test_ctx()).await;

        // Then the result indicates failure with exit code.
        assert!(!result.success);
        assert!(result.content.contains("Command exited with code 1"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_resolves_cwd() {
        // Given a bash tool call that prints the working directory.
        let dir = tempfile::tempdir().expect("create temp dir");
        let ctx = ToolContext {
            cwd: dir.path().to_owned(),
            timeout: None,
            state: None,
            session_id: None,
        };

        let call = ToolCall {
            id: "call_4".to_owned(),
            name: "bash".to_owned(),
            arguments: serde_json::json!({
                "command": "pwd"
            })
            .to_string(),
        };

        // When executing the bash tool.
        let result = execute(call, ctx).await;

        // Then the output shows the CWD.
        assert!(result.success);
        assert!(result.content.contains(dir.path().to_str().unwrap()));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_error_on_empty_command() {
        // Given a bash tool call with an empty command.
        let call = ToolCall {
            id: "call_5".to_owned(),
            name: "bash".to_owned(),
            arguments: serde_json::json!({
                "command": ""
            })
            .to_string(),
        };

        // When executing the bash tool.
        let result = execute(call, test_ctx()).await;

        // Then the result indicates failure.
        assert!(!result.success);
        assert!(result.content.contains("command is empty"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_returns_error_on_bad_json() {
        // Given a bash tool call with invalid JSON.
        let call = ToolCall {
            id: "call_6".to_owned(),
            name: "bash".to_owned(),
            arguments: "not json".to_owned(),
        };

        // When executing the bash tool.
        let result = execute(call, test_ctx()).await;

        // Then the result indicates failure.
        assert!(!result.success);
        assert!(result.content.contains("failed to parse arguments"));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn execute_supports_per_call_timeout() {
        // Given a bash tool call with a 1-second timeout and a command that sleeps for 10 seconds.
        let call = ToolCall {
            id: "call_7".to_owned(),
            name: "bash".to_owned(),
            arguments: serde_json::json!({
                "command": "sleep 10",
                "timeout": 1
            })
            .to_string(),
        };

        // When executing the bash tool.
        let result = execute(call, test_ctx()).await;

        // Then the result indicates timeout.
        assert!(!result.success);
        assert!(result.content.contains("timed out"));
    }
}
