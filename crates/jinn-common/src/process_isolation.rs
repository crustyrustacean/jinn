//! Terminal isolation for spawned child processes.
//!
//! Children inherit jinn's controlling terminal unless explicitly detached.
//! Programs that write to the terminal directly (`ssh`/`git` host-key prompts
//! → `/dev/tty` on Unix; console writes via `CONOUT$` on Windows) bypass
//! piped stdio and print over the TUI. [`isolate`] closes that channel.

/// Detaches `cmd`'s future child from jinn's controlling terminal.
///
/// - Unix: `setsid()` between fork and exec — the child becomes a session
///   and process-group leader (pid == pgid == sid) with **no controlling
///   terminal**. This supersedes `Command::process_group(0)`; never combine
///   the two (see below).
/// - Windows: `CREATE_NO_WINDOW` — console-subsystem children get their own
///   hidden console instead of attaching to jinn's. No-op for GUI-subsystem
///   binaries. Process-tree relationships (and thus `kill_tree`) are
///   unaffected.
///
/// Not combined with `Command::process_group(0)`: that makes the child a
/// group leader *before* exec, so a subsequent `setsid()` fails with `EPERM`
/// and the spawn aborts. Use one or the other; the kill path
/// (`kill(-pgid, SIGKILL)`) works identically under both because pid == pgid.
pub fn isolate(cmd: &mut std::process::Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: `isolate_unix` only calls `setsid(2)` — a raw syscall that
        // is async-signal-safe, as required of `pre_exec` closures.
        unsafe { cmd.pre_exec(isolate_unix) };
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

/// `pre_exec` hook: start a new session so the child has no controlling tty.
///
/// `setsid` fails with `EPERM` only if the caller is already a process-group
/// leader. A freshly forked child cannot be one unless the parent put it
/// there (i.e. `process_group(0)` — forbidden with this helper), so any
/// failure is a real error: abort the spawn rather than leak the tty.
#[cfg(unix)]
fn isolate_unix() -> std::io::Result<()> {
    // SAFETY: `setsid(2)` is a raw syscall with no preconditions beyond
    // the group-leader check documented above, which the caller (a
    // freshly forked pre-exec child) satisfies.
    if unsafe { libc::setsid() } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
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
    use std::process::Stdio;

    /// Builds a shell command with isolated stdio and terminal detachment,
    /// the standard shape used by every jinn spawn site.
    #[cfg(unix)]
    fn isolated_sh(script: &str) -> std::process::Command {
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        isolate(&mut cmd);
        cmd
    }

    #[cfg(unix)]
    #[rstest::rstest]
    #[test]
    fn isolation_child_has_no_controlling_tty() {
        // Given a script that tries to open the controlling tty.
        let script = "exec 3>/dev/tty";
        let mut cmd = isolated_sh(script);

        // When spawning it as an isolated child.
        let output = cmd.output().expect("spawn should succeed");

        // Then the child exited non-zero: opening /dev/tty must fail without
        // a controlling terminal.
        assert!(
            !output.status.success(),
            "expected /dev/tty open to fail in an isolated child; exit = {:?}",
            output.status.code()
        );
    }

    #[cfg(unix)]
    #[rstest::rstest]
    #[test]
    fn isolation_child_runs_in_new_session() {
        // Given an isolated child that stays alive long enough to inspect.
        let mut child = isolated_sh("sleep 5")
            .spawn()
            .expect("spawn should succeed");

        // When reading the session ids of the test process and the child
        // via libc. `ps` cannot be used here: procps has `sid`, BSD ps has
        // no working equivalent (`sess` reports 0), and `getsid(2)` needs
        // neither.
        // SAFETY: `getsid` reads a pid field and cannot fault; a dead child
        // pid yields ESRCH, which the mismatch assertion below catches.
        let own_sid = unsafe { libc::getsid(0) };
        // SAFETY: same as above; the child pid is freshly spawned and
        // reaped immediately after, and ESRCH on a raced exit is harmless
        // (the mismatch assertion reports it).
        let child_sid = unsafe { libc::getsid(child.id() as libc::pid_t) };
        let _ = child.kill();
        let _ = child.wait();

        // Then the child runs in a different session from the test process.
        assert_ne!(
            child_sid, own_sid,
            "isolated child must not share jinn's session"
        );
    }

    #[cfg(windows)]
    #[rstest::rstest]
    #[test]
    fn isolation_windows_child_completes_with_piped_output() {
        // Given an isolated cmd child writing to piped stdout.
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/C", "echo ok"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        isolate(&mut cmd);

        // When spawning it.
        let output = cmd.output().expect("spawn should succeed");

        // Then the output was captured and the child exited zero — the
        // creation flags do not break spawn or capture.
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("ok"));
    }
}
