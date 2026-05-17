//! Lifecycle command runner — executes setup and teardown commands.
//!
//! Two entry points:
//! - [`run_setup_command`] — expects stdout output (last line becomes the session CWD)
//! - [`run_teardown_command`] — only checks exit code, output is irrelevant

use std::path::PathBuf;

use error_stack::Report;
use wherror::Error;

/// Errors that can occur when running a lifecycle command.
#[derive(Debug, Error)]
pub enum LifecycleCommandError {
    /// The command exited with a non-zero status.
    #[error("command failed with exit code {exit_code:?}")]
    CommandFailed {
        /// The process exit code.
        exit_code: Option<i32>,
        /// Captured stdout output.
        stdout: String,
        /// Captured stderr output.
        stderr: String,
    },
    /// The command succeeded but produced no output (empty stdout).
    #[error("command produced no output")]
    NoOutput,
    /// Failed to spawn the command.
    #[error("failed to execute command")]
    ExecutionFailed,
}

/// Shared shell invocation logic used by both setup and teardown runners.
async fn run_command(
    command: &str,
) -> Result<(std::process::Output, String, String), Report<LifecycleCommandError>> {
    use error_stack::ResultExt as _;

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned());

    let output = tokio::process::Command::new(&shell)
        .arg("-c")
        .arg(command)
        .output()
        .await
        .change_context(LifecycleCommandError::ExecutionFailed)
        .attach("failed to spawn lifecycle command")?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        return Err(Report::new(LifecycleCommandError::CommandFailed {
            exit_code: output.status.code(),
            stdout,
            stderr,
        }));
    }

    Ok((output, stdout, stderr))
}

/// Runs a setup command and returns the resulting directory path.
///
/// Spawns the command via the user's shell (`$SHELL`, falling back to `/bin/sh`).
/// Captures stdout and stderr. On success, returns the last non-empty line of
/// stdout (trimmed) as a [`PathBuf`].
///
/// # Errors
///
/// Returns [`LifecycleCommandError::CommandFailed`] if the process exits non-zero.
/// Returns [`LifecycleCommandError::NoOutput`] if stdout is empty.
/// Returns [`LifecycleCommandError::ExecutionFailed`] if the process cannot be spawned.
pub async fn run_setup_command(
    command: &str,
) -> Result<PathBuf, Report<LifecycleCommandError>> {
    let (_output, stdout, _stderr) = run_command(command).await?;

    let last_line = stdout
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .last()
        .ok_or_else(|| Report::new(LifecycleCommandError::NoOutput))?;

    Ok(PathBuf::from(last_line))
}

/// Runs a teardown command. Only checks the exit code — output is ignored.
///
/// Spawns the command via the user's shell (`$SHELL`, falling back to `/bin/sh`).
///
/// # Errors
///
/// Returns [`LifecycleCommandError::CommandFailed`] if the process exits non-zero.
/// Returns [`LifecycleCommandError::ExecutionFailed`] if the process cannot be spawned.
pub async fn run_teardown_command(command: &str) -> Result<(), Report<LifecycleCommandError>> {
    run_command(command).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Setup command tests ---

    #[rstest::rstest]
    #[tokio::test]
    async fn setup_returns_last_line_as_path() {
        // Given a command that echoes a directory path.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().to_string_lossy().to_string();

        // When running the setup command.
        let result = run_setup_command(&format!("echo {path}")).await;

        // Then the result is the directory path.
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from(&path));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn setup_returns_last_non_empty_line() {
        // Given a command that outputs multiple lines.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().to_string_lossy().to_string();

        // When running the setup command with leading output.
        let result =
            run_setup_command(&format!("echo 'setting up...'; echo '{path}'")).await;

        // Then the result is the last non-empty line.
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from(&path));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn setup_trims_whitespace() {
        // Given a command that echoes with trailing whitespace.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().to_string_lossy().to_string();

        // When running the setup command.
        let result = run_setup_command(&format!("echo '{path}  '")).await;

        // Then the result is trimmed.
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from(&path));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn setup_returns_error_on_nonzero_exit() {
        // Given a command that exits with code 1.
        let result = run_setup_command("exit 1").await;

        // Then the result is a CommandFailed error.
        assert!(result.is_err());
        let report = result.unwrap_err();
        let err = report
            .downcast_ref::<LifecycleCommandError>()
            .expect("downcast");
        match err {
            LifecycleCommandError::CommandFailed { exit_code, .. } => {
                assert_eq!(*exit_code, Some(1));
            }
            other => panic!("expected CommandFailed, got {other:?}"),
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn setup_captures_stdout_and_stderr_on_failure() {
        // Given a command that writes to both stdout and stderr before failing.
        let result = run_setup_command(
            "echo 'stdout message'; echo 'stderr message' >&2; exit 1",
        )
        .await;

        // Then the error contains both stdout and stderr output.
        assert!(result.is_err());
        let report = result.unwrap_err();
        let err = report
            .downcast_ref::<LifecycleCommandError>()
            .expect("downcast");
        match err {
            LifecycleCommandError::CommandFailed {
                stdout,
                stderr,
                ..
            } => {
                assert!(stdout.contains("stdout message"));
                assert!(stderr.contains("stderr message"));
            }
            other => panic!("expected CommandFailed, got {other:?}"),
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn setup_returns_error_on_empty_stdout() {
        // Given a setup command that succeeds with no output.
        let result = run_setup_command("true").await;

        // Then the result is a NoOutput error.
        assert!(result.is_err());
        let report = result.unwrap_err();
        let err = report
            .downcast_ref::<LifecycleCommandError>()
            .expect("downcast");
        assert!(matches!(err, LifecycleCommandError::NoOutput));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn setup_accepts_relative_path() {
        // Given a setup command that outputs a relative path.
        let result = run_setup_command("echo ./my-project").await;

        // Then the result is the relative path.
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from("./my-project"));
    }

    // --- Teardown command tests ---

    #[rstest::rstest]
    #[tokio::test]
    async fn teardown_succeeds_with_empty_stdout() {
        // Given a teardown command that removes a marker file (no stdout).
        let dir = tempfile::tempdir().expect("temp dir");
        let marker = dir.path().join("teardown-marker");
        std::fs::write(&marker, b"present").expect("write marker");
        assert!(marker.exists(), "marker should exist before teardown");

        // When running the teardown command.
        let result = run_teardown_command(&format!("rm -f {}", marker.display())).await;

        // Then the command succeeds (teardown doesn't require stdout).
        assert!(result.is_ok(), "teardown should succeed without stdout");
        // And the marker file is gone.
        assert!(!marker.exists(), "teardown should have removed the marker");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn teardown_returns_failure_on_nonzero_exit() {
        // Given a teardown command that fails.
        let result = run_teardown_command("exit 42").await;

        // Then we get a CommandFailed error.
        assert!(result.is_err());
        let report = result.unwrap_err();
        let err = report.downcast_ref::<LifecycleCommandError>();
        assert!(
            matches!(err, Some(LifecycleCommandError::CommandFailed { exit_code, .. }) if *exit_code == Some(42)),
            "expected CommandFailed with exit code 42"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn teardown_commands_run_sequentially() {
        // Given two marker files.
        let dir = tempfile::tempdir().expect("temp dir");
        let marker_a = dir.path().join("a");
        let marker_b = dir.path().join("b");
        std::fs::write(&marker_a, b"a").expect("write a");
        std::fs::write(&marker_b, b"b").expect("write b");

        // When running two teardown commands sequentially (as shutdown does).
        let cmd_a = format!("rm -f {}", marker_a.display());
        let cmd_b = format!("rm -f {}", marker_b.display());
        let _ = run_teardown_command(&cmd_a).await;
        let _ = run_teardown_command(&cmd_b).await;

        // Then both files are removed.
        assert!(!marker_a.exists(), "first teardown should remove marker a");
        assert!(!marker_b.exists(), "second teardown should remove marker b");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn teardown_succeeds_with_any_stdout() {
        // Given a teardown command that produces stdout but exits 0.
        let result = run_teardown_command("echo 'cleaning up...'").await;

        // Then the command succeeds (stdout is ignored).
        assert!(result.is_ok());
    }
}
