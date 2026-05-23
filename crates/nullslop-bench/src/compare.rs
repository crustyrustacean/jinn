//! `compare` subcommand — diff two bench CSVs and show deltas.

#![allow(clippy::print_stdout, reason = "CLI output")]
#![allow(clippy::cast_precision_loss, reason = "display formatting")]
#![allow(clippy::float_cmp, reason = "delta comparison")]

use std::collections::HashMap;
use std::path::Path;

use comfy_table::{Cell, Color, Table, presets::UTF8_FULL_CONDENSED};

use crate::csv::BenchResult;
use crate::show::read_csv;

/// Compares two bench CSVs and renders a diff table.
///
/// # Errors
///
/// Returns an error if either file cannot be read or parsed.
pub fn compare_results(csv_a: &Path, csv_b: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let results_a = read_csv(csv_a)?;
    let results_b = read_csv(csv_b)?;

    let map_a: HashMap<(String, String), &BenchResult> = results_a
        .iter()
        .map(|r| ((r.name.clone(), r.model.clone()), r))
        .collect();
    let map_b: HashMap<(String, String), &BenchResult> = results_b
        .iter()
        .map(|r| ((r.name.clone(), r.model.clone()), r))
        .collect();

    // Collect all unique keys, sorted for stable output.
    let mut keys: Vec<_> = map_a.keys().chain(map_b.keys()).collect();
    keys.sort();
    keys.dedup();

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

    for key in keys {
        let a = map_a.get(key);
        let b = map_b.get(key);

        let (name, model) = key;

        match (a, b) {
            (Some(a), Some(b)) => {
                let turns_cell = diff_cell(a.turns, b.turns, DiffStyle::I64);
                let tokens_in_cell = diff_cell(a.tokens_in, b.tokens_in, DiffStyle::U64);
                let tokens_out_cell = diff_cell(a.tokens_out, b.tokens_out, DiffStyle::U64);
                let cost_cell = diff_cell_f64(a.cost, b.cost);
                let time_cell = diff_cell_ms(a.wall_time_ms, b.wall_time_ms);
                let passed_cell = if a.passed == b.passed {
                    Cell::new(if a.passed { "\u{2713}" } else { "\u{2717}" })
                } else {
                    let text = format!(
                        "{}\u{2192}{}",
                        if a.passed { "\u{2713}" } else { "\u{2717}" },
                        if b.passed { "\u{2713}" } else { "\u{2717}" }
                    );
                    Cell::new(text).fg(Color::Yellow)
                };
                let status_cell = Cell::new(&b.status);

                table.add_row(vec![
                    Cell::new(name),
                    Cell::new(&b.category),
                    Cell::new(model),
                    turns_cell,
                    tokens_in_cell,
                    tokens_out_cell,
                    cost_cell,
                    time_cell,
                    passed_cell,
                    status_cell,
                ]);
            }
            (None, Some(b)) => {
                table.add_row(vec![
                    Cell::new(name),
                    Cell::new(&b.category),
                    Cell::new(model),
                    Cell::new("NEW").fg(Color::Green),
                    Cell::new(b.tokens_in),
                    Cell::new(b.tokens_out),
                    Cell::new(format!("${:.4}", b.cost)),
                    Cell::new(format_duration(b.wall_time_ms)),
                    Cell::new(if b.passed { "\u{2713}" } else { "\u{2717}" }),
                    Cell::new(&b.status),
                ]);
            }
            (Some(a), None) => {
                table.add_row(vec![
                    Cell::new(name),
                    Cell::new(&a.category),
                    Cell::new(model),
                    Cell::new("REMOVED").fg(Color::Red),
                    Cell::new(a.tokens_in),
                    Cell::new(a.tokens_out),
                    Cell::new(format!("${:.4}", a.cost)),
                    Cell::new(format_duration(a.wall_time_ms)),
                    Cell::new(if a.passed { "\u{2713}" } else { "\u{2717}" }),
                    Cell::new(&a.status),
                ]);
            }
            (None, None) => {
                // unreachable — keys come from the union of both maps
            }
        }
    }

    println!("{table}");
    Ok(())
}

/// Style hint for diff formatting.
enum DiffStyle {
    /// Signed 64-bit integer diff.
    I64,
    /// Unsigned 64-bit integer diff.
    U64,
}

/// Creates a cell showing the delta between two integer values.
fn diff_cell<T>(a: T, b: T, _style: DiffStyle) -> Cell
where
    T: std::ops::Sub<Output = T> + PartialOrd + Copy + std::fmt::Display,
{
    if b > a {
        let d = b - a;
        Cell::new(format!("+{d}")).fg(Color::Red)
    } else if b < a {
        let d = a - b;
        Cell::new(format!("-{d}")).fg(Color::Green)
    } else {
        Cell::new("0")
    }
}

/// Creates a cell showing the delta between two f64 values (cost).
fn diff_cell_f64(a: f64, b: f64) -> Cell {
    let delta = b - a;
    if delta > 0.0001 {
        Cell::new(format!("+${delta:.4}")).fg(Color::Red)
    } else if delta < -0.0001 {
        Cell::new(format!("-${:.4}", delta.abs())).fg(Color::Green)
    } else {
        Cell::new("$0.0000")
    }
}

/// Creates a cell showing the delta between two millisecond durations.
fn diff_cell_ms(a: u64, b: u64) -> Cell {
    match b.cmp(&a) {
        std::cmp::Ordering::Greater => {
            let d = b - a;
            Cell::new(format!("+{d}ms")).fg(Color::Red)
        }
        std::cmp::Ordering::Less => {
            let d = a - b;
            Cell::new(format!("-{d}ms")).fg(Color::Green)
        }
        std::cmp::Ordering::Equal => Cell::new("0ms"),
    }
}

/// Formats milliseconds as a human-readable duration string.
fn format_duration(ms: u64) -> String {
    let secs = f64::from(u32::try_from(ms).unwrap_or(u32::MAX)) / 1000.0;
    format!("{secs:.1}s")
}
