//! Shared check helpers for bench task verification.
//!
//! Each check function runs a verification step and returns a [`CheckResult`].
//! Subprocess output (stdout/stderr) is stripped of ANSI escape sequences to
//! ensure clean plain-text detail strings in session chat entries and CSV output.

#![allow(clippy::missing_docs_in_private_items, reason = "helpers")]

use std::path::Path;

use crate::task::CheckResult;
use strip_ansi_escapes::strip_str as strip_ansi;

/// Checks that a file exists in the given directory.
pub fn check_file_exists(dir: &Path, name: &str) -> CheckResult {
    let check_name = format!("file_exists({name})");
    let path = dir.join(name);
    if path.is_file() {
        CheckResult::pass(check_name)
    } else {
        CheckResult::fail(
            check_name,
            format!("expected file to exist: {}", path.display()),
        )
    }
}

/// Checks that a file in the given directory contains the specified string.
pub fn check_file_contains(dir: &Path, name: &str, needle: &str) -> CheckResult {
    let check_name = format!("file_contains({name}, {needle:?})");
    let content = std::fs::read_to_string(dir.join(name)).unwrap_or_default();
    if content.contains(needle) {
        CheckResult::pass(check_name)
    } else {
        CheckResult::fail(
            check_name,
            format!(
                "expected {name} to contain {needle:?}, content length: {} bytes",
                content.len()
            ),
        )
    }
}

/// Runs `cargo check` in the given directory.
pub fn check_cargo_check(dir: &Path) -> CheckResult {
    let output = std::process::Command::new("cargo")
        .args(["check"])
        .current_dir(dir)
        .output();

    match output {
        Ok(o) if o.status.success() => CheckResult::pass("cargo_check"),
        Ok(o) => {
            let stderr = strip_ansi(&String::from_utf8_lossy(&o.stderr));
            CheckResult::fail(
                "cargo_check",
                format!(
                    "cargo check failed (exit {:?}): {}",
                    o.status.code(),
                    stderr.trim()
                ),
            )
        }
        Err(e) => CheckResult::fail(
            "cargo_check",
            format!("failed to execute cargo check: {e}"),
        ),
    }
}

/// Runs `cargo run` in the given directory.
pub fn check_cargo_run(dir: &Path) -> CheckResult {
    let output = std::process::Command::new("cargo")
        .args(["run"])
        .current_dir(dir)
        .output();

    match output {
        Ok(o) if o.status.success() => CheckResult::pass("cargo_run"),
        Ok(o) => {
            let stderr = strip_ansi(&String::from_utf8_lossy(&o.stderr));
            let stdout = strip_ansi(&String::from_utf8_lossy(&o.stdout));
            CheckResult::fail(
                "cargo_run",
                format!(
                    "cargo run failed (exit {:?})\nstdout: {}\nstderr: {}",
                    o.status.code(),
                    stdout.trim(),
                    stderr.trim()
                ),
            )
        }
        Err(e) => CheckResult::fail(
            "cargo_run",
            format!("failed to execute cargo run: {e}"),
        ),
    }
}

/// Runs a Python script in the given directory.
pub fn check_python_run(dir: &Path, script: &str) -> CheckResult {
    let check_name = format!("python_run({script})");
    let output = std::process::Command::new("python3")
        .arg(script)
        .current_dir(dir)
        .output();

    match output {
        Ok(o) if o.status.success() => CheckResult::pass(check_name),
        Ok(o) => {
            let stderr = strip_ansi(&String::from_utf8_lossy(&o.stderr));
            let stdout = strip_ansi(&String::from_utf8_lossy(&o.stdout));
            CheckResult::fail(
                check_name,
                format!(
                    "python3 {script} failed (exit {:?})\nstdout: {}\nstderr: {}",
                    o.status.code(),
                    stdout.trim(),
                    stderr.trim()
                ),
            )
        }
        Err(e) => CheckResult::fail(
            check_name,
            format!("failed to execute python3 {script}: {e}"),
        ),
    }
}

/// Check that stdout from a `cargo run` contains an expected string.
pub fn check_cargo_run_contains(dir: &Path, expected: &str) -> CheckResult {
    let check_name = format!("cargo_run_contains({expected:?})");
    let output = std::process::Command::new("cargo")
        .args(["run"])
        .current_dir(dir)
        .output();

    match output {
        Ok(o) if !o.status.success() => {
            let stderr = strip_ansi(&String::from_utf8_lossy(&o.stderr));
            CheckResult::fail(
                check_name,
                format!(
                    "cargo run failed (exit {:?}): {}",
                    o.status.code(),
                    stderr.trim()
                ),
            )
        }
        Ok(o) => {
            let stdout = strip_ansi(&String::from_utf8_lossy(&o.stdout));
            if stdout.contains(expected) {
                CheckResult::pass(check_name)
            } else {
                CheckResult::fail(
                    check_name,
                    format!(
                        "expected stdout to contain {expected:?}\nactual stdout: {}",
                        stdout.trim()
                    ),
                )
            }
        }
        Err(e) => CheckResult::fail(
            check_name,
            format!("failed to execute cargo run: {e}"),
        ),
    }
}

/// Check that stdout from a `python3` run contains an expected string.
pub fn check_python_run_contains(dir: &Path, script: &str, expected: &str) -> CheckResult {
    let check_name = format!("python_run_contains({script}, {expected:?})");
    let output = std::process::Command::new("python3")
        .arg(script)
        .current_dir(dir)
        .output();

    match output {
        Ok(o) if !o.status.success() => {
            let stderr = strip_ansi(&String::from_utf8_lossy(&o.stderr));
            CheckResult::fail(
                check_name,
                format!(
                    "python3 {script} failed (exit {:?}): {}",
                    o.status.code(),
                    stderr.trim()
                ),
            )
        }
        Ok(o) => {
            let stdout = strip_ansi(&String::from_utf8_lossy(&o.stdout));
            if stdout.contains(expected) {
                CheckResult::pass(check_name)
            } else {
                CheckResult::fail(
                    check_name,
                    format!(
                        "expected stdout to contain {expected:?}\nactual stdout: {}",
                        stdout.trim()
                    ),
                )
            }
        }
        Err(e) => CheckResult::fail(
            check_name,
            format!("failed to execute python3 {script}: {e}"),
        ),
    }
}

/// Custom check for FizzBuzz: verify that stdout contains "FizzBuzz" as a
/// standalone line (not just as part of another word).
pub fn check_fizzbuzz_output(dir: &Path) -> CheckResult {
    let output = std::process::Command::new("cargo")
        .args(["run"])
        .current_dir(dir)
        .output();

    match output {
        Ok(o) if !o.status.success() => {
            let stderr = strip_ansi(&String::from_utf8_lossy(&o.stderr));
            CheckResult::fail(
                "fizzbuzz_output",
                format!(
                    "cargo run failed (exit {:?}): {}",
                    o.status.code(),
                    stderr.trim()
                ),
            )
        }
        Ok(o) => {
            let stdout = strip_ansi(&String::from_utf8_lossy(&o.stdout));
            for line in stdout.lines() {
                if line.trim() == "FizzBuzz" {
                    return CheckResult::pass("fizzbuzz_output");
                }
            }
            CheckResult::fail(
                "fizzbuzz_output",
                format!(
                    "expected \"FizzBuzz\" as a standalone line\nactual stdout:\n{}",
                    stdout.trim()
                ),
            )
        }
        Err(e) => CheckResult::fail(
            "fizzbuzz_output",
            format!("failed to execute cargo run: {e}"),
        ),
    }
}

/// Compares a file in the working directory against expected content byte-for-byte.
///
/// `expected` is typically provided via `include_str!` from an `expected/` directory
/// co-located with the task definition. The comparison is exact — trailing newlines,
/// whitespace, and encoding must all match.
pub fn check_snapshot(dir: &Path, name: &str, expected: &str) -> CheckResult {
    let check_name = format!("snapshot({name})");
    let actual = std::fs::read_to_string(dir.join(name)).unwrap_or_default();

    if actual == expected {
        CheckResult::pass(check_name)
    } else {
        let expected_lines: Vec<&str> = expected.lines().collect();
        let actual_lines: Vec<&str> = actual.lines().collect();

        let mut first_diff = None;
        for (i, (e, a)) in expected_lines.iter().zip(actual_lines.iter()).enumerate() {
            if e != a {
                first_diff = Some((i + 1, *e, *a));
                break;
            }
        }

        let detail = match first_diff {
            Some((line, exp, act)) => format!(
                "snapshot mismatch at line {line}\n  expected: {exp}\n  actual:   {act}\n  expected {} lines, got {} lines",
                expected_lines.len(),
                actual_lines.len()
            ),
            None => format!(
                "snapshot mismatch: expected {} lines, got {} lines",
                expected_lines.len(),
                actual_lines.len()
            ),
        };

        CheckResult::fail(check_name, detail)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]

    use super::*;

    #[test]
    fn check_snapshot_passes_when_content_matches() {
        // Given a temp directory with a file matching expected content.
        let root = tempfile::TempDir::new().expect("temp dir");
        std::fs::write(root.path().join("hello.txt"), "hello world\n").expect("write");

        // When checking snapshot.
        let result = check_snapshot(root.path(), "hello.txt", "hello world\n");

        // Then it passes.
        assert!(result.passed);
        assert_eq!(result.name, "snapshot(hello.txt)");
    }

    #[test]
    fn check_snapshot_fails_with_first_diff_when_content_mismatches() {
        // Given a temp directory with a file that differs on line 2.
        let root = tempfile::TempDir::new().expect("temp dir");
        std::fs::write(root.path().join("data.txt"), "line1\nwrong\nline3\n")
            .expect("write");

        // When checking snapshot against expected content.
        let result = check_snapshot(root.path(), "data.txt", "line1\nline2\nline3\n");

        // Then it fails with diff info about line 2.
        assert!(!result.passed);
        assert_eq!(result.name, "snapshot(data.txt)");
        assert!(
            result.detail.contains("line 2"),
            "detail should mention line 2: {}",
            result.detail
        );
        assert!(
            result.detail.contains("expected: line2"),
            "detail should show expected: {}",
            result.detail
        );
        assert!(
            result.detail.contains("actual:   wrong"),
            "detail should show actual: {}",
            result.detail
        );
    }
}
