//! PowerShell built-in tool — runs Windows-native commands.
//!
//! The sibling of the `bash` tool, for the work that has to happen in
//! PowerShell: Windows modules, the registry, the Windows filesystem, and
//! the tooling that ships with Windows rather than with a POSIX
//! environment.
//!
//! The tool is registered only on Windows. On Linux and macOS the model
//! never sees it: a tool it cannot run is worse than no tool, because the
//! model spends a turn discovering that. The *schema* is compiled on
//! every platform, so its shape is tested everywhere rather than only on
//! the one platform where the tool exists.
//!
//! The PowerShell executable is resolved at launch, the same way the
//! POSIX shell is — see [`jinn_common::shell`]. A machine with no
//! PowerShell is a fact about the machine, not a launch failure: the tool
//! reports it to the model when the model reaches for it.

use jinn_core_types::tool_types::ToolDefinition;

#[cfg(windows)]
use tokio::sync::mpsc;

#[cfg(windows)]
use std::process::Stdio;

#[cfg(windows)]
use crate::tool_types::ToolContext;
#[cfg(windows)]
use crate::tool_types::failed;
#[cfg(windows)]
use jinn_core_types::tool_types::{ToolCall, ToolResult};
#[cfg(windows)]
use jinn_kernel::common::process_kill::kill_process_tree;
#[cfg(windows)]
use jinn_tools_msg::ToolExecutionStarted;
#[cfg(windows)]
use jinn_tools_msg::truncation::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, format_size, truncate_tail,
};

#[cfg(windows)]
use super::BoxedToolFuture;

/// RAII guard that kills the entire process tree when dropped.
///
/// The same guard the `bash` tool uses: aborting the tool's future (on a
/// timeout, or on a cancelled turn) drops the handle, and the drop
/// terminates the child and its descendants rather than orphaning them.
#[cfg(windows)]
struct KillOnDrop {
    child: tokio::process::Child,
}

#[cfg(windows)]
impl KillOnDrop {
    /// Wraps a spawned child process in the kill-on-drop guard.
    fn new(child: tokio::process::Child) -> Self {
        Self { child }
    }
}

#[cfg(windows)]
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        // The cross-platform tree-kill helper: on Unix it signals the
        // child's whole process group, on Windows it enumerates and
        // terminates the process tree.
        kill_process_tree(&mut self.child);
    }
}

#[cfg(windows)]
impl std::ops::Deref for KillOnDrop {
    type Target = tokio::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.child
    }
}

#[cfg(windows)]
impl std::ops::DerefMut for KillOnDrop {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}

/// The `powershell` tool's schema.
///
/// The description names the same timeout affordance the `bash` tool's
/// does, because the model reasons about both the same way.
#[must_use]
pub fn definition(default_timeout_secs: u64) -> ToolDefinition {
    let default_secs_str = default_timeout_secs.to_string();
    ToolDefinition {
        name: "powershell".to_owned(),
        description: format!(
            "Execute a PowerShell command in the current working directory. \
            Returns stdout and stderr. Output is truncated to last 2000 lines or 50KB \
            (whichever is hit first). \
            \
            TIMEOUT: the default is {default_secs_str}s. \
            Pass `max_duration_secs` as a tool-call argument to override — NOT the shell `timeout` command. \
            Example: {{\"command\": \"Get-ChildItem\", \"max_duration_secs\": 600}}. \
            Set `max_duration_secs: 0` to disable the timeout for that call."
        ),
        prompt_snippet: Some("Execute PowerShell commands (Get-ChildItem, Get-Content, etc.)".to_owned()),
        prompt_guidelines: vec![
            "Use this tool for Windows-native commands and PowerShell modules; use the bash tool for POSIX shell syntax (pipes, &&, $VAR).".to_owned(),
            "Proactively set max_duration_secs for commands that may run long (builds, tests, network requests, large compiles). Do NOT use the shell `timeout` command — pass max_duration_secs as a tool-call argument.".to_owned(),
        ],
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "PowerShell command to execute"
                },
                "max_duration_secs": {
                    "type": "number",
                    "description": format!("Maximum duration in seconds before the command is killed. Overrides the default of {default_secs_str}s. Set to 0 to disable the timeout for this call. Pass as a tool-call argument — NOT the shell `timeout` command. Example: {{\"command\": \"Get-ChildItem\", \"max_duration_secs\": 600}}.")
                }
            },
            "required": ["command"]
        }),
        server_tool_type: None,
    }
}

/// Parses the command from the tool call JSON.
///
/// Only extracts `command` — the `max_duration_secs` override is peeked
/// by the dispatcher's `extract_max_duration` reserved-field helper.
#[cfg(windows)]
fn parse_args(raw: &str) -> Result<String, serde_json::Error> {
    let v: serde_json::Value = serde_json::from_str(raw)?;
    let command = v
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    Ok(command)
}

/// Spawns a PowerShell command with stdout/stderr piped and terminal-isolated.
#[cfg(windows)]
fn spawn_powershell_command(
    command: &str,
    cwd: &std::path::Path,
    powershell: &str,
) -> std::io::Result<KillOnDrop> {
    let mut std_cmd = std::process::Command::new(powershell);
    std_cmd
        .args(jinn_common::shell::powershell_command_args(command))
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Detach from jinn's console (CREATE_NO_WINDOW) so a console write
    // cannot print over the TUI, and so the child has no controlling
    // terminal to inherit.
    jinn_common::process_isolation::isolate(&mut std_cmd);

    let mut cmd = tokio::process::Command::from(std_cmd);
    cmd.spawn().map(KillOnDrop::new)
}

/// Reads lines from an async buffered reader and sends them through an mpsc channel.
#[cfg(windows)]
fn spawn_line_reader<R>(reader: R, tx: mpsc::UnboundedSender<String>) -> tokio::task::JoinHandle<()>
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

/// Emits a streaming tool event if both bus and session_id are available.
#[cfg(windows)]
async fn emit_stream_event(
    bus: Option<&jinn_kernel::common::services::bus_service::BusService>,
    session_id: Option<&jinn_core_types::SessionId>,
    event: impl jinn_kernel::common::bus::BusMessage
    + trouper::schema::Schema
    + serde::Serialize
    + trouper::envelope::PayloadValue,
) {
    if let (Some(bus), Some(_)) = (bus, session_id) {
        bus.publish(event).await;
    }
}

/// Formats the final [`ToolResult`] from the process exit status and accumulated output.
#[cfg(windows)]
fn format_exit_result(
    exit_result: &Result<std::process::ExitStatus, std::io::Error>,
    accumulated: &str,
    tool_call_id: jinn_core_types::ToolCallId,
    tool_name: String,
    max_lines: usize,
    max_bytes: usize,
) -> ToolResult {
    let mut content = accumulated.to_owned();
    let success = match exit_result {
        Ok(status) => status.success(),
        Err(_) => false,
    };

    if !success && !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    if !success {
        let code = exit_result
            .as_ref()
            .ok()
            .and_then(std::process::ExitStatus::code)
            .map_or_else(|| "unknown (signal)".to_owned(), |code| code.to_string());
        content.push_str(&format!("Command exited with code {code}"));
    }

    let truncation_result = truncate_tail(&content, max_lines, max_bytes);
    if truncation_result.truncated {
        let meta = truncation_result.meta;
        let full = Some(content);
        let truncated = truncation_result.content;
        let (output, truncation) = match meta {
            Some(meta) => {
                let start_line = meta.total_lines.saturating_sub(meta.output_lines) + 1;
                let end_line = meta.total_lines;
                let notice = if meta.truncated_by == jinn_core_types::tool_types::TruncatedBy::Bytes
                {
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
                let mut output = truncated;
                output.push_str(&notice);
                (output, Some(meta))
            }
            None => (truncated, None),
        };
        ToolResult {
            tool_call_id,
            name: tool_name,
            content: output,
            success,
            full_content: full,
            truncation,
            pin_position: None,
        }
    } else {
        ToolResult {
            tool_call_id,
            name: tool_name,
            content: truncation_result.content,
            success,
            full_content: None,
            truncation: None,
            pin_position: None,
        }
    }
}

/// Executes the `powershell` built-in tool.
///
/// Windows-only: the tool is not registered elsewhere, so this is never
/// reached on Linux or macOS.
#[cfg(windows)]
pub fn execute(call: ToolCall, ctx: ToolContext) -> BoxedToolFuture {
    Box::pin(async move {
        let command = match parse_args(&call.arguments) {
            Ok(command) => command,
            Err(error) => {
                return failed(
                    call.id,
                    call.name,
                    format!("failed to parse arguments: {error}"),
                );
            }
        };

        if command.is_empty() {
            return failed(call.id, call.name, "command is empty".to_owned());
        }

        // The PowerShell executable, resolved at launch. A machine with no
        // PowerShell is a real state, and the model reads why the command
        // could not run.
        let powershell = match ctx.shells.powershell() {
            Ok(shell) => shell.program().to_owned(),
            Err(error) => {
                return failed(call.id, call.name, error.to_string());
            }
        };

        let cwd = ctx.cwd.clone();

        emit_stream_event(
            ctx.bus.as_ref(),
            ctx.session_id.as_ref(),
            ToolExecutionStarted {
                session_id: ctx.session_id.clone().unwrap_or_default(),
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                dispatched_at: jiff::Timestamp::now(),
            },
        )
        .await;

        let mut child = match spawn_powershell_command(&command, &cwd, &powershell) {
            Ok(child) => child,
            Err(error) => {
                return failed(
                    call.id,
                    call.name,
                    format!("failed to execute command: {error}"),
                );
            }
        };

        let Some(stdout) = child.stdout.take() else {
            return failed(
                call.id,
                call.name,
                "failed to capture stdout: pipe was not set up".to_owned(),
            );
        };
        let Some(stderr) = child.stderr.take() else {
            return failed(
                call.id,
                call.name,
                "failed to capture stderr: pipe was not set up".to_owned(),
            );
        };

        let max_lines = ctx.max_output_lines.unwrap_or(DEFAULT_MAX_LINES);
        let max_bytes = ctx.max_output_bytes.unwrap_or(DEFAULT_MAX_BYTES);

        let mut accumulated = String::new();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let stdout_handle = spawn_line_reader(tokio::io::BufReader::new(stdout), tx.clone());
        let stderr_handle = spawn_line_reader(tokio::io::BufReader::new(stderr), tx);

        while let Some(line) = rx.recv().await {
            accumulated.push_str(&line);
            accumulated.push('\n');
        }

        let _ = stdout_handle.await;
        let _ = stderr_handle.await;

        let exit_result = child.wait().await;

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
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;

    #[rstest::rstest]
    #[test]
    fn the_definition_names_the_powershell_tool() {
        // Given the tool's schema.
        // When reading its name.
        // Then it is `powershell`.
        assert_eq!(definition(300).name, "powershell");
    }

    #[rstest::rstest]
    #[test]
    fn the_definition_requires_a_command_parameter() {
        // Given the tool's schema.
        let definition = definition(300);

        // When reading its parameters.
        // Then the command is required and the timeout is offered.
        let required = definition
            .parameters
            .get("required")
            .and_then(|value| value.as_array())
            .expect("the schema names its required parameters");
        assert_eq!(
            required,
            &vec![serde_json::Value::String("command".to_owned())]
        );
        assert!(
            definition
                .parameters
                .get("properties")
                .and_then(|value| value.get("max_duration_secs"))
                .is_some(),
            "the schema must offer max_duration_secs"
        );
    }

    #[rstest::rstest]
    #[test]
    fn the_definition_names_the_default_timeout() {
        // Given the tool's schema built with a default timeout.
        let definition = definition(42);

        // When reading its description.
        // Then the timeout is named.
        assert!(
            definition.description.contains("42s"),
            "the description must name the default timeout: {}",
            definition.description
        );
    }

    #[rstest::rstest]
    #[test]
    fn the_definition_separates_powershell_from_posix_syntax() {
        // Given the tool's schema.
        let definition = definition(300);

        // When reading its guidelines.
        // Then one of them points at the bash tool for POSIX syntax.
        assert!(
            definition
                .prompt_guidelines
                .iter()
                .any(|guideline| guideline.contains("bash tool")),
            "the guidelines must point at the bash tool: {:?}",
            definition.prompt_guidelines
        );
    }

    #[rstest::rstest]
    #[test]
    fn the_command_runs_with_no_profile_and_non_interactively() {
        // Given the argument list the tool spawns PowerShell with.
        // When reading it.
        // Then the profile is skipped and the shell never prompts.
        let args = jinn_common::shell::powershell_command_args("Get-ChildItem");
        assert_eq!(
            &args[..2],
            &["-NoProfile".to_owned(), "-NonInteractive".to_owned()]
        );
    }
}
