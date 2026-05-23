//! `show` subcommand — render bench CSV as a formatted terminal table.

#![allow(clippy::print_stdout, reason = "CLI output")]
#![allow(clippy::cast_precision_loss, reason = "display formatting")]

use std::path::Path;

use comfy_table::{Cell, Table, presets::UTF8_FULL_CONDENSED};

use crate::csv::BenchResult;

/// Aggregate statistics for a group of bench results (per-model or grand total).
#[derive(Debug, Clone, Default)]
struct BenchSummary {
    /// Number of tasks in this group.
    tasks: u64,
    /// Sum of turns across all tasks.
    turns: u32,
    /// Sum of input tokens across all tasks.
    tokens_in: u64,
    /// Sum of output tokens across all tasks.
    tokens_out: u64,
    /// Sum of cost (USD) across all tasks.
    cost: f64,
    /// Sum of wall time (ms) across all tasks.
    wall_time_ms: u64,
    /// Count of tasks where `passed == true`.
    passed_count: u64,
    /// Count of tasks where `passed == false && status != "timeout"`.
    failed_count: u64,
    /// Count of tasks where `status == "timeout"`.
    timeout_count: u64,
}

impl BenchSummary {
    /// Accumulates a single bench result into this summary.
    fn add(&mut self, result: &BenchResult) {
        self.tasks += 1;
        self.turns += result.turns;
        self.tokens_in += result.tokens_in;
        self.tokens_out += result.tokens_out;
        self.cost += result.cost;
        self.wall_time_ms += result.wall_time_ms;

        if result.passed {
            self.passed_count += 1;
        } else if result.status.eq_ignore_ascii_case("timeout") {
            self.timeout_count += 1;
        } else {
            self.failed_count += 1;
        }
    }

    /// Returns the pass rate as a percentage (0.0–100.0), or `None` if no tasks.
    fn pass_rate(&self) -> Option<f64> {
        if self.tasks == 0 {
            return None;
        }
        Some(
            f64::from(u32::try_from(self.passed_count).unwrap_or(u32::MAX))
                / f64::from(u32::try_from(self.tasks).unwrap_or(u32::MAX))
                * 100.0,
        )
    }

    /// Returns average wall time in ms, or `None` if no tasks.
    fn avg_time_ms(&self) -> Option<u64> {
        if self.tasks == 0 {
            return None;
        }
        Some(self.wall_time_ms / self.tasks)
    }
}

/// Reads a bench CSV and renders it as a formatted table.
///
/// # Errors
///
/// Returns an error if the file cannot be read or parsed.
pub fn show_results(csv_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let results = read_csv(csv_path)?;

    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(vec![
        "Task",
        "Category",
        "Model",
        "Turns",
        "Tokens \u{2191}",
        "Tokens \u{2193}",
        "Cost",
        "Time",
        "Passed",
        "Status",
    ]);

    for r in &results {
        let check = if r.passed { "\u{2713}" } else { "\u{2717}" };
        let time = format_duration(r.wall_time_ms);
        table.add_row(vec![
            Cell::new(&r.name),
            Cell::new(&r.category),
            Cell::new(&r.model),
            Cell::new(r.turns),
            Cell::new(r.tokens_in),
            Cell::new(r.tokens_out),
            Cell::new(format!("${:.4}", r.cost)),
            Cell::new(time),
            Cell::new(check),
            Cell::new(&r.status),
        ]);
    }

    println!("{table}");
    Ok(())
}

/// Formats milliseconds as a human-readable duration string.
fn format_duration(ms: u64) -> String {
    let secs = f64::from(u32::try_from(ms).unwrap_or(u32::MAX)) / 1000.0;
    format!("{secs:.1}s")
}

/// Reads a bench CSV file into a vector of results.
pub(crate) fn read_csv(path: &Path) -> Result<Vec<BenchResult>, Box<dyn std::error::Error>> {
    let mut reader = csv::Reader::from_path(path).map_err(|e| {
        format!("Failed to open '{}' as CSV: {e}", path.display())
    })?;
    let mut results = Vec::new();

    for record in reader.records() {
        let record = record.map_err(|e| {
            format!(
                "Failed to parse '{}' as CSV — is this actually a CSV file? {e}",
                path.display()
            )
        })?;
        let passed = record
            .get(8)
            .is_some_and(|v| v.eq_ignore_ascii_case("true"));
        results.push(BenchResult {
            name: record.get(0).unwrap_or_default().to_owned(),
            category: record.get(1).unwrap_or_default().to_owned(),
            model: record.get(2).unwrap_or_default().to_owned(),
            turns: record.get(3).unwrap_or_default().parse().unwrap_or(0),
            tokens_in: record.get(4).unwrap_or_default().parse().unwrap_or(0),
            tokens_out: record.get(5).unwrap_or_default().parse().unwrap_or(0),
            cost: record.get(6).unwrap_or_default().parse().unwrap_or(0.0),
            wall_time_ms: record.get(7).unwrap_or_default().parse().unwrap_or(0),
            passed,
            status: record.get(9).unwrap_or_default().to_owned(),
        });
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_result(passed: bool, status: &str) -> BenchResult {
        BenchResult {
            name: "test".to_owned(),
            category: "one_shot".to_owned(),
            model: "test-model".to_owned(),
            turns: 3,
            tokens_in: 100,
            tokens_out: 50,
            cost: 0.001,
            wall_time_ms: 5000,
            passed,
            status: status.to_owned(),
        }
    }

    #[test]
    fn add_classifies_passed_result() {
        // Given an empty summary.
        let mut summary = BenchSummary::default();

        // When adding a passed result.
        summary.add(&make_result(true, "completed"));

        // Then passed_count is incremented.
        assert_eq!(summary.tasks, 1);
        assert_eq!(summary.passed_count, 1);
        assert_eq!(summary.failed_count, 0);
        assert_eq!(summary.timeout_count, 0);
    }

    #[test]
    fn add_classifies_failed_result() {
        // Given an empty summary.
        let mut summary = BenchSummary::default();

        // When adding a failed (non-timeout) result.
        summary.add(&make_result(false, "completed"));

        // Then failed_count is incremented.
        assert_eq!(summary.tasks, 1);
        assert_eq!(summary.passed_count, 0);
        assert_eq!(summary.failed_count, 1);
        assert_eq!(summary.timeout_count, 0);
    }

    #[test]
    fn add_classifies_timeout_result() {
        // Given an empty summary.
        let mut summary = BenchSummary::default();

        // When adding a timeout result.
        summary.add(&make_result(false, "timeout"));

        // Then timeout_count is incremented.
        assert_eq!(summary.tasks, 1);
        assert_eq!(summary.passed_count, 0);
        assert_eq!(summary.failed_count, 0);
        assert_eq!(summary.timeout_count, 1);
    }

    #[test]
    fn add_accumulates_numeric_fields() {
        // Given an empty summary.
        let mut summary = BenchSummary::default();

        // When adding two results.
        summary.add(&make_result(true, "completed"));
        summary.add(&make_result(false, "completed"));

        // Then numeric fields are summed.
        assert_eq!(summary.tasks, 2);
        assert_eq!(summary.turns, 6);
        assert_eq!(summary.tokens_in, 200);
        assert_eq!(summary.tokens_out, 100);
        assert_eq!(summary.cost, 0.002);
        assert_eq!(summary.wall_time_ms, 10_000);
    }

    #[test]
    fn pass_rate_returns_none_when_no_tasks() {
        // Given an empty summary.
        let summary = BenchSummary::default();

        // When computing pass rate.
        // Then it returns None.
        assert!(summary.pass_rate().is_none());
    }

    #[test]
    fn pass_rate_computes_percentage() {
        // Given a summary with 2 passed out of 4 tasks.
        let mut summary = BenchSummary::default();
        summary.tasks = 4;
        summary.passed_count = 2;

        // When computing pass rate.
        let rate = summary.pass_rate().unwrap();

        // Then it is 50%.
        assert!((rate - 50.0).abs() < 0.001);
    }
}
