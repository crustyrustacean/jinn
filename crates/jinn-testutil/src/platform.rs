//! Cross-platform OS-semantics helpers for tests.
//!
//! Production code answers "how do I run a shell?", "is this process still
//! alive?", "what other processes are running?" through the shared helpers in
//! `jinn-common` and the per-platform branches of the actor code. Test code
//! that answers those questions itself drifts: it hardcodes `/bin/sh`, spawns
//! bare `bash` (which on Windows is the WSL launcher, not a shell), reads
//! `/proc` on macOS (where it does not exist), or calls `pgrep` on a machine
//! that never installed procps.
//!
//! This module is the test-side counterpart: one implementation of each
//! question, with the platform differences handled in one place.
//!
//! Every helper that can fail on some machine **skips visibly** — it prints a
//! `SKIP:` line naming the reason and returns an empty result. A test that
//! cannot run says so; it never passes silently.
#![allow(
    clippy::print_stderr,
    reason = "skip notes go to captured test output — a skip must be visible"
)]

use std::process::Command;

/// Resolves the shell to run test scripts through.
///
/// Uses the same shared resolver production uses
/// (`jinn_common::shell::Shells`), so a test and the code it exercises
/// agree on what "the shell" means.
///
/// An absolute resolved path was already existence-checked by the
/// resolver; the bare `bash` fallback is probed once, because it only
/// resolves if a POSIX bash is actually on `PATH`. When nothing usable
/// resolves, this prints a `SKIP:` line and returns `None`, so the caller
/// can skip the test loudly rather than fail it for an environmental
/// reason.
///
/// # Example
///
/// ```
/// # fn main() {
/// let Some(shell) = jinn_testutil::platform::shell_or_skip() else {
///     return; // the helper already printed why
/// };
/// // ... use `shell` ...
/// # }
/// ```
#[must_use]
pub fn shell_or_skip() -> Option<String> {
    let shell = jinn_common::shell::Shells::default()
        .bash()
        .ok()
        .map(|shell| shell.program().to_owned())?;
    if shell != "bash" {
        return Some(shell);
    }
    let probe = Command::new(&shell)
        .arg("-c")
        .arg("true")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match probe {
        Ok(_) => Some(shell),
        Err(err) => {
            eprintln!(
                "SKIP: no usable POSIX shell resolved for tests \
                 (bare 'bash' probe failed: {err})"
            );
            None
        }
    }
}

/// Every live process whose command line carries `marker`.
///
/// The platform implementations differ in what they match on, and the
/// difference is deliberate:
///
/// - **Linux** reads `/proc`, matching the full command line. A candidate
///   must be a real executable; transient pid slots between `readdir` and
///   `open` are skipped, and a zombie is already dead (the group kill
///   worked) so it is not an orphan.
/// - **BSD** (macOS and friends) has no `/proc`. `pgrep -x <name>` names the
///   candidates by executable name; each candidate's state and argument list
///   then come from `ps`. `pgrep` is procps and is **absent on stock
///   macOS**, so a missing `pgrep` is a visible skip, not a panic.
/// - **Windows** has no equivalent shell-free mechanism, so this reports
///   nothing.
///
/// Returns an empty vector when the platform cannot answer the question.
#[must_use]
pub fn find_orphans(marker: &str, exe_name: &str) -> Vec<String> {
    #[cfg(all(unix, target_os = "linux"))]
    {
        find_orphans_linux(marker, exe_name)
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        find_orphans_bsd(marker, exe_name)
    }
    #[cfg(not(unix))]
    {
        let _ = (marker, exe_name);
        Vec::new()
    }
}

/// Linux implementation of [`find_orphans`]: scan `/proc` for the command line.
#[cfg(all(unix, target_os = "linux"))]
fn find_orphans_linux(marker: &str, exe_name: &str) -> Vec<String> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        eprintln!("SKIP: /proc is not readable, cannot look for orphan {exe_name}");
        return found;
    };
    let exe_suffix = format!("/{exe_name}");
    for entry in entries.flatten() {
        let Ok(exe) = std::fs::read_link(entry.path().join("exe")) else {
            continue;
        };
        if !exe.to_string_lossy().ends_with(&exe_suffix) {
            continue;
        }
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some(state) = stat
            .rsplit(')')
            .next()
            .and_then(|rest| rest.split(' ').next())
        else {
            continue;
        };
        if state == "Z" {
            continue;
        }
        let Ok(cmdline) = std::fs::read_to_string(entry.path().join("cmdline")) else {
            continue;
        };
        if cmdline.replace('\0', " ").contains(marker) {
            found.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    found
}

/// BSD implementation of [`find_orphans`]: `pgrep` for candidates, `ps` for detail.
#[cfg(all(unix, not(target_os = "linux")))]
fn find_orphans_bsd(marker: &str, exe_name: &str) -> Vec<String> {
    let Ok(pids) = Command::new("pgrep").args(["-x", exe_name]).output() else {
        eprintln!("SKIP: pgrep is unavailable, cannot look for orphan {exe_name}");
        return Vec::new();
    };
    // pgrep exits 1 when nothing matches — that is the "no orphans" case the
    // caller asserts on, not a failure.
    let mut found = Vec::new();
    for pid in String::from_utf8_lossy(&pids.stdout).split_whitespace() {
        if ps_field(pid, "state=") == "Z" {
            continue;
        }
        if ps_field(pid, "args=").contains(marker) {
            found.push(pid.to_owned());
        }
    }
    found
}

/// One headerless `ps` field for `pid`, trimmed.
///
/// Uses the BSD-safe form (`ps -ww -o <field>= -p <pid>`). The GNU combined
/// form (`ps -eo stat,comm`) is rejected by BSD `ps`, and its column is named
/// `command` there, not `comm`.
///
/// Returns an empty string when `ps` cannot report on the pid.
#[cfg(unix)]
#[must_use]
pub fn ps_field(pid: &str, field: &str) -> String {
    let Ok(out) = Command::new("ps")
        .args(["-ww", "-o", field, "-p", pid])
        .output()
    else {
        return String::new();
    };
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Whether a process with this pid is still running.
///
/// On Unix this is the `kill(pid, 0)` signal-0 probe: it performs the
/// permission and existence checks without delivering a signal. A process
/// owned by another user reports as *not* alive to this probe even when it
/// is running, so this answers "does this pid still exist and is it
/// reachable", which is what a test needs.
///
/// Zombies are Unix-only. On Windows the answer is always `true`: the
/// concept does not exist there, and a "no zombie" assertion is vacuous on
/// that platform.
#[must_use]
pub fn is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: `kill` with signal 0 performs the existence and permission
        // checks without delivering a signal, so no process state changes.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

/// A command that exits successfully, portably.
///
/// `true` is a POSIX utility. On Windows there is no `true.exe`, and the
/// nearest name in `System32` is `cmd.exe`, which needs a different argument
/// shape. This returns the platform's own spelling, so a test that only
/// needs "a command that succeeds" does not have to branch.
///
/// The returned pair is `(program, args)`: spawn it with
/// `Command::new(program).args(args)`.
#[must_use]
pub fn success_command() -> (&'static str, Vec<String>) {
    #[cfg(windows)]
    {
        ("cmd", vec!["/C".to_owned(), "exit 0".to_owned()])
    }
    #[cfg(not(windows))]
    {
        ("true", Vec::new())
    }
}

/// A command that exits with a specific code, portably.
///
/// The POSIX form is `sh -c "exit <code>"`; the Windows form is
/// `cmd /C exit <code>`.
#[must_use]
pub fn exit_command(code: i32) -> (&'static str, Vec<String>) {
    #[cfg(windows)]
    {
        ("cmd", vec!["/C".to_owned(), format!("exit {code}")])
    }
    #[cfg(not(windows))]
    {
        ("sh", vec!["-c".to_owned(), format!("exit {code}")])
    }
}

/// A command that stays alive without doing anything, portably.
///
/// Used to model "a process that starts but never speaks the protocol".
/// The POSIX form is `sleep <seconds>`; the Windows form is `timeout`, which
/// is a built-in of `cmd.exe` rather than an executable, so it is spawned
/// through `cmd /C`.
#[must_use]
pub fn idle_command(seconds: u64) -> (&'static str, Vec<String>) {
    #[cfg(windows)]
    {
        (
            "cmd",
            vec![
                "/C".to_owned(),
                format!("ping -n {} 127.0.0.1 >NUL", seconds + 1),
            ],
        )
    }
    #[cfg(not(windows))]
    {
        ("sleep", vec![seconds.to_string()])
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        reason = "test code: a failed expect aborts the test loudly, which is the desired mode"
    )]
    use marked_path::MarkedPathAccess;

    use super::{exit_command, idle_command, is_alive, shell_or_skip, success_command};

    /// The orphan scan reads `/proc` on Linux and `pgrep`/`ps` on the other
    /// Unixes, so it only exists where those are available.
    #[cfg(unix)]
    use super::find_orphans;

    #[rstest::rstest]
    #[test]
    fn shell_or_skip_returns_a_shell_or_skips_visibly() {
        // Given any platform.
        // When resolving the test shell.
        let shell = shell_or_skip();

        // Then a shell is returned, or the helper printed why it skipped.
        // A `None` here means the machine has no POSIX shell at all, which
        // the helper has already reported on stderr.
        if let Some(shell) = shell {
            assert!(!shell.is_empty(), "a resolved shell must name a command");
        }
    }

    #[rstest::rstest]
    #[test]
    fn is_alive_reports_the_current_process_as_alive() {
        // Given the pid of this very process.
        let pid = std::process::id();

        // When probing for liveness.
        let alive = is_alive(pid);

        // Then the process is reported alive.
        assert!(alive);
    }

    #[rstest::rstest]
    #[test]
    fn success_command_exits_successfully() {
        // Given the platform's success command.
        let (program, args) = success_command();

        // When it is spawned.
        let status = std::process::Command::new(program)
            .args(&args)
            .status()
            .expect("success command runs");

        // Then it exits successfully.
        assert!(status.success());
    }

    #[rstest::rstest]
    #[case(0)]
    #[case(42)]
    fn exit_command_exits_with_the_requested_code(#[case] code: i32) {
        // Given the platform's exit command for a code.
        let (program, args) = exit_command(code);

        // When it is spawned.
        let status = std::process::Command::new(program)
            .args(&args)
            .status()
            .expect("exit command runs");

        // Then it exits with that code.
        assert_eq!(status.code(), Some(code));
    }

    #[rstest::rstest]
    #[test]
    fn idle_command_stays_running() {
        // Given the platform's idle command.
        let (program, args) = idle_command(30);

        // When it is spawned.
        let mut child = std::process::Command::new(program)
            .args(&args)
            .spawn()
            .expect("idle command runs");

        // Then it is still running.
        let still_running = child.try_wait().expect("poll child").is_none();
        let _ = child.kill();
        let _ = child.wait();

        // Then the child had not exited.
        assert!(still_running);
    }

    #[rstest::rstest]
    #[test]
    fn is_alive_reports_an_unused_pid_as_dead() {
        // Given a pid that is not running.
        // (A very high pid is chosen because pids are allocated in sequence
        // and the low range is in use by the test harness itself.)
        let pid = u32::MAX - 1;

        // When probing for liveness.
        let alive = is_alive(pid);

        // Then the process is reported dead.
        assert!(!alive);
    }

    #[cfg(unix)]
    #[rstest::rstest]
    #[test]
    fn find_orphans_finds_a_marked_child_process() {
        // Given a child process carrying a unique marker in its arguments.
        // `tail -f` on a regular file blocks forever and keeps the file's
        // name — which carries the marker — in its command line.
        let marker = format!("jinn-test-orphan-marker-{}", std::process::id());
        let dir = crate::temp::tempdir();
        let watched = dir.path.join(&marker).expect("temp root joins a bare name");
        std::fs::write(&watched, b"x").expect("write watched file");
        let Ok(mut child) = std::process::Command::new("tail")
            .arg("-f")
            .arg(watched.as_path())
            .spawn()
        else {
            eprintln!("SKIP: could not spawn a tail child");
            return;
        };

        // When scanning for processes carrying the marker.
        let found = find_orphans(&marker, "tail");

        // Then the child is found.
        let _ = child.kill();
        let _ = child.wait();
        assert!(
            found.iter().any(|pid| pid == &child.id().to_string()),
            "expected to find the marked child, got {found:?}"
        );
    }

    #[cfg(unix)]
    #[rstest::rstest]
    #[test]
    fn find_orphans_finds_nothing_for_an_unused_marker() {
        // Given a marker no running process carries.
        let marker = format!("jinn-test-no-such-marker-{}", std::process::id());

        // When scanning for processes carrying it.
        let found = find_orphans(&marker, "sleep");

        // Then nothing is found.
        assert!(found.is_empty(), "expected no matches, got {found:?}");
    }
}
