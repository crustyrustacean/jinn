//! edit-large-replace-small-file bench task — rewrite procedural Python into class-based.

#![allow(clippy::missing_docs_in_private_items, reason = "task definition")]

use std::path::Path;
use std::time::Duration;

use crate::task::{BenchTask, BenchTools, CheckResult, VerificationReport};
use crate::tasks::checks;

pub fn task() -> BenchTask {
    BenchTask {
        name: "edit-large-replace-small-file",
        category: "edit",
        messages: vec![
            "Rewrite main.py as a proper class-based program. Create a `NumberProcessor` \
             class with methods: `load(data)` to set the numbers, `sum()` to return the sum, \
             `average()` to return the mean, `median()` to return the median, and `__str__` \
             for display. Keep the existing `main()` function but have it create a \
             `NumberProcessor` instance. Use the same data: [1, 5, 3, 9, 2, 7, 4, 8, 6].",
        ],
        fixture_dir: Some("src/tasks/edit/edit_large_replace_small_file/fixtures"),
        timeout: Duration::from_secs(300),
        persona: None,
        tools: BenchTools {
            builtins: vec!["read", "write"],
            custom: vec![],
        },
        verify,
    }
}

fn verify(dir: &Path) -> VerificationReport {
    let mut checks: Vec<CheckResult> = Vec::new();

    // Structural checks (AST) — verify the required class and methods exist.
    checks.push(checks::check_python_class_exists(
        dir,
        "main.py",
        "NumberProcessor",
    ));
    checks.extend(checks::check_python_class_has_methods(
        dir,
        "main.py",
        "NumberProcessor",
        &["__init__", "load", "sum", "average", "median", "__str__"],
    ));
    checks.push(checks::check_python_top_level_function_exists(
        dir,
        "main.py",
        "main",
    ));

    // Behavioral checks — run the program and verify output.
    checks.push(checks::check_python_run(dir, "main.py"));
    checks.push(checks::check_python_run_contains(
        dir,
        "main.py",
        "Numbers: [1, 5, 3, 9, 2, 7, 4, 8, 6]",
    ));
    checks.push(checks::check_python_run_contains(dir, "main.py", "Sum: 45"));

    VerificationReport::new("edit-large-replace-small-file", checks)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]

    use super::*;

    /// The expected correct output (from the original expected/main.py).
    const CORRECT_CODE: &str = include_str!("edit_large_replace_small_file/expected/main.py");

    /// Same program but with added docstrings, type hints, and comments.
    const CODE_WITH_EXTRAS: &str = r#"
"""Number processor module."""

from typing import List


class NumberProcessor:
    """Processes a list of numbers."""

    def __init__(self) -> None:
        """Initialize with empty list."""
        self.numbers: List[int] = []

    def load(self, data: List[int]) -> None:
        """Load data into the processor."""
        self.numbers = list(data)

    def sum(self) -> int:
        """Return the sum."""
        return sum(self.numbers)

    def average(self) -> float:
        """Return the average."""
        if not self.numbers:
            return 0.0
        return sum(self.numbers) / len(self.numbers)

    def median(self) -> float:
        """Return the median."""
        if not self.numbers:
            return 0.0
        sorted_nums = sorted(self.numbers)
        n = len(sorted_nums)
        mid = n // 2
        if n % 2 == 0:
            return (sorted_nums[mid - 1] + sorted_nums[mid]) / 2
        return float(sorted_nums[mid])

    def __str__(self) -> str:
        """Return string representation."""
        return (
            f"Numbers: {self.numbers}\n"
            f"Sum: {self.sum()}\n"
            f"Average: {self.average():.2f}\n"
            f"Median: {self.median():.1f}"
        )


def main() -> None:
    """Main entry point."""
    processor = NumberProcessor()
    processor.load([1, 5, 3, 9, 2, 7, 4, 8, 6])
    print(processor)


if __name__ == "__main__":
    main()
"#;

    /// The original fixture — no class, wrong structure.
    const WRONG_STRUCTURE: &str = include_str!("edit_large_replace_small_file/fixtures/main.py");

    fn write_and_verify(content: &str) -> VerificationReport {
        let dir = tempfile::TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join("main.py"), content).expect("write");
        verify(dir.path())
    }

    #[test]
    fn verify_passes_with_correct_code() {
        // Given the expected correct output.
        // When verifying.
        let report = write_and_verify(CORRECT_CODE);

        // Then all checks pass.
        for check in &report.checks {
            assert!(check.passed, "{} failed: {}", check.name, check.detail);
        }
        assert!(report.passed());
    }

    #[test]
    fn verify_passes_with_code_plus_docstrings_and_type_hints() {
        // Given correct code augmented with docstrings, type hints, and comments.
        // When verifying.
        let report = write_and_verify(CODE_WITH_EXTRAS);

        // Then all checks pass — extras are ignored.
        for check in &report.checks {
            assert!(check.passed, "{} failed: {}", check.name, check.detail);
        }
        assert!(report.passed());
    }

    #[test]
    fn verify_reports_structural_failures_with_wrong_structure() {
        // Given the original fixture (no class, procedural code).
        // When verifying.
        let report = write_and_verify(WRONG_STRUCTURE);

        // Then structural checks fail.
        assert!(!report.passed());

        // And the report mentions the missing class.
        let class_check = report
            .checks
            .iter()
            .find(|c| c.name.contains("python_class_exists"))
            .expect("class check");
        assert!(!class_check.passed);
        assert!(
            class_check.detail.contains("NumberProcessor not found"),
            "detail: {}",
            class_check.detail
        );

        // And method checks also report the missing class.
        let method_check = report
            .checks
            .iter()
            .find(|c| c.name.contains("method_exists"))
            .expect("method check");
        assert!(!method_check.passed);
    }

    #[test]
    fn verify_always_runs_all_checks() {
        // Given the wrong structure (no class).
        // When verifying.
        let report = write_and_verify(WRONG_STRUCTURE);

        // Then all checks ran regardless of earlier failures.
        // 1 class_exists + 1 method_exists (class-missing shortcut) + 1 function_exists
        // + 1 python_run + 2 python_run_contains = 6
        // Note: class_has_methods returns 1 result when class is missing (not 6 per-method),
        // but all other checks still run.
        assert_eq!(report.checks.len(), 6);

        // And behavioral checks ran despite structural failures.
        let run_check = report
            .checks
            .iter()
            .find(|c| c.name.contains("python_run("))
            .expect("run check");
        assert!(run_check.passed, "python_run should pass even with wrong structure");
    }

    #[test]
    fn verify_produces_clear_diagnostic_messages() {
        // Given the wrong structure.
        // When verifying.
        let report = write_and_verify(WRONG_STRUCTURE);

        // Then the class check mentions the class name and file.
        let class_check = report
            .checks
            .iter()
            .find(|c| c.name.contains("python_class_exists"))
            .expect("class check");
        assert!(class_check.name.contains("NumberProcessor"));
        assert!(class_check.name.contains("main.py"));
    }
}
