//! Bash built-in tool - executes shell commands.
//!
//! Spawns a shell process with piped stdout/stderr, streaming output
//! via batched `ToolExecutionOutput` events. Output lines are accumulated
//! and flushed every 500ms (or when the buffer exceeds 4KB) to reduce
//! event volume and prevent dropped keystrokes from terminal overload.

use std::fmt::Write as _;
use std::process::Stdio;
use std::time::Duration;

use crate::common::actor::message_sink::MessageSink;
use crate::common::process_kill::kill_process_tree;
use crate::feat::preferences_actor::user_preferences::BashConfig;
use crate::feat::tools_actor::protocol::event::{ToolExecutionOutput, ToolExecutionStarted};
use crate::feat::tools_actor::tool_types::{ToolCall, ToolContext, ToolDefinition, ToolResult};
use crate::protocol::{Event, SessionId};

use super::truncation::{DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, format_size, truncate_tail};

use super::BoxedToolFuture;

/// RAII guard that kills the entire process group when dropped.
///
/// When the tokio task running a bash command is aborted (via
/// `JoinHandle::abort()`), this guard's `Drop` implementation sends
/// `SIGKILL` to the child's entire process group, ensuring that
/// descendant processes (e.g., `find /` spawned by bash) are also
/// terminated. Without this, aborting the future only drops the
/// handle - the OS processes continue running as orphans.
struct KillOnDrop {
    child: tokio::process::Child,
}

impl KillOnDrop {
    /// Wraps a spawned child process in the kill-on-drop guard.
    fn new(child: tokio::process::Child) -> Self {
        Self { child }
    }
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        // Delegate to the cross-platform tree-kill helper. On Unix this sends
        // SIGKILL to the child's entire process group (race-free); on Windows it
        // enumerates and terminates the process tree via `kill_tree`. See
        // `common::process_kill` for the per-platform guarantees and trade-offs.
        kill_process_tree(&mut self.child);
    }
}

impl std::ops::Deref for KillOnDrop {
    type Target = tokio::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.child
    }
}

impl std::ops::DerefMut for KillOnDrop {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}

/// Returns the tool definition for the `bash` built-in tool.
///
/// The resolved `default_timeout_secs` from `config` is injected into the
/// schema descriptions so the model can see what default it is operating under.
pub fn definition(config: &BashConfig) -> ToolDefinition {
    let default_secs = config.default_timeout_secs.unwrap_or(0);
    let default_secs_str = default_secs.to_string();
    ToolDefinition {
        name: "bash".to_owned(),
        description: format!(
            "Execute a bash command in the current working directory. \
            Returns stdout and stderr. Output is truncated to last 2000 lines or 50KB \
            (whichever is hit first). Default timeout is {default_secs_str}s; \
            override per call via the `timeout` parameter."
        ),
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
                    "description": format!("Timeout in seconds. Overrides the default of {default_secs_str}s.")
                }
            },
            "required": ["command"]
        }),
        server_tool_type: None,
    }
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

/// Flush the streaming event buffer early when it exceeds this size,
/// preventing unbounded memory growth between timer ticks.
const STREAM_FLUSH_THRESHOLD: usize = 4096;

/// Interval for batching streaming tool output events.
/// Accumulated output is flushed every 500ms to reduce event volume
/// and prevent dropped keystrokes from terminal emulator overload.
const STREAM_FLUSH_INTERVAL: Duration = Duration::from_millis(500);

/// Truncates accumulated streaming output to prevent unbounded memory growth.
/// Uses the shared tail-truncation logic.
fn truncate_streaming_output(content: &str) -> String {
    truncate_tail(content, DEFAULT_MAX_LINES, DEFAULT_MAX_BYTES).content
}

/// Emits buffered output as a single `ToolExecutionOutput` event.
fn flush_buffer(
    buffer: &str,
    sink: Option<&std::sync::Arc<dyn MessageSink>>,
    session_id: Option<&SessionId>,
    tool_call_id: &str,
) {
    emit_stream_event(
        sink,
        session_id,
        Event::ToolExecutionOutput(ToolExecutionOutput {
            session_id: session_id.cloned().unwrap_or_default(),
            tool_call_id: tool_call_id.to_owned(),
            output: buffer.to_owned(),
        }),
    );
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
        pin_position: None,
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
    let mut content = accumulated.to_owned();
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
            let notice = if meta.truncated_by == jinn_provider::tool_types::TruncatedBy::Bytes {
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
                pin_position: None,
            }
        } else {
            // truncated but no meta - return unformatted truncated content
            ToolResult {
                tool_call_id,
                name: tool_name,
                content: truncation_result.content,
                success,
                full_content: Some(content),
                truncation: None,
                pin_position: None,
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
            pin_position: None,
        }
    }
}

/// Reads lines from an async buffered reader and sends them through an mpsc channel.
///
/// Designed for merging stdout and stderr from a child process into a single
/// stream. The spawned task exits when the reader reaches EOF or the channel
/// is closed.
fn spawn_line_reader<R>(
    reader: R,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
) -> tokio::task::JoinHandle<()>
where
    R: tokio::io::AsyncBufRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if tx.send(line).is_err() {
                break;
            }
        }
    })
}

/// Buffers streaming output lines and flushes them as batched events.
///
/// Accumulates lines into an internal buffer, emitting a `ToolExecutionOutput`
/// event when the buffer exceeds [`STREAM_FLUSH_THRESHOLD`] or when explicitly
/// flushed (timer tick or loop exit). Also guards against unbounded memory
/// growth by truncating the caller's `accumulated` string in-place when it
/// exceeds [`STREAM_BUFFER_MAX_BYTES`].
struct StreamingBatcher {
    buffer: String,
}

impl StreamingBatcher {
    fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    /// Appends a line to both the flush buffer and the caller's accumulated output.
    ///
    /// If `accumulated` exceeds [`STREAM_BUFFER_MAX_BYTES`], it is truncated
    /// in-place using [`truncate_streaming_output`].
    fn push_line(&mut self, line: &str, accumulated: &mut String) {
        accumulated.push_str(line);
        accumulated.push('\n');
        self.buffer.push_str(line);
        self.buffer.push('\n');

        if accumulated.len() > STREAM_BUFFER_MAX_BYTES {
            let truncated = truncate_streaming_output(accumulated);
            *accumulated = truncated;
        }
    }

    /// Returns `true` when the buffer exceeds [`STREAM_FLUSH_THRESHOLD`].
    fn should_flush(&self) -> bool {
        self.buffer.len() > STREAM_FLUSH_THRESHOLD
    }

    /// Emits the buffered output as a streaming event and clears the buffer.
    ///
    /// No-op when the buffer is empty.
    fn flush(
        &mut self,
        sink: Option<&std::sync::Arc<dyn MessageSink>>,
        session_id: Option<&SessionId>,
        tool_call_id: &str,
    ) {
        if self.buffer.is_empty() {
            return;
        }
        flush_buffer(&self.buffer, sink, session_id, tool_call_id);
        self.buffer.clear();
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

    let stdout_handle = spawn_line_reader(tokio::io::BufReader::new(stdout), tx.clone());
    let stderr_handle = spawn_line_reader(tokio::io::BufReader::new(stderr), tx.clone());
    drop(tx);

    let mut batcher = StreamingBatcher::new();
    let mut timer = tokio::time::interval(STREAM_FLUSH_INTERVAL);
    timer.tick().await; // Consume the immediate first tick.

    loop {
        tokio::select! {
            line = rx.recv() => match line {
                Some(line) => {
                    batcher.push_line(&line, accumulated);
                    if batcher.should_flush() {
                        batcher.flush(sink, session_id, tool_call_id);
                    }
                }
                None => break,
            },
            _ = timer.tick() => {
                batcher.flush(sink, session_id, tool_call_id);
            }
        }
    }

    batcher.flush(sink, session_id, tool_call_id);

    // Wait for both readers to finish.
    let _ = stdout_handle.await;
    let _ = stderr_handle.await;

    // Wait for process exit.
    child.wait().await
}

/// Spawns a shell command with stdout/stderr piped and isolated process group.
///
/// Disconnects stdin from the controlling TTY so child processes
/// (and their entire process tree) are removed from the kernel's
/// TTY input fd list. Without this, child processes inherit the
/// parent's stdin fd, and under heavy spawn pressure (e.g. cargo
/// nextest spawning hundreds of test binaries) the kernel's TTY
/// input buffer overflows before the event thread can drain it,
/// causing dropped keystrokes.
///
/// Places the child in its own process group so that the
/// kill-on-drop guard can terminate the entire group
/// (child + all descendants) on cancel.
fn spawn_shell_command(
    shell: &str,
    command: &str,
    cwd: &std::path::Path,
) -> std::io::Result<KillOnDrop> {
    let mut cmd = tokio::process::Command::new(shell);
    cmd.arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Place the child in its own process group on Unix so that
    // `kill_process_tree` can atomically signal the whole group with
    // `kill(-pgid)`. Windows has no process-group signalling analogue
    // (`Command::process_group` is Unix-only); its tree kill is enumerative
    // via `kill_tree`.
    #[cfg(unix)]
    cmd.process_group(0);

    cmd.spawn().map(KillOnDrop::new)
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

        let (shell, cwd) = (ctx.shell.clone(), ctx.cwd.clone());

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

        let spawn_result = spawn_shell_command(&shell, &command, &cwd);

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
            .or(ctx.bash_default_timeout)
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
                    // Timeout - kill the process.
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
    #![allow(clippy::expect_used, clippy::panic, clippy::unreachable, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use crate::common::actor::RecordingSink;
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
            bash_default_timeout: None,
            state: None,
            session_id: None,
            app_paths: crate::common::app_paths::AppPaths::default(),
            sink: None,
            shell: "/bin/sh".to_owned(),
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

    #[tokio::test]
    async fn execute_uses_bash_default_timeout_when_no_per_call() {
        // Given a ToolContext with a 1-second bash_default_timeout and no per-call timeout.
        let mut ctx = test_ctx();
        ctx.bash_default_timeout = Some(std::time::Duration::from_secs(1));
        let call = ToolCall {
            id: "call_default_to".to_owned(),
            name: "bash".to_owned(),
            arguments: serde_json::json!({"command": "sleep 10"}).to_string(),
        };

        // When executing.
        let result = execute(call, ctx).await;

        // Then the bash default timeout fires.
        assert!(!result.success);
        assert!(result.content.contains("timed out"));
    }

    #[tokio::test]
    async fn execute_per_call_timeout_overrides_bash_default() {
        // Given a short per-call timeout and a longer bash_default_timeout.
        let mut ctx = test_ctx();
        ctx.bash_default_timeout = Some(std::time::Duration::from_secs(10));
        let call = ToolCall {
            id: "call_override".to_owned(),
            name: "bash".to_owned(),
            arguments: serde_json::json!({
                "command": "sleep 5",
                "timeout": 1
            })
            .to_string(),
        };

        // When executing.
        let result = execute(call, ctx).await;

        // Then the per-call (1s) wins over the default (10s).
        assert!(!result.success);
        assert!(result.content.contains("timed out"));
    }

    #[tokio::test]
    async fn execute_no_timeout_when_default_is_none_and_no_per_call() {
        // Given no per-call timeout and bash_default_timeout = None.
        let ctx = test_ctx(); // bash_default_timeout: None
        let call = ToolCall {
            id: "call_no_to".to_owned(),
            name: "bash".to_owned(),
            arguments: serde_json::json!({"command": "echo hi"}).to_string(),
        };

        // When executing a fast command.
        let result = execute(call, ctx).await;

        // Then it succeeds (no timeout enforced).
        assert!(result.success);
        assert!(result.content.contains("hi"));
    }

    /// Verifies that `KillOnDrop` terminates the child process and its
    /// entire process group when dropped.
    ///
    /// Spawns a bash command that starts a background sleep child, then
    /// drops the guard. Both processes should be killed.
    #[cfg(unix)]
    #[rstest::rstest]
    #[tokio::test]
    async fn kill_on_drop_terminates_process_group() {
        use std::process::Stdio;

        // Given a bash command that spawns a background child process.
        let shell = "/bin/sh".to_owned();
        let guard = {
            let child = tokio::process::Command::new(&shell)
                .arg("-c")
                .arg("sleep 60 & sleep 60")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
                .expect("spawn should succeed");
            let pid = child.id().expect("child should have a PID");
            assert!(pid > 0, "child PID should be positive");
            KillOnDrop::new(child)
        };

        // Give the processes a moment to start.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // When dropping the guard (simulating task abort).
        drop(guard);

        // Then give the kernel a moment to reap the processes.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Verify no 'sleep 60' processes survived.
        let output = tokio::process::Command::new("pgrep")
            .arg("-f")
            .arg("sleep 60")
            .output()
            .await
            .expect("pgrep should run");
        let stdout = String::from_utf8_lossy(&output.stdout);
        // No "sleep 60" processes should be running.
        assert!(
            stdout.trim().is_empty(),
            "expected no 'sleep 60' processes, but found: {stdout}"
        );
    }

    /// Windows mirror of `kill_on_drop_terminates_process_group`.
    ///
    /// Cannot run in the Linux dev environment — included so CI on Windows
    /// validates the `kill_tree`-based `Drop` path. Uses `ping` as a Windows-native
    /// long-running command (the classic `cmd` sleep substitute) and verifies
    /// termination via `tasklist`.
    #[cfg(windows)]
    #[rstest::rstest]
    #[tokio::test]
    async fn kill_on_drop_terminates_process_tree_windows() {
        use std::process::Stdio;

        // Given a cmd command that launches a long-running background child.
        // `ping -n 60` waits ~59s — a reliable Windows-native delay.
        let guard = {
            let child = tokio::process::Command::new("cmd")
                .arg("/c")
                .arg("ping 127.0.0.1 -n 60 > nul")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn should succeed");
            assert!(child.id().unwrap_or(0) > 0, "child PID should be positive");
            KillOnDrop::new(child)
        };

        // Give the process a moment to start.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // When dropping the guard (simulating task abort).
        drop(guard);

        // Then give kill_tree a moment to enumerate and terminate the tree.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Verify no `ping` processes survived via tasklist.
        let output = tokio::process::Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq ping.exe", "/NH"])
            .output()
            .await
            .expect("tasklist should run");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.trim().is_empty() || stdout.contains("INFO: No tasks"),
            "expected no 'ping' processes, but found: {stdout}"
        );
    }

    // --- StreamingBatcher unit tests ---

    #[rstest::rstest]
    fn push_line_appends_to_accumulated() {
        // Given a fresh batcher.
        let mut batcher = StreamingBatcher::new();
        let mut accumulated = String::new();

        // When pushing a line.
        batcher.push_line("hello world", &mut accumulated);

        // Then the accumulated string contains the line.
        assert_eq!(accumulated, "hello world\n");
    }

    #[rstest::rstest]
    fn should_flush_returns_false_below_threshold() {
        // Given a batcher with a short line.
        let mut batcher = StreamingBatcher::new();
        let mut accumulated = String::new();
        batcher.push_line("short", &mut accumulated);

        // When checking if the buffer should flush.
        // Then it returns false (buffer is well under 4096 bytes).
        assert!(!batcher.should_flush());
    }

    #[rstest::rstest]
    fn should_flush_returns_true_at_threshold() {
        // Given a batcher with enough data to exceed the flush threshold.
        let mut batcher = StreamingBatcher::new();
        let mut accumulated = String::new();

        // Push lines until buffer exceeds STREAM_FLUSH_THRESHOLD (4096 bytes).
        let line = "a".repeat(1000);
        for _ in 0..5 {
            batcher.push_line(&line, &mut accumulated);
        }

        // When checking if the buffer should flush.
        // Then it returns true.
        assert!(batcher.should_flush());
    }

    #[rstest::rstest]
    fn flush_is_noop_on_empty_buffer() {
        // Given a fresh batcher and a recording sink.
        let mut batcher = StreamingBatcher::new();
        let recording = std::sync::Arc::new(RecordingSink::new());
        let sink: std::sync::Arc<dyn MessageSink> = recording.clone();
        let session_id = SessionId::default();

        // When flushing an empty buffer.
        batcher.flush(Some(&sink), Some(&session_id), "test_call");

        // Then no events were sent.
        assert!(recording.events().is_empty());
    }

    #[rstest::rstest]
    fn flush_emits_and_clears_buffer() {
        // Given a batcher with pushed lines and a recording sink.
        let mut batcher = StreamingBatcher::new();
        let mut accumulated = String::new();
        let recording = std::sync::Arc::new(RecordingSink::new());
        let sink: std::sync::Arc<dyn MessageSink> = recording.clone();
        let session_id = SessionId::default();

        batcher.push_line("line one", &mut accumulated);
        batcher.push_line("line two", &mut accumulated);

        // When flushing.
        batcher.flush(Some(&sink), Some(&session_id), "test_call");

        // Then a ToolExecutionOutput event was emitted.
        let events = recording.events();
        assert_eq!(events.len(), 1);

        // And the event contains both lines.
        let Event::ToolExecutionOutput(output) = &events[0] else {
            panic!("expected ToolExecutionOutput event");
        };
        assert!(output.output.contains("line one"));
        assert!(output.output.contains("line two"));

        // And the buffer is now empty (should_flush returns false).
        assert!(!batcher.should_flush());
    }

    #[rstest::rstest]
    fn push_line_truncates_accumulated_at_max_bytes() {
        // Given a fresh batcher.
        let mut batcher = StreamingBatcher::new();
        let mut accumulated = String::new();

        // When pushing enough data to exceed STREAM_BUFFER_MAX_BYTES.
        // STREAM_BUFFER_MAX_BYTES = DEFAULT_MAX_BYTES * 2 = 50KB * 2 = 100KB.
        let line = "x".repeat(2048); // 2KB per line
        for _ in 0..55 {
            // 55 * 2KB = 110KB > 100KB
            batcher.push_line(&line, &mut accumulated);
        }

        // Then accumulated was truncated (less than the untruncated 110KB).
        assert!(
            accumulated.len() < 110 * 1024,
            "accumulated should be truncated, but is {} bytes",
            accumulated.len()
        );
    }

    // --- Phase 4: Mutation-killing tests ---

    #[rstest::rstest]
    fn format_exit_result_success_with_output() {
        // Given a successful exit and some output.
        let status = std::process::Command::new("true")
            .status()
            .expect("true should run");
        let exit_result: Result<std::process::ExitStatus, std::io::Error> = Ok(status);

        // When formatting the exit result.
        let result = format_exit_result(
            &exit_result,
            "hello world\n",
            "call_1".to_owned(),
            "bash".to_owned(),
            DEFAULT_MAX_LINES,
            DEFAULT_MAX_BYTES,
        );

        // Then success is true and content contains the output.
        assert!(result.success);
        assert_eq!(result.content, "hello world\n");
        assert!(result.full_content.is_none());
    }

    #[rstest::rstest]
    fn format_exit_result_failure_with_exit_code() {
        // Given a non-zero exit.
        let status = std::process::Command::new("bash")
            .args(["-c", "exit 42"])
            .status()
            .expect("bash should run");
        let exit_result: Result<std::process::ExitStatus, std::io::Error> = Ok(status);

        // When formatting the exit result with output.
        let result = format_exit_result(
            &exit_result,
            "some output\n",
            "call_2".to_owned(),
            "bash".to_owned(),
            DEFAULT_MAX_LINES,
            DEFAULT_MAX_BYTES,
        );

        // Then success is false and content includes exit code.
        assert!(!result.success);
        assert!(result.content.contains("some output"));
        assert!(result.content.contains("Command exited with code 42"));
    }

    #[rstest::rstest]
    fn format_exit_result_failure_without_output() {
        // Given a non-zero exit with empty output.
        let status = std::process::Command::new("bash")
            .args(["-c", "exit 1"])
            .status()
            .expect("bash should run");
        let exit_result: Result<std::process::ExitStatus, std::io::Error> = Ok(status);

        // When formatting the exit result.
        let result = format_exit_result(
            &exit_result,
            "",
            "call_3".to_owned(),
            "bash".to_owned(),
            DEFAULT_MAX_LINES,
            DEFAULT_MAX_BYTES,
        );

        // Then success is false and content includes exit code.
        assert!(!result.success);
        assert!(result.content.contains("Command exited with code 1"));
    }

    #[rstest::rstest]
    fn format_exit_result_io_error() {
        // Given an I/O error.
        let exit_result: Result<std::process::ExitStatus, std::io::Error> = Err(
            std::io::Error::new(std::io::ErrorKind::NotFound, "not found"),
        );

        // When formatting the exit result.
        let result = format_exit_result(
            &exit_result,
            "",
            "call_4".to_owned(),
            "bash".to_owned(),
            DEFAULT_MAX_LINES,
            DEFAULT_MAX_BYTES,
        );

        // Then success is false.
        assert!(!result.success);
    }

    #[rstest::rstest]
    fn format_exit_result_failure_appends_newline_if_missing() {
        // Given a non-zero exit with output not ending in newline.
        let status = std::process::Command::new("bash")
            .args(["-c", "exit 3"])
            .status()
            .expect("bash should run");
        let exit_result: Result<std::process::ExitStatus, std::io::Error> = Ok(status);

        // When formatting the exit result with output not ending in newline.
        let result = format_exit_result(
            &exit_result,
            "no trailing newline",
            "call_5".to_owned(),
            "bash".to_owned(),
            DEFAULT_MAX_LINES,
            DEFAULT_MAX_BYTES,
        );

        // Then a newline is added before the exit code message.
        assert!(!result.success);
        assert!(
            result
                .content
                .contains("no trailing newline\nCommand exited with code 3")
        );
    }

    #[rstest::rstest]
    fn truncate_streaming_output_returns_non_empty() {
        // Given a string that's well within limits.
        let content = "hello\nworld\n";

        // When truncating.
        let result = truncate_streaming_output(content);

        // Then the content is returned as-is.
        assert_eq!(result, content);
    }

    #[rstest::rstest]
    fn push_line_at_exact_stream_buffer_max_does_not_truncate_prematurely() {
        // Given a batcher and accumulated exactly at STREAM_BUFFER_MAX_BYTES - 1.
        let mut batcher = StreamingBatcher::new();
        let mut accumulated = String::new();

        // Push lines until just below the threshold.
        let line = "a".repeat(1000);
        for _ in 0..(STREAM_BUFFER_MAX_BYTES / 1001) {
            batcher.push_line(&line, &mut accumulated);
        }
        let _len_before = accumulated.len();

        // When pushing one more line that pushes over the threshold.
        batcher.push_line("extra", &mut accumulated);

        // Then accumulated grew (or was truncated).
        assert_ne!(accumulated.len(), 0);
    }

    #[rstest::rstest]
    fn should_flush_returns_false_at_exact_threshold() {
        // Given a batcher with exactly STREAM_FLUSH_THRESHOLD bytes.
        let mut batcher = StreamingBatcher::new();
        let mut accumulated = String::new();

        // Push data that is exactly at the threshold boundary (not exceeding).
        let line = "a".repeat(STREAM_FLUSH_THRESHOLD - 1); // +1 for \n = STREAM_FLUSH_THRESHOLD
        batcher.push_line(&line, &mut accumulated);

        // When checking if it should flush.
        // Then it returns false (len == threshold, not > threshold).
        assert!(!batcher.should_flush());
    }

    #[rstest::rstest]
    fn definition_includes_default_timeout_secs_in_schema() {
        // Given the default BashConfig.
        let config = BashConfig::default();

        // When building the tool definition.
        let def = definition(&config);

        // Then the tool description mentions the default.
        assert!(
            def.description.contains("180"),
            "description must mention 180, got: {}",
            def.description
        );

        // And the timeout field description mentions the default.
        let params = &def.parameters;
        let timeout_desc = params
            .get("properties")
            .and_then(|p| p.get("timeout"))
            .and_then(|t| t.get("description"))
            .and_then(|d| d.as_str())
            .expect("timeout description present");
        assert!(
            timeout_desc.contains("180"),
            "timeout description must mention 180, got: {timeout_desc}"
        );
    }

    #[rstest::rstest]
    fn definition_includes_custom_timeout_in_schema() {
        // Given a BashConfig with a custom default.
        let config = BashConfig {
            default_timeout_secs: Some(60),
        };

        // When building the tool definition.
        let def = definition(&config);

        // Then the timeout field description mentions the custom default.
        let timeout_desc = def
            .parameters
            .get("properties")
            .and_then(|p| p.get("timeout"))
            .and_then(|t| t.get("description"))
            .and_then(|d| d.as_str())
            .expect("timeout description present");
        assert!(
            timeout_desc.contains("60"),
            "timeout description must mention 60, got: {timeout_desc}"
        );
    }

    #[rstest::rstest]
    fn definition_handles_none_timeout_gracefully() {
        // Given a BashConfig with no default timeout.
        let config = BashConfig {
            default_timeout_secs: None,
        };

        // When building the tool definition.
        let def = definition(&config);

        // Then it succeeds and the timeout field still has a description.
        let timeout_desc = def
            .parameters
            .get("properties")
            .and_then(|p| p.get("timeout"))
            .and_then(|t| t.get("description"))
            .and_then(|d| d.as_str())
            .expect("timeout description present");
        assert!(
            !timeout_desc.is_empty(),
            "timeout description must not be empty"
        );
    }
}
