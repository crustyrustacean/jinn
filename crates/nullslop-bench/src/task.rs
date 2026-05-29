//! Bench task definitions — what to run and how to verify results.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::time::Duration;

use nullslop_domain::feat::tools_actor::tool_types::ToolContext;
use nullslop_provider::ToolDefinition;

/// A boxed future returned by tool execute functions.
pub type BoxedToolFuture = Pin<Box<dyn Future<Output = nullslop_provider::ToolResult> + Send>>;

/// The outcome of a single verification check.
#[derive(Debug, Clone)]
pub struct CheckResult {
    /// Short name for the check (e.g., "file_exists(src/main.rs)", "cargo_check").
    pub name: String,
    /// Whether the check passed.
    pub passed: bool,
    /// Human-readable detail describing what was expected vs what was found.
    /// Empty on success.
    pub detail: String,
}

impl CheckResult {
    /// Create a passing check.
    pub fn pass<N>(name: N) -> Self
    where
        N: Into<String>,
    {
        Self {
            name: name.into(),
            passed: true,
            detail: String::new(),
        }
    }

    /// Create a failing check with a detail message.
    pub fn fail<N, D>(name: N, detail: D) -> Self
    where
        N: Into<String>,
        D: Into<String>,
    {
        Self {
            name: name.into(),
            passed: false,
            detail: detail.into(),
        }
    }
}

/// The result of running a verification function.
///
/// Contains individual [`CheckResult`] entries so that failures can be
/// traced, logged, and surfaced to the user.
#[derive(Debug, Clone)]
pub struct VerificationReport {
    /// The task name this report is for.
    pub task: String,
    /// Individual check results.
    pub checks: Vec<CheckResult>,
}

impl VerificationReport {
    /// Create a new report for the given task.
    pub fn new<T>(task: T, checks: Vec<CheckResult>) -> Self
    where
        T: Into<String>,
    {
        Self {
            task: task.into(),
            checks,
        }
    }

    /// Returns `true` if all checks passed.
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|c| c.passed)
    }

    /// Returns an iterator over only the failing checks.
    pub fn failures(&self) -> impl Iterator<Item = &CheckResult> {
        self.checks.iter().filter(|c| !c.passed)
    }
}

/// A single benchmark task definition.
#[derive(Debug, Clone)]
pub struct BenchTask {
    /// Human-readable task name (used in CSV, fixture paths, progress output).
    pub name: &'static str,
    /// Bench category (e.g., "one_shot", "fix_code", "redirect").
    pub category: &'static str,
    /// Messages to send sequentially. Each message waits for `PhaseKind::Idle`
    /// before sending the next.
    pub messages: Vec<&'static str>,
    /// Fixture directory embedded in the binary.
    /// `None` means an empty working directory.
    pub fixture_dir: Option<&'static include_dir::Dir<'static>>,
    /// Per-task timeout (total wall time for all messages).
    pub timeout: Duration,
    /// Persona name to activate before running. `None` = default persona.
    pub persona: Option<&'static str>,
    /// Which tools to make available for this task.
    pub tools: BenchTools,
    /// Verification function run against the fixture directory after completion.
    /// Returns a [`VerificationReport`] with per-check results.
    pub verify: fn(&Path) -> VerificationReport,
}

/// Tool configuration for a bench task.
#[derive(Debug, Clone)]
pub struct BenchTools {
    /// Subset of built-in tool names to register (e.g., `["bash", "read", "write"]`).
    /// Empty means all built-in tools are registered.
    pub builtins: Vec<&'static str>,
    /// Additional custom tools with their definitions and execute functions.
    pub custom: Vec<CustomTool>,
}

/// A custom tool provided by a bench task.
#[derive(Debug, Clone)]
pub struct CustomTool {
    /// The tool's JSON-schema definition.
    pub definition: ToolDefinition,
    /// The function that executes the tool call.
    pub execute: fn(nullslop_provider::ToolCall, ToolContext) -> BoxedToolFuture,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]
    #![allow(clippy::get_first, reason = "test code")]

    use super::*;

    #[test]
    fn report_passed_returns_true_when_all_checks_pass() {
        // Given a report with 3 passing checks.
        let report = VerificationReport::new(
            "test",
            vec![
                CheckResult::pass("a"),
                CheckResult::pass("b"),
                CheckResult::pass("c"),
            ],
        );

        // Then passed() returns true.
        assert!(report.passed());
    }

    #[test]
    fn report_passed_returns_false_when_any_check_fails() {
        // Given a report with 2 passing and 1 failing check.
        let report = VerificationReport::new(
            "test",
            vec![
                CheckResult::pass("a"),
                CheckResult::fail("b", "something went wrong"),
                CheckResult::pass("c"),
            ],
        );

        // Then passed() returns false.
        assert!(!report.passed());
    }

    #[test]
    fn report_failures_returns_only_failed_checks() {
        // Given a report with mixed results.
        let report = VerificationReport::new(
            "test",
            vec![
                CheckResult::pass("a"),
                CheckResult::fail("b", "fail 1"),
                CheckResult::pass("c"),
                CheckResult::fail("d", "fail 2"),
            ],
        );

        // Then failures() returns only the failed checks.
        let failures: Vec<&CheckResult> = report.failures().collect();
        assert_eq!(failures.len(), 2);
        assert_eq!(failures.get(0).expect("first failure").name, "b");
        assert_eq!(failures.get(1).expect("second failure").name, "d");
    }

    #[test]
    fn check_result_pass_has_empty_detail() {
        // Given a passing check.
        let check = CheckResult::pass("file_exists(src/main.rs)");

        // Then passed is true and detail is empty.
        assert!(check.passed);
        assert!(check.detail.is_empty());
    }

    #[test]
    fn check_result_fail_has_detail() {
        // Given a failing check.
        let check = CheckResult::fail("cargo_check", "exit code 1: expected `Cargo.toml`");

        // Then passed is false and detail describes the failure.
        assert!(!check.passed);
        assert_eq!(check.detail, "exit code 1: expected `Cargo.toml`");
    }
}
