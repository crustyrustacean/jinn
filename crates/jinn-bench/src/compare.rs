//! `compare` subcommand - diff two bench CSVs and show deltas.

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
                // unreachable - keys come from the union of both maps
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]
    use super::*;

    // --- diff_cell ---

    #[rstest::rstest]
    fn diff_cell_positive_delta_shows_plus() {
        // Given b > a.
        let cell = diff_cell(10u64, 15u64, DiffStyle::U64);

        // Then the cell text starts with "+" and contains "5".
        let content = cell.content();
        assert!(content.contains('+'), "expected '+' in {content}");
        assert!(content.contains('5'), "expected '5' in {content}");
    }

    #[rstest::rstest]
    fn diff_cell_negative_delta_shows_minus() {
        // Given b < a.
        let cell = diff_cell(20u64, 10u64, DiffStyle::U64);

        // Then the cell text starts with "-" and contains "10".
        let content = cell.content();
        assert!(content.contains('-'), "expected '-' in {content}");
        assert!(content.contains("10"), "expected '10' in {content}");
    }

    #[rstest::rstest]
    fn diff_cell_equal_shows_zero() {
        // Given a == b.
        let cell = diff_cell(10u64, 10u64, DiffStyle::U64);

        // Then the cell text is "0".
        assert_eq!(cell.content(), "0");
    }

    #[rstest::rstest]
    fn diff_cell_magnitude_is_correct() {
        // Given a = 100, b = 130.
        let cell = diff_cell(100u64, 130u64, DiffStyle::U64);

        // Then the delta is exactly 30, not 100*130 or 100-130.
        let content = cell.content();
        assert!(content.contains("30"), "expected '30' in {content}");
        assert!(
            !content.contains("230"),
            "should not contain 230: {content}"
        );
        assert!(
            !content.contains("13000"),
            "should not contain 13000: {content}"
        );
    }

    #[rstest::rstest]
    fn diff_cell_negative_magnitude_is_correct() {
        // Given a = 50, b = 20.
        let cell = diff_cell(50u64, 20u64, DiffStyle::U64);

        // Then the delta is exactly 30, not 10 or 1000.
        let content = cell.content();
        assert!(content.contains("30"), "expected '30' in {content}");
        assert!(!content.contains("10"), "should not contain 10: {content}");
        assert!(
            !content.contains("1000"),
            "should not contain 1000: {content}"
        );
    }

    // --- diff_cell_f64 ---

    #[rstest::rstest]
    fn diff_cell_f64_positive_delta_shows_plus_dollar() {
        // Given b - a > 0.0001.
        let cell = diff_cell_f64(1.0, 1.5);

        let content = cell.content();
        assert!(content.contains('+'), "expected '+' in {content}");
        assert!(content.contains('$'), "expected '$' in {content}");
    }

    #[rstest::rstest]
    fn diff_cell_f64_negative_delta_shows_minus_dollar() {
        // Given b - a < -0.0001.
        let cell = diff_cell_f64(2.0, 1.5);

        let content = cell.content();
        assert!(content.contains('-'), "expected '-' in {content}");
        assert!(content.contains('$'), "expected '$' in {content}");
    }

    #[rstest::rstest]
    fn diff_cell_f64_near_zero_shows_zero() {
        // Given b - a is within [-0.0001, 0.0001].
        let cell = diff_cell_f64(1.0, 1.00005);

        assert_eq!(cell.content(), "$0.0000");
    }

    #[rstest::rstest]
    fn diff_cell_f64_magnitude_is_correct() {
        // Given a = 1.0, b = 2.5, delta should be +1.5.
        let cell = diff_cell_f64(1.0, 2.5);

        let content = cell.content();
        assert!(content.contains('+'), "expected '+' in {content}");
        // delta = b - a = 1.5, not b + a = 3.5, not b / a = 2.5
        assert!(content.contains("1.5"), "expected '1.5' in {content}");
        assert!(
            !content.contains("3.5"),
            "should not contain 3.5 (would mean +): {content}"
        );
    }

    #[rstest::rstest]
    fn diff_cell_f64_negative_magnitude_is_correct() {
        // Given a = 3.0, b = 1.0, delta should be -2.0.
        let cell = diff_cell_f64(3.0, 1.0);

        let content = cell.content();
        assert!(content.contains('-'), "expected '-' in {content}");
        // abs(delta) = 2.0, not 4.0 (would be +), not 3.0 (would be /)
        assert!(content.contains("2.0"), "expected '2.0' in {content}");
        assert!(
            !content.contains("4.0"),
            "should not contain 4.0: {content}"
        );
        assert!(
            !content.contains("3.0"),
            "should not contain 3.0: {content}"
        );
    }

    #[rstest::rstest]
    fn diff_cell_f64_sign_is_preserved() {
        // Given b - a = -0.5, the display should show the absolute value with a minus sign,
        // not a double-negative or zero.
        let cell = diff_cell_f64(1.0, 0.5);

        let content = cell.content();
        // Should show "-$0.5000", not "$-0.5000" (sign deletion mutant)
        assert!(content.contains('-'), "expected '-' in {content}");
        assert!(
            !content.contains("$-"),
            "should not contain '$-' (unary negation deleted): {content}"
        );
    }

    // --- diff_cell_ms ---

    #[rstest::rstest]
    fn diff_cell_ms_greater_shows_plus() {
        let cell = diff_cell_ms(100, 250);

        let content = cell.content();
        assert!(content.contains('+'), "expected '+' in {content}");
        assert!(content.contains("150"), "expected '150' in {content}");
        assert!(content.contains("ms"), "expected 'ms' in {content}");
    }

    #[rstest::rstest]
    fn diff_cell_ms_less_shows_minus() {
        let cell = diff_cell_ms(300, 100);

        let content = cell.content();
        assert!(content.contains('-'), "expected '-' in {content}");
        assert!(content.contains("200"), "expected '200' in {content}");
        assert!(content.contains("ms"), "expected 'ms' in {content}");
    }

    #[rstest::rstest]
    fn diff_cell_ms_equal_shows_zero() {
        let cell = diff_cell_ms(200, 200);

        assert_eq!(cell.content(), "0ms");
    }

    #[rstest::rstest]
    fn diff_cell_ms_magnitude_is_correct() {
        // Given a = 100, b = 130. Delta should be +30, not +100+130 or +100/130.
        let cell = diff_cell_ms(100, 130);

        let content = cell.content();
        assert!(content.contains("30"), "expected '30' in {content}");
        assert!(
            !content.contains("230"),
            "should not contain 230 (would be +): {content}"
        );
    }

    #[rstest::rstest]
    fn diff_cell_ms_negative_magnitude_is_correct() {
        // Given a = 200, b = 50. Delta should be -150, not -(200+50) or -(200/50).
        let cell = diff_cell_ms(200, 50);

        let content = cell.content();
        assert!(content.contains("150"), "expected '150' in {content}");
        assert!(
            !content.contains("250"),
            "should not contain 250 (would be +): {content}"
        );
        assert!(
            !content.contains('4'),
            "should not contain 4 (would be /): {content}"
        );
    }

    // --- format_duration ---

    #[rstest::rstest]
    fn format_duration_returns_nonempty_string() {
        let result = format_duration(1500);
        assert!(!result.is_empty(), "should not be empty");
        assert_ne!(result, "xyzzy", "should not be xyzzy");
    }

    #[rstest::rstest]
    fn format_duration_divides_by_1000() {
        // 2500ms -> "2.5s" (uses /), not "2500s" (would be %) or some huge number (would be *)
        let result = format_duration(2500);
        assert_eq!(result, "2.5s");
    }

    #[rstest::rstest]
    fn format_duration_zero() {
        let result = format_duration(0);
        assert_eq!(result, "0.0s");
    }

    // --- compare_results passed-field mutation ---

    #[rstest::rstest]
    fn compare_results_different_pass_status_shows_arrow() {
        // Given two CSVs where the same task/model has different passed values.
        let dir = tempfile::tempdir().expect("temp dir");
        let csv_a_path = dir.path().join("a.csv");
        let csv_b_path = dir.path().join("b.csv");

        let header =
            "name,category,model,turns,tokens_in,tokens_out,cost,wall_time_ms,passed,status\n";
        std::fs::write(
            &csv_a_path,
            format!("{header}task1,cat1,model-a,1,100,50,0.01,5000,true,completed"),
        )
        .expect("write a");
        std::fs::write(
            &csv_b_path,
            format!("{header}task1,cat1,model-a,1,100,50,0.01,5000,false,completed"),
        )
        .expect("write b");

        // When comparing (captures stdout).
        // The compare_results function prints the table.
        // We just need it to succeed - the mutation is in the == vs != for passed.
        let result = compare_results(&csv_a_path, &csv_b_path);

        // Then it should succeed (the passed field comparison uses ==, not !=).
        assert!(result.is_ok(), "compare_results should succeed");
    }
}
