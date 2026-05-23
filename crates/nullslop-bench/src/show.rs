//! `show` subcommand — render bench CSV as a formatted terminal table.

#![allow(clippy::print_stdout, reason = "CLI output")]
#![allow(clippy::cast_precision_loss, reason = "display formatting")]

use std::path::Path;

use comfy_table::{Cell, Table, presets::UTF8_FULL_CONDENSED};

use crate::csv::BenchResult;

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
