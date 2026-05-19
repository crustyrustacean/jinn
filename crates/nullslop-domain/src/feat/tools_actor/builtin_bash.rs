//! Bash built-in tool — executes shell commands.
//!
//! Spawns a shell process with piped stdout/stderr, streaming output
//! line-by-line via `ToolExecutionOutput` events. Falls back to a
//! non-streaming path when the message sink is unavailable.

use std::fmt::Write as _;
use std::process::Stdio;

use crate::common::actor::message_sink::MessageSink;
use crate::feat::tools_actor::protocol::event::{ToolExecutionOutput, ToolExecutionStarted};
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};
use crate::protocol::{Event, SessionId};

use super::truncation::{DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, format_size, truncate_tail};

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
        prompt_guidelines: vec!["Use bash for file operations like ls, rg, find".to_owned()],
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

/// Maximum bytes to buffer before truncating accumulated streaming output.
/// This is a streaming-only threshold to prevent unbounded memory growth.
/// The final truncation uses the shared limits from the truncation module.
const STREAM_BUFFER_MAX_BYTES: usize = DEFAULT_MAX_BYTES * 2;

/// Truncates accumulated streaming output to prevent unbounded memory growth.
/// Uses the shared tail-truncation logic.
fn truncate_streaming_output(content: &str) -> String {
    truncate_tail(content, DEFAULT_MAX_LINES, DEFAULT_MAX_BYTES).content
}

/// Emits a streaming tool event if both sink and session_id are available.
fn emit_stream_event(
    sink: Option<&std::sync::Arc<dyn MessageSink>>,
    session_id: Option<&SessionId>,
    event: Event,
) {
    if let (Some(sink), Some(_)) = (sink, session_id)
        && let Err(e) = sink.send_event(event)
    {
        tracing::warn!(err = ?e, "bash: failed to emit streaming event");
    }
}

/// Creates an error [`ToolResult`] with the given fields and `success: false`.
fn error_tool_result(tool_call_id: String, name: String, content: String) -> ToolResult {
    ToolResult {
        tool_call_id,
        name,
        content,
        success: false,
        full_content: None,
        truncation: None,
    }
}

/// Formats the final [`ToolResult`] from the process exit status and accumulated output.
///
/// Applies tail-truncation when output exceeds the configured limits.
/// Stores the full output in `full_content` when truncation occurs.
fn format_exit_result(
    exit_result: &Result<std::process::ExitStatus, std::io::Error>,
    accumulated: &str,
    tool_call_id: String,
    tool_name: String,
    max_lines: usize,
    max_bytes: usize,
) -> ToolResult {
    let mut content = accumulated.to_string();
    let success = match exit_result {
        Ok(status) => status.success(),
        Err(_) => false,
    };

    if !success {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        let _ = write!(
            content,
            "Command exited with code {}",
            exit_result
                .as_ref()
                .ok()
                .and_then(std::process::ExitStatus::code)
                .map_or_else(|| "unknown (signal)".to_owned(), |c: i32| c.to_string())
        );
    }

    // Apply tail-truncation to the final output.
    let truncation_result = truncate_tail(&content, max_lines, max_bytes);
    if truncation_result.truncated {
        if let Some(meta) = truncation_result.meta {
            let start_line = meta.total_lines.saturating_sub(meta.output_lines) + 1;
            let end_line = meta.total_lines;
            let notice = if meta.truncated_by == nullslop_provider::tool_types::TruncatedBy::Bytes {
                format!(
                    "\n\n[Showing lines {start_line}-{end_line} of {} ({} limit)]",
                    meta.total_lines,
                    format_size(max_bytes)
                )
            } else {
                format!(
                    "\n\n[Showing lines {start_line}-{end_line} of {}]",
                    meta.total_lines
                )
            };
            let mut output = truncation_result.content;
            output.push_str(&notice);
            ToolResult {
                tool_call_id,
                name: tool_name,
                content: output,
                success,
                full_content: Some(content),
                truncation: Some(meta),
            }
        } else {
            // truncated but no meta — return unformatted truncated content
            ToolResult {
                tool_call_id,
                name: tool_name,
                content: truncation_result.content,
                success,
                full_content: Some(content),
                truncation: None,
            }
        }
    } else {
        ToolResult {
            tool_call_id,
            name: tool_name,
            content,
            success,
            full_content: None,
            truncation: None,
        }
    }
}

/// Reads stdout/stderr concurrently, emitting streaming events, then waits for the child to exit.
///
/// Lines are appended to `accumulated` and streamed via [`emit_stream_event`].
/// Large outputs are truncated in-place when they exceed `2 * MAX_BYTES`.
async fn read_child_output_and_wait(
    child: &mut tokio::process::Child,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
    accumulated: &mut String,
    sink: Option<&std::sync::Arc<dyn MessageSink>>,
    session_id: Option<&SessionId>,
    tool_call_id: &str,
) -> Result<std::process::ExitStatus, std::io::Error> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    let stdout_reader = tokio::io::BufReader::new(stdout);
    let stderr_reader = tokio::io::BufReader::new(stderr);

    let stdout_handle = {
        let tx = tx.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut lines = stdout_reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if tx.send(line).is_err() {
                    break;
                }
            }
        })
    };

    let stderr_handle = {
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut lines = stderr_reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if tx_clone.send(line).is_err() {
                    break;
                }
            }
        })
    };

    // Drop the sender so the receiver knows when both are done.
    drop(tx);

    // Receive lines and emit events.
    while let Some(line) = rx.recv().await {
        accumulated.push_str(&line);
        accumulated.push('\n');

        emit_stream_event(
            sink,
            session_id,
            Event::ToolExecutionOutput(ToolExecutionOutput {
                session_id: session_id.cloned().unwrap_or_default(),
                tool_call_id: tool_call_id.to_owned(),
                output: format!("{line}\n"),
            }),
        );

        if accumulated.len() > STREAM_BUFFER_MAX_BYTES {
            let truncated = truncate_streaming_output(accumulated);
            *accumulated = truncated;
        }
    }

    // Wait for both readers to finish.
    let _ = stdout_handle.await;
    let _ = stderr_handle.await;

    // Wait for process exit.
    child.wait().await
}

/// Executes the `bash` built-in tool with streaming output.
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let (command, per_call_timeout) = match parse_args(&call.arguments) {
            Ok(v) => v,
            Err(e) => {
                return error_tool_result(
                    call.id,
                    call.name,
                    format!("failed to parse arguments: {e}"),
                );
            }
        };

        if command.is_empty() {
            return error_tool_result(call.id, call.name, "command is empty".to_owned());
        }

        let shell = shell_path();
        let cwd = ctx.cwd.clone();

        // Emit ToolExecutionStarted if we have a sink and session_id.
        emit_stream_event(
            ctx.sink.as_ref(),
            ctx.session_id.as_ref(),
            Event::ToolExecutionStarted(ToolExecutionStarted {
                session_id: ctx.session_id.clone().unwrap_or_default(),
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
            }),
        );

        let spawn_result = tokio::process::Command::new(&shell)
            .arg("-c")
            .arg(&command)
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();

        let mut child = match spawn_result {
            Ok(child) => child,
            Err(e) => {
                return error_tool_result(
                    call.id,
                    call.name,
                    format!("failed to execute command: {e}"),
                );
            }
        };

        let Some(stdout) = child.stdout.take() else {
            return error_tool_result(
                call.id,
                call.name,
                "failed to capture stdout: pipe was not set up".to_owned(),
            );
        };
        let Some(stderr) = child.stderr.take() else {
            return error_tool_result(
                call.id,
                call.name,
                "failed to capture stderr: pipe was not set up".to_owned(),
            );
        };

        // Extract truncation limits from context.
        let max_lines = ctx.max_output_lines.unwrap_or(DEFAULT_MAX_LINES);
        let max_bytes = ctx.max_output_bytes.unwrap_or(DEFAULT_MAX_BYTES);

        // Read stdout and stderr concurrently using tokio async IO.
        let mut accumulated = String::new();

        let read_fut = read_child_output_and_wait(
            &mut child,
            stdout,
            stderr,
            &mut accumulated,
            ctx.sink.as_ref(),
            ctx.session_id.as_ref(),
            &call.id,
        );

        // Apply timeout to the entire read+wait sequence.
        let exit_result: Result<std::process::ExitStatus, std::io::Error> = match per_call_timeout
            .map(std::time::Duration::from_secs)
            .or(ctx.timeout)
        {
            Some(dur) => match tokio::time::timeout(dur, read_fut).await {
                Ok(Ok(status)) => Ok(status),
                Ok(Err(e)) => {
                    return error_tool_result(
                        call.id,
                        call.name,
                        format!("failed to wait for process: {e}"),
                    );
                }
                Err(_) => {
                    // Timeout — kill the process.
                    let _ = child.kill().await;
                    return error_tool_result(
                        call.id,
                        call.name,
                        format!("command timed out after {}s", dur.as_secs()),
                    );
                }
            },
            None => read_fut.await,
        };

        format_exit_result(
            &exit_result,
            &accumulated,
            call.id,
            call.name,
            max_lines,
            max_bytes,
        )
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
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
            max_output_lines: None,
            max_output_bytes: None,
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
            app_paths: crate::common::app_paths::AppPaths::default(),
            sink: None,
            max_output_lines: None,
            max_output_bytes: None,
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
