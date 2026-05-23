//! `show` subcommand — render bench CSV as a formatted terminal table.

#![allow(clippy::print_stdout, reason = "CLI output")]
#![allow(clippy::cast_precision_loss, reason = "display formatting")]

use std::collections::HashMap;
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

/// Groups results by model and computes per-model and grand-total summaries.
fn summarize(results: &[BenchResult]) -> (Vec<(String, BenchSummary)>, BenchSummary) {
    let mut map: HashMap<String, BenchSummary> = HashMap::new();

    for result in results {
        map.entry(result.model.clone())
            .or_default()
            .add(result);
    }

    let mut per_model: Vec<_> = map.into_iter().collect();
    per_model.sort_by(|a, b| a.0.cmp(&b.0));

    let grand = per_model
        .iter()
        .fold(BenchSummary::default(), |mut acc, (_, summary)| {
            acc.tasks += summary.tasks;
            acc.turns += summary.turns;
            acc.tokens_in += summary.tokens_in;
            acc.tokens_out += summary.tokens_out;
            acc.cost += summary.cost;
            acc.wall_time_ms += summary.wall_time_ms;
            acc.passed_count += summary.passed_count;
            acc.failed_count += summary.failed_count;
            acc.timeout_count += summary.timeout_count;
            acc
        });

    (per_model, grand)
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

    let (per_model, grand) = summarize(&results);
    print_summary_table(&per_model, &grand);
    print_grand_total_line(&grand);

    Ok(())
}

/// Formats milliseconds as a human-readable duration string (for individual rows).
fn format_duration(ms: u64) -> String {
    let secs = f64::from(u32::try_from(ms).unwrap_or(u32::MAX)) / 1000.0;
    format!("{secs:.1}s")
}

/// Formats milliseconds as a human-readable duration string (handles large totals).
fn format_duration_ms(ms: u64) -> String {
    let duration = std::time::Duration::from_millis(ms);
    humantime::format_duration(duration).to_string()
}

/// Formats a token count as a human-readable string (e.g. "15.1k", "1.2M").
fn format_tokens_human(count: u64) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

/// Renders the per-model summary table and grand-total row.
fn print_summary_table(per_model: &[(String, BenchSummary)], grand: &BenchSummary) {
    if per_model.is_empty() {
        println!("\nNo results to summarize.");
        return;
    }

    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(vec![
        "Model",
        "Tasks",
        "Turns",
        "Tokens \u{2191}",
        "Tokens \u{2193}",
        "Cost",
        "Wall Time",
        "Avg Time",
        "Passed",
        "Failed",
        "Timeout",
        "Pass Rate",
    ]);

    for (model, s) in per_model {
        table.add_row(summary_row(model, s));
    }

    table.add_row(summary_row("TOTAL", grand));

    println!("\nSummary:");
    println!("{table}");
}

/// Builds a single table row from a label and summary.
fn summary_row(label: &str, s: &BenchSummary) -> Vec<Cell> {
    let rate = match s.pass_rate() {
        Some(r) => format!("{r:.1}%"),
        None => "N/A".to_owned(),
    };
    let avg = match s.avg_time_ms() {
        Some(ms) => format_duration_ms(ms),
        None => "N/A".to_owned(),
    };
    vec![
        Cell::new(label),
        Cell::new(s.tasks),
        Cell::new(s.turns),
        Cell::new(format_tokens_human(s.tokens_in)),
        Cell::new(format_tokens_human(s.tokens_out)),
        Cell::new(format!("${:.4}", s.cost)),
        Cell::new(format_duration_ms(s.wall_time_ms)),
        Cell::new(avg),
        Cell::new(s.passed_count),
        Cell::new(s.failed_count),
        Cell::new(s.timeout_count),
        Cell::new(rate),
    ]
}

/// Prints the grand-total summary as a single text line.
fn print_grand_total_line(grand: &BenchSummary) {
    if grand.tasks == 0 {
        return;
    }

    let rate = grand
        .pass_rate()
        .map_or_else(|| "N/A".to_owned(), |r| format!("{r:.1}%"));

    let avg = grand
        .avg_time_ms()
        .map_or_else(|| "N/A".to_owned(), format_duration_ms);

    println!(
        "TOTAL: {} tasks | {} passed, {} failed, {} timeout ({}) | {} turns | {} \u{2191} / {} \u{2193} | ${:.4} | {} total, {} avg",
        grand.tasks,
        grand.passed_count,
        grand.failed_count,
        grand.timeout_count,
        rate,
        grand.turns,
        grand.tokens_in,
        grand.tokens_out,
        grand.cost,
        format_duration_ms(grand.wall_time_ms),
        avg,
    );
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

    fn make_result_with_model(model: &str, passed: bool, status: &str) -> BenchResult {
        BenchResult {
            name: "test".to_owned(),
            category: "one_shot".to_owned(),
            model: model.to_owned(),
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
    fn summarize_returns_empty_when_no_results() {
        // Given no results.
        let results: Vec<BenchResult> = Vec::new();

        // When summarizing.
        let (per_model, grand) = summarize(&results);

        // Then per_model is empty and grand total is zeroed.
        assert!(per_model.is_empty());
        assert_eq!(grand.tasks, 0);
    }

    #[test]
    fn summarize_groups_by_model() {
        // Given results from two models.
        let results = vec![
            make_result_with_model("bravo/model-b", true, "completed"),
            make_result_with_model("alpha/model-a", true, "completed"),
            make_result_with_model("alpha/model-a", false, "completed"),
        ];

        // When summarizing.
        let (per_model, grand) = summarize(&results);

        // Then we get two sorted groups with correct counts.
        assert_eq!(per_model.len(), 2);
        let alpha = per_model.iter().find(|(n, _)| n == "alpha/model-a").unwrap();
        let bravo = per_model.iter().find(|(n, _)| n == "bravo/model-b").unwrap();
        // Alpha sorts before bravo.
        assert!(alpha.0 < bravo.0);
        assert_eq!(alpha.1.tasks, 2);
        assert_eq!(alpha.1.passed_count, 1);
        assert_eq!(alpha.1.failed_count, 1);
        assert_eq!(bravo.1.tasks, 1);
        assert_eq!(bravo.1.passed_count, 1);

        // And grand total matches.
        assert_eq!(grand.tasks, 3);
        assert_eq!(grand.passed_count, 2);
        assert_eq!(grand.failed_count, 1);
    }

    #[test]
    fn summarize_single_model_grand_matches_per_model() {
        // Given results from one model.
        let results = vec![
            make_result_with_model("test-model", true, "completed"),
            make_result_with_model("test-model", false, "timeout"),
        ];

        // When summarizing.
        let (per_model, grand) = summarize(&results);

        // Then grand total equals the single per-model summary.
        assert_eq!(per_model.len(), 1);
        let model = &per_model.first().unwrap().1;
        assert_eq!(grand.tasks, model.tasks);
        assert_eq!(grand.passed_count, model.passed_count);
        assert_eq!(grand.timeout_count, model.timeout_count);
    }

    #[test]
    fn summarize_classifies_mixed_pass_fail_timeout() {
        // Given a mix of passed, failed, and timeout results across models.
        let results = vec![
            make_result_with_model("model-a", true, "completed"),
            make_result_with_model("model-a", false, "completed"),
            make_result_with_model("model-a", false, "timeout"),
            make_result_with_model("model-b", true, "completed"),
            make_result_with_model("model-b", false, "timeout"),
        ];

        // When summarizing.
        let (per_model, _grand) = summarize(&results);

        // Then model-a has 1 passed, 1 failed, 1 timeout.
        let model_a = &per_model
            .iter()
            .find(|(name, _)| name == "model-a")
            .unwrap()
            .1;
        assert_eq!(model_a.passed_count, 1);
        assert_eq!(model_a.failed_count, 1);
        assert_eq!(model_a.timeout_count, 1);

        // And model-b has 1 passed, 0 failed, 1 timeout.
        let model_b = &per_model
            .iter()
            .find(|(name, _)| name == "model-b")
            .unwrap()
            .1;
        assert_eq!(model_b.passed_count, 1);
        assert_eq!(model_b.failed_count, 0);
        assert_eq!(model_b.timeout_count, 1);
    }

    #[test]
    fn pass_rate_computes_percentage() {
        // Given a summary with 2 passed out of 4 tasks.
        let summary = BenchSummary {
            tasks: 4,
            passed_count: 2,
            ..Default::default()
        };

        // When computing pass rate.
        let rate = summary.pass_rate().unwrap();

        // Then it is 50%.
        assert!((rate - 50.0).abs() < 0.001);
    }
}
