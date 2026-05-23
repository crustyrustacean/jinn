//! Shared check helpers for bench task verification.

#![allow(clippy::missing_docs_in_private_items, reason = "helpers")]

use std::path::Path;

use crate::task::CheckResult;

/// Checks that a file exists in the given directory.
pub fn check_file_exists(dir: &Path, name: &str) -> CheckResult {
    let check_name = format!("file_exists({name})");
    let path = dir.join(name);
    if path.is_file() {
        tracing::info!(check = %check_name, "PASSED");
        CheckResult::pass(check_name)
    } else {
        let detail = format!("expected file to exist: {}", path.display());
        tracing::warn!(check = %check_name, %detail, "FAILED");
        CheckResult::fail(check_name, detail)
    }
}

/// Checks that a file in the given directory contains the specified string.
pub fn check_file_contains(dir: &Path, name: &str, needle: &str) -> CheckResult {
    let check_name = format!("file_contains({name}, {needle:?})");
    let content = std::fs::read_to_string(dir.join(name)).unwrap_or_default();
    if content.contains(needle) {
        tracing::info!(check = %check_name, "PASSED");
        CheckResult::pass(check_name)
    } else {
        let detail = format!(
            "expected {name} to contain {needle:?}, content length: {} bytes",
            content.len()
        );
        tracing::warn!(check = %check_name, %detail, "FAILED");
        CheckResult::fail(check_name, detail)
    }
}

/// Runs `cargo check` in the given directory.
pub fn check_cargo_check(dir: &Path) -> CheckResult {
    let output = std::process::Command::new("cargo")
        .args(["check"])
        .current_dir(dir)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            tracing::info!(check = "cargo_check", "PASSED");
            CheckResult::pass("cargo_check")
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let detail = format!(
                "cargo check failed (exit {:?}): {}",
                o.status.code(),
                stderr.trim()
            );
            tracing::warn!(check = "cargo_check", %detail, "FAILED");
            CheckResult::fail("cargo_check", detail)
        }
        Err(e) => {
            let detail = format!("failed to execute cargo check: {e}");
            tracing::warn!(check = "cargo_check", %detail, "FAILED");
            CheckResult::fail("cargo_check", detail)
        }
    }
}

/// Runs `cargo run` in the given directory.
pub fn check_cargo_run(dir: &Path) -> CheckResult {
    let output = std::process::Command::new("cargo")
        .args(["run"])
        .current_dir(dir)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            tracing::info!(check = "cargo_run", "PASSED");
            CheckResult::pass("cargo_run")
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let stdout = String::from_utf8_lossy(&o.stdout);
            let detail = format!(
                "cargo run failed (exit {:?})\nstdout: {}\nstderr: {}",
                o.status.code(),
                stdout.trim(),
                stderr.trim()
            );
            tracing::warn!(check = "cargo_run", %detail, "FAILED");
            CheckResult::fail("cargo_run", detail)
        }
        Err(e) => {
            let detail = format!("failed to execute cargo run: {e}");
            tracing::warn!(check = "cargo_run", %detail, "FAILED");
            CheckResult::fail("cargo_run", detail)
        }
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
        Ok(o) if o.status.success() => {
            tracing::info!(check = %check_name, "PASSED");
            CheckResult::pass(check_name)
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let stdout = String::from_utf8_lossy(&o.stdout);
            let detail = format!(
                "python3 {script} failed (exit {:?})\nstdout: {}\nstderr: {}",
                o.status.code(),
                stdout.trim(),
                stderr.trim()
            );
            tracing::warn!(check = %check_name, %detail, "FAILED");
            CheckResult::fail(check_name, detail)
        }
        Err(e) => {
            let detail = format!("failed to execute python3 {script}: {e}");
            tracing::warn!(check = %check_name, %detail, "FAILED");
            CheckResult::fail(check_name, detail)
        }
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
            let stderr = String::from_utf8_lossy(&o.stderr);
            let detail = format!(
                "cargo run failed (exit {:?}): {}",
                o.status.code(),
                stderr.trim()
            );
            tracing::warn!(check = %check_name, %detail, "FAILED");
            CheckResult::fail(check_name, detail)
        }
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            if stdout.contains(expected) {
                tracing::info!(check = %check_name, "PASSED");
                CheckResult::pass(check_name)
            } else {
                let detail = format!(
                    "expected stdout to contain {expected:?}\nactual stdout: {}",
                    stdout.trim()
                );
                tracing::warn!(check = %check_name, %detail, "FAILED");
                CheckResult::fail(check_name, detail)
            }
        }
        Err(e) => {
            let detail = format!("failed to execute cargo run: {e}");
            tracing::warn!(check = %check_name, %detail, "FAILED");
            CheckResult::fail(check_name, detail)
        }
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
            let stderr = String::from_utf8_lossy(&o.stderr);
            let detail = format!(
                "python3 {script} failed (exit {:?}): {}",
                o.status.code(),
                stderr.trim()
            );
            tracing::warn!(check = %check_name, %detail, "FAILED");
            CheckResult::fail(check_name, detail)
        }
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            if stdout.contains(expected) {
                tracing::info!(check = %check_name, "PASSED");
                CheckResult::pass(check_name)
            } else {
                let detail = format!(
                    "expected stdout to contain {expected:?}\nactual stdout: {}",
                    stdout.trim()
                );
                tracing::warn!(check = %check_name, %detail, "FAILED");
                CheckResult::fail(check_name, detail)
            }
        }
        Err(e) => {
            let detail = format!("failed to execute python3 {script}: {e}");
            tracing::warn!(check = %check_name, %detail, "FAILED");
            CheckResult::fail(check_name, detail)
        }
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
            let stderr = String::from_utf8_lossy(&o.stderr);
            let detail = format!(
                "cargo run failed (exit {:?}): {}",
                o.status.code(),
                stderr.trim()
            );
            tracing::warn!(check = "fizzbuzz_output", %detail, "FAILED");
            CheckResult::fail("fizzbuzz_output", detail)
        }
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            for line in stdout.lines() {
                if line.trim() == "FizzBuzz" {
                    tracing::info!(check = "fizzbuzz_output", "PASSED");
                    return CheckResult::pass("fizzbuzz_output");
                }
            }
            let detail = format!(
                "expected \"FizzBuzz\" as a standalone line\nactual stdout:\n{}",
                stdout.trim()
            );
            tracing::warn!(check = "fizzbuzz_output", %detail, "FAILED");
            CheckResult::fail("fizzbuzz_output", detail)
        }
        Err(e) => {
            let detail = format!("failed to execute cargo run: {e}");
            tracing::warn!(check = "fizzbuzz_output", %detail, "FAILED");
            CheckResult::fail("fizzbuzz_output", detail)
        }
    }
}
