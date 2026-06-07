//! Lifecycle command runner - executes setup and teardown commands.
//!
//! Two entry points:
//! - [`run_setup_command`] - expects stdout output (last line becomes the session CWD)
//! - [`run_teardown_command`] - only checks exit code, output is irrelevant

use std::path::PathBuf;

use error_stack::Report;
use wherror::Error;

// Note: kill_process_tree(&mut Child) is NOT imported here — the cancel path
// uses the PID-based kill_process_group_by_pid (called from the actor's cancel
// handler), and this module no longer holds the Child past spawn.

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

    /// The path returned by the command could not be resolved on the filesystem.
    #[error("path does not exist or cannot be resolved: {path}")]
    InvalidPath {
        /// The path that was returned by the command.
        path: PathBuf,
    },
    /// The path returned by the command is not a directory.
    #[error("path is not a directory: {path}")]
    NotADirectory {
        /// The path that was returned by the command.
        path: PathBuf,
    },
    /// Failed to spawn the command.
    #[error("failed to execute command")]
    ExecutionFailed,
}

/// Shared shell invocation logic used by both setup and teardown runners.
async fn run_command(
    command: &str,
    shell: &str,
) -> Result<(std::process::Output, String, String), Report<LifecycleCommandError>> {
    use error_stack::ResultExt as _;

    let output = tokio::process::Command::new(shell)
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

/// Runs a setup command and returns the resulting directory path, if any.
///
/// Spawns the command via the provided shell.
/// Captures stdout and stderr. On success, returns the last non-empty line of
/// stdout (trimmed), canonicalized and verified as an existing directory.
///
/// A command that exits 0 but prints no usable stdout line is treated as a
/// successful side-effect-only setup and returns `Ok(None)`; the caller keeps
/// the default CWD.
///
/// # Errors
///
/// Returns [`LifecycleCommandError::CommandFailed`] if the process exits non-zero.
/// Returns [`LifecycleCommandError::InvalidPath`] if the path cannot be resolved.
/// Returns [`LifecycleCommandError::NotADirectory`] if the path is not a directory.
/// Returns [`LifecycleCommandError::ExecutionFailed`] if the process cannot be spawned.
pub async fn run_setup_command(
    command: &str,
    shell: &str,
) -> Result<Option<PathBuf>, Report<LifecycleCommandError>> {
    use error_stack::ResultExt as _;

    let (_output, stdout, _stderr) = run_command(command, shell).await?;

    let last_line = stdout.lines().map(str::trim).rfind(|line| !line.is_empty());

    let canonical = match last_line {
        Some(last_line) => {
            let raw_path = PathBuf::from(last_line);

            let canonical = tokio::fs::canonicalize(&raw_path)
                .await
                .change_context(LifecycleCommandError::InvalidPath {
                    path: raw_path.clone(),
                })
                .attach("setup command output is not a valid path")?;

            if !canonical.is_dir() {
                return Err(Report::new(LifecycleCommandError::NotADirectory {
                    path: canonical,
                }));
            }

            canonical
        }
        None => return Ok(None),
    };

    Ok(Some(canonical))
}

/// Runs a teardown command. Only checks the exit code - output is ignored.
///
/// Spawns the command via the provided shell.
///
/// # Errors
///
/// Returns [`LifecycleCommandError::CommandFailed`] if the process exits non-zero.
/// Returns [`LifecycleCommandError::ExecutionFailed`] if the process cannot be spawned.
pub async fn run_teardown_command(
    command: &str,
    shell: &str,
) -> Result<(), Report<LifecycleCommandError>> {
    run_command(command, shell).await?;
    Ok(())
}

/// Handle used to cancel a running lifecycle shell process from outside its
/// reader task.
///
/// Carries:
/// - `pid`: the process-group leader's id. On Unix the child is spawned with
///   `process_group(0)`, so its pid _is_ the process-group id. The cancel
///   path signals the whole group via `kill_process_group_by_pid(pid)`, which
///   reaches the leader _and_ any backgrounded descendants (grandchildren)
///   in a single syscall — without touching the reader task's owned `Child`.
/// - `abort_handle`: the abort handle of the **inner reader task**. Aborting
///   it makes the outer wrapper task (in `lifecycle.rs`) observe a `JoinError`
///   and take its `Err(_)` arm, which emits the "cancelled" finish command. The
///   finish handler then owns all cleanup (busy, cwd, chat entry, phase).
///
/// This is lock-free and await-free: the cancel handler can run on a tokio
///   worker thread without panicking (the previous `SharedChild` +
///   `blocking_lock` design crashed and would also have deadlocked, since the
///   reader task held the lock across `child.wait().await`).
#[derive(Debug)]
pub struct LifecycleCancelHandle {
    /// Process-group id of the running lifecycle command (also its pid).
    pub pid: u32,
    /// Abort handle for the inner reader task.
    pub abort_handle: tokio::task::AbortHandle,
}

/// Result type returned by [`spawn_setup_command`].
pub type SpawnSetupResult = Result<
    (
        LifecycleCancelHandle,
        tokio::task::JoinHandle<Result<Option<PathBuf>, Report<LifecycleCommandError>>>,
    ),
    Report<LifecycleCommandError>,
>;

/// Result type returned by [`spawn_teardown_command`].
pub type SpawnTeardownResult = Result<
    (
        LifecycleCancelHandle,
        tokio::task::JoinHandle<Result<(), Report<LifecycleCommandError>>>,
    ),
    Report<LifecycleCommandError>,
>;

/// Spawns a setup command and returns a cancel handle and a joinable reader task.
///
/// The [`LifecycleCancelHandle`] can be used to cancel the command from outside
/// this task (see its docs). The reader task awaits exit, reads stdout/stderr,
/// and produces the canonicalized CWD path.
///
/// # Panics
///
/// Panics if the spawned child has no PID immediately after spawn (should never happen for a process group child).
///
/// # Errors
/// Returns an error under any of these circumstances:
/// - spawning command fails
/// - joining fails
/// - the returned path cannot be canonicalized
pub fn spawn_setup_command(command: &str, shell: &str) -> SpawnSetupResult {
    use error_stack::ResultExt as _;

    let mut child = {
        let mut cmd = tokio::process::Command::new(shell);
        cmd.arg("-c")
            .arg(command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Place the child in its own process group on Unix so that
        // `kill_process_tree` can atomically signal the whole group with
        // `kill(-pgid)`. Windows ignores this (it has no process-group
        // signalling analogue); its tree kill is enumerative via `kill_tree`.
        #[cfg(unix)]
        cmd.process_group(0);

        cmd.spawn()
            .change_context(LifecycleCommandError::ExecutionFailed)
            .attach("failed to spawn lifecycle command")?
    };

    // Capture the process-group id BEFORE moving the child into the reader task.
    // On Unix the child was spawned with `process_group(0)`, so its pid is also
    // its process-group id. The cancel path signals the whole group via
    // `kill_process_group_by_pid(pid)`, which needs no Child handle.
    let pid = child.id().expect("child has a pid immediately after spawn");

    // Take pipes before moving the child.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let handle = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;

        // Read stdout.
        let stdout_bytes = if let Some(mut pipe) = stdout_pipe {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf).await;
            buf
        } else {
            Vec::new()
        };

        // Read stderr.
        let stderr_bytes = if let Some(mut pipe) = stderr_pipe {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf).await;
            buf
        } else {
            Vec::new()
        };

        // Wait for exit. The reader owns the Child by value (no lock). When the
        // cancel path SIGKILLs the process group, wait() resolves with a
        // signal-killed status naturally.
        let status = child
            .wait()
            .await
            .change_context(LifecycleCommandError::ExecutionFailed)
            .attach("failed to wait for lifecycle command")?;

        let stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
        let stderr = String::from_utf8_lossy(&stderr_bytes).to_string();

        if !status.success() {
            return Err(Report::new(LifecycleCommandError::CommandFailed {
                exit_code: status.code(),
                stdout,
                stderr,
            }));
        }

        // A successful command may print no CWD (side-effect-only setups,
        // e.g. `sleep 3` to warm a cache). Treat that as success-without-cwd;
        // the caller advances lifecycle and keeps the default CWD.
        let last_line = stdout.lines().map(str::trim).rfind(|line| !line.is_empty());

        let canonical = match last_line {
            Some(last_line) => {
                let raw_path = PathBuf::from(last_line);

                let canonical = tokio::fs::canonicalize(&raw_path)
                    .await
                    .change_context(LifecycleCommandError::InvalidPath {
                        path: raw_path.clone(),
                    })
                    .attach("setup command output is not a valid path")?;

                if !canonical.is_dir() {
                    return Err(Report::new(LifecycleCommandError::NotADirectory {
                        path: canonical,
                    }));
                }

                canonical
            }
            None => return Ok(None),
        };

        Ok(Some(canonical))
    });

    let abort_handle = handle.abort_handle();
    Ok((LifecycleCancelHandle { pid, abort_handle }, handle))
}

/// Spawns a teardown command and returns a shared child handle and a joinable task.
///
/// Same pattern as [`spawn_setup_command`] but only checks the exit code.
///
/// # Panics
///
/// Panics if the spawned child has no PID immediately after spawn (should never happen for a process group child).
///
/// # Errors
///
/// Returns an error if the shell command fails to spawn or canonicalize the working directory.
pub fn spawn_teardown_command(command: &str, shell: &str) -> SpawnTeardownResult {
    use error_stack::ResultExt as _;

    let mut child = {
        let mut cmd = tokio::process::Command::new(shell);
        cmd.arg("-c")
            .arg(command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Same process-group rationale as `spawn_setup_command` — see that
        // function for the Unix/Windows split.
        #[cfg(unix)]
        cmd.process_group(0);

        cmd.spawn()
            .change_context(LifecycleCommandError::ExecutionFailed)
            .attach("failed to spawn lifecycle command")?
    };

    // Capture the process-group id BEFORE moving the child into the reader task
    // (see spawn_setup_command for the rationale).
    let pid = child.id().expect("child has a pid immediately after spawn");

    // Take pipes before moving the child.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let handle = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;

        // Read stdout.
        let stdout_bytes = if let Some(mut pipe) = stdout_pipe {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf).await;
            buf
        } else {
            Vec::new()
        };

        // Read stderr.
        let stderr_bytes = if let Some(mut pipe) = stderr_pipe {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf).await;
            buf
        } else {
            Vec::new()
        };

        // Wait for exit (reader owns the Child by value; no lock). When the
        // cancel path SIGKILLs the process group, wait() resolves naturally.
        let status = child
            .wait()
            .await
            .change_context(LifecycleCommandError::ExecutionFailed)
            .attach("failed to wait for lifecycle command")?;

        if !status.success() {
            let stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
            let stderr = String::from_utf8_lossy(&stderr_bytes).to_string();
            return Err(Report::new(LifecycleCommandError::CommandFailed {
                exit_code: status.code(),
                stdout,
                stderr,
            }));
        }

        Ok(())
    });

    let abort_handle = handle.abort_handle();
    Ok((LifecycleCancelHandle { pid, abort_handle }, handle))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;

    // --- Setup command tests ---

    #[rstest::rstest]
    #[tokio::test]
    async fn setup_returns_canonicalized_directory_path() {
        // Given a command that echoes a directory path.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().to_string_lossy().to_string();
        let expected = std::fs::canonicalize(dir.path()).expect("canonicalize");

        // When running the setup command.
        let result = run_setup_command(&format!("echo {path}"), "/bin/sh").await;

        // Then the result is the canonicalized directory path.
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some(expected));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn setup_canonicalizes_last_non_empty_line() {
        // Given a command that outputs multiple lines.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().to_string_lossy().to_string();
        let expected = std::fs::canonicalize(dir.path()).expect("canonicalize");

        // When running the setup command with leading output.
        let result =
            run_setup_command(&format!("echo 'setting up...'; echo '{path}'"), "/bin/sh").await;

        // Then the result is the canonicalized last non-empty line.
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some(expected));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn setup_trims_whitespace_before_canonicalizing() {
        // Given a command that echoes with trailing whitespace.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().to_string_lossy().to_string();
        let expected = std::fs::canonicalize(dir.path()).expect("canonicalize");

        // When running the setup command.
        let result = run_setup_command(&format!("echo '{path}  '"), "/bin/sh").await;

        // Then the result is trimmed and canonicalized.
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some(expected));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn setup_returns_error_on_nonzero_exit() {
        // Given a command that exits with code 1.
        let result = run_setup_command("exit 1", "/bin/sh").await;

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
            "/bin/sh",
        )
        .await;

        // Then the error contains both stdout and stderr output.
        assert!(result.is_err());
        let report = result.unwrap_err();
        let err = report
            .downcast_ref::<LifecycleCommandError>()
            .expect("downcast");
        match err {
            LifecycleCommandError::CommandFailed { stdout, stderr, .. } => {
                assert!(stdout.contains("stdout message"));
                assert!(stderr.contains("stderr message"));
            }
            other => panic!("expected CommandFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn setup_returns_none_on_empty_stdout() {
        // Given a setup command that succeeds with no output (side-effect-only).
        let result = run_setup_command("true", "/bin/sh").await;

        // Then the result is Ok(None): success, but no CWD path to apply.
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn setup_canonicalizes_relative_path_to_absolute() {
        // Given a real subdirectory of the current working directory.
        let dir = tempfile::tempdir_in(".").expect("temp dir in cwd");
        let dir_name = dir
            .path()
            .file_name()
            .expect("dir name")
            .to_string_lossy()
            .to_string();
        let expected = std::fs::canonicalize(dir.path()).expect("canonicalize");

        // When running the setup command with a relative path.
        let result = run_setup_command(&format!("echo './{dir_name}'"), "/bin/sh").await;

        // Then the result is the canonicalized absolute path.
        assert!(
            result.is_ok(),
            "expected Ok, got Err: {:?}",
            result.unwrap_err()
        );
        assert_eq!(result.unwrap(), Some(expected));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn setup_returns_error_when_path_does_not_exist() {
        // Given a setup command that outputs a non-existent path.
        let result = run_setup_command("echo /nonexistent/path/xyzzy", "/bin/sh").await;

        // Then the result is an InvalidPath error.
        assert!(result.is_err());
        let report = result.unwrap_err();
        let err = report
            .downcast_ref::<LifecycleCommandError>()
            .expect("downcast");
        match err {
            LifecycleCommandError::InvalidPath { path } => {
                assert_eq!(path, &PathBuf::from("/nonexistent/path/xyzzy"));
            }
            other => panic!("expected InvalidPath, got {other:?}"),
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn setup_returns_error_when_path_is_not_a_directory() {
        // Given a setup command that outputs a file path (not a directory).
        let dir = tempfile::tempdir().expect("temp dir");
        let file_path = dir.path().join("some-file.txt");
        std::fs::write(&file_path, b"contents").expect("write file");

        // When running the setup command.
        let result = run_setup_command(&format!("echo '{}'", file_path.display()), "/bin/sh").await;

        // Then the result is a NotADirectory error.
        assert!(result.is_err());
        let report = result.unwrap_err();
        let err = report
            .downcast_ref::<LifecycleCommandError>()
            .expect("downcast");
        match err {
            LifecycleCommandError::NotADirectory { path } => {
                assert_eq!(
                    path,
                    &std::fs::canonicalize(&file_path).expect("canonicalize")
                );
            }
            other => panic!("expected NotADirectory, got {other:?}"),
        }
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn setup_canonicalizes_dot_to_actual_cwd() {
        // Given a setup command that outputs ".".
        let expected = std::fs::canonicalize(".").expect("canonicalize cwd");

        // When running the setup command.
        let result = run_setup_command("echo .", "/bin/sh").await;

        // Then the result is the canonicalized current working directory.
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some(expected));
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
        let result = run_teardown_command(&format!("rm -f {}", marker.display()), "/bin/sh").await;

        // Then the command succeeds (teardown doesn't require stdout).
        assert!(result.is_ok(), "teardown should succeed without stdout");
        // And the marker file is gone.
        assert!(!marker.exists(), "teardown should have removed the marker");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn teardown_returns_failure_on_nonzero_exit() {
        // Given a teardown command that fails.
        let result = run_teardown_command("exit 42", "/bin/sh").await;

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
        let _ = run_teardown_command(&cmd_a, "/bin/sh").await;
        let _ = run_teardown_command(&cmd_b, "/bin/sh").await;

        // Then both files are removed.
        assert!(!marker_a.exists(), "first teardown should remove marker a");
        assert!(!marker_b.exists(), "second teardown should remove marker b");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn teardown_succeeds_with_any_stdout() {
        // Given a teardown command that produces stdout but exits 0.
        let result = run_teardown_command("echo 'cleaning up...'", "/bin/sh").await;

        // Then the command succeeds (stdout is ignored).
        assert!(result.is_ok());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn aborting_reader_task_yields_cancelled_or_failed_outcome() {
        // Given a setup command that sleeps indefinitely.
        let (handle, join_handle) = spawn_setup_command("sleep 30", "/bin/sh").expect("spawn");

        // When killing the process group then aborting the inner reader task.
        crate::common::process_kill::kill_process_group_by_pid(handle.pid);
        handle.abort_handle.abort();

        // Then the join resolves to a failure outcome — either the inner task
        // observed the SIGKILL and returned CommandFailed (reaped before abort),
        // or the abort won and produced a JoinError. Either way: not success.
        let outcome = join_handle.await;
        let failed = !matches!(outcome, Ok(Ok(_)));
        // Outcome is timing-dependent (Edge Case #7): either the inner task
        // observed the SIGKILL and returned CommandFailed, or the abort won
        // and produced a JoinError. Both are valid "cancelled" outcomes.
        assert!(failed, "cancel must produce a non-success outcome");
    }

    #[test]
    fn cancel_handle_kills_and_aborts_without_panic() {
        // Given a setup command that sleeps, spawned on a multi-thread runtime
        // so the cancel runs on a WORKER thread (the original bug: blocking_lock
        // on a tokio worker thread panicked with SIGABRT).
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("multi-thread runtime");

        // When running the cancel sequence from inside a spawned task (i.e., on a
        // tokio worker thread). The spawn itself happens in runtime context so the
        // internally-spawned reader task can register with the reactor. If the cancel
        // panicked like the old code, the join below would surface it as a panic payload.
        let handle_cl = rt.handle().clone();
        rt.block_on(async move {
            let (handle, _join_handle) = spawn_setup_command("sleep 30", "/bin/sh").expect("spawn");

            // Cancel from a runtime worker thread, exactly as the session actor
            // does. The whole point of this test is that this does NOT panic.
            let cancel = handle_cl.spawn(async move {
                crate::common::process_kill::kill_process_group_by_pid(handle.pid);
                handle.abort_handle.abort();
            });
            cancel.await.expect("cancel task must not panic");
        });
        rt.shutdown_background();
    }
}
