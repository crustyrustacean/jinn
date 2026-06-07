//! Cross-platform process-tree termination.
//!
//! Provides [`kill_process_tree`], a single entry point used by both the bash
//! tool's `KillOnDrop` guard and the session-lifecycle `kill_shared_child`
//! helper to terminate a spawned child _and its entire descendant tree_ on
//! cancel/timeout/abort.
//!
//! Each platform uses its most appropriate mechanism:
//!
//! - **Unix** — `libc::kill(-pgid, SIGKILL)`. Race-free and atomic: the kernel
//!   delivers the signal to every member of the child's process group in one
//!   syscall. Descendants spawned before the call are guaranteed to die.
//! - **Windows** — [`kill_tree`] enumeration via `CreateToolhelp32Snapshot`.
//!   See the Windows branch below for the documented TOCTOU trade-off.

/// Terminates `child` and its entire process tree.
///
/// Infallible by design: this is invoked from `Drop` impls and synchronous
/// cancel handlers where propagating an error is neither possible nor useful.
/// Failures (e.g. the process already exited) are logged at `warn` level and
/// otherwise swallowed. The helper performs **no async operations** and must
/// not depend on a tokio runtime context.
pub fn kill_process_tree(child: &mut tokio::process::Child) {
    // Read the PID once; if the child already exited there is nothing to do.
    let Some(pid) = child.id() else {
        return;
    };

    #[cfg(unix)]
    {
        kill_process_tree_unix(pid, child);
    }

    #[cfg(windows)]
    {
        kill_process_tree_windows(pid);
    }
}

/// Unix implementation: atomic process-group kill.
#[cfg(unix)]
fn kill_process_tree_unix(pid: u32, child: &mut tokio::process::Child) {
    // Belt-and-suspenders: terminate the direct child first.
    let _ = child.start_kill();

    // The child was spawned with `process_group(0)`, placing it in its own
    // process group whose pgid equals the child's pid. Signalling the negative
    // pgid instructs the kernel to deliver SIGKILL to the entire group,
    // atomically killing all descendants.
    //
    // SAFETY: `libc::kill` is safe when the PID argument is valid. The group
    // id is derived from `child.id()`, which is a real OS PID. The negative
    // argument targets the process group rather than a single process.
    let group_id = pid as libc::pid_t;
    if group_id > 0 {
        let _ = unsafe { libc::kill(-group_id, libc::SIGKILL) };
    }
}

/// Windows implementation: enumerative tree kill via [`kill_tree`].
#[cfg(windows)]
fn kill_process_tree_windows(pid: u32) {
    // TOCTOU RACE — ACCEPTED TRADE-OFF (Option B, see `.plans/windows-build-fix/plan.md`).
    //
    // `kill_tree` builds a point-in-time snapshot of the process tree via
    // `CreateToolhelp32Snapshot`, walks parent/child relationships, and calls
    // `TerminateProcess` on each descendant. Unlike the Unix branch (a single
    // atomic group signal), this is an enumerative walk: a grandchild that
    // spawns _between_ the snapshot and the per-process `TerminateProcess`
    // loop will NOT be signalled and can survive as an orphan.
    //
    // This race is accepted for this use case (an AI coding agent's bash tool
    // and session lifecycle commands) because the window is small and the
    // consequence — one orphaned background process — is mild. The atomic,
    // race-free alternative (Windows Job Objects with
    // `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) was considered and rejected as
    // Option C: it would require `windows-sys` FFI plus a
    // `CREATE_SUSPENDED` + `AssignProcessToJobObject` + `ResumeThread` dance
    // that `tokio::Command::spawn()` does not expose natively.
    let config = kill_tree::Config {
        signal: "SIGKILL".to_string(),
        ..Default::default()
    };
    if let Err(e) = kill_tree::blocking::kill_tree_with_config(pid, &config) {
        tracing::warn!(err = %e, pid, "process_kill: windows kill_tree failed");
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]
    #[cfg(unix)]
    use super::*;

    /// Calling the helper on an already-exited child must not panic. On Unix the
    /// process-group `kill(-pgid)` returns `ESRCH` (no such process) which is
    /// swallowed by `let _ =`; on Windows `kill_tree` surfaces a
    /// `MaybeAlready_terminated`-style error that is logged and swallowed.
    #[cfg(unix)]
    #[rstest::rstest]
    #[tokio::test]
    async fn kill_process_tree_handles_missing_pid() {
        // Given a child that has already exited (`true` returns immediately).
        let mut child = tokio::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("true")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn should succeed");
        // Await full exit and reap.
        let _ = child.wait().await;

        // When killing the (already-dead) process tree.
        // Then it returns without panicking.
        kill_process_tree(&mut child);
    }
}
