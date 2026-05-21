//! CSV output for bench results.
//!
//! Defines the result schema and progressive CSV writer. Each task/model pair
//! produces one row. The writer flushes after every row so partial results
//! survive Ctrl+C.

use std::path::Path;

use csv::Writer;

/// One row of bench output — the result of running a single task against a single model.
#[derive(Debug, Clone)]
pub struct BenchResult {
    /// Task name.
    pub name: String,
    /// Model ID (e.g., "openai/gpt-4o").
    pub model: String,
    /// Number of turns (user + assistant entries).
    pub turns: u32,
    /// Total tokens sent (input).
    pub tokens_in: u64,
    /// Total tokens received (output).
    pub tokens_out: u64,
    /// Total cost in USD.
    pub cost: f64,
    /// Wall time in milliseconds.
    pub wall_time_ms: u64,
    /// Whether the verification function passed.
    pub passed: bool,
    /// Status: "completed" or "timeout".
    pub status: String,
}

/// CSV column header, in order.
const HEADER: [&str; 9] = [
    "name",
    "model",
    "turns",
    "tokens_in",
    "tokens_out",
    "cost",
    "wall_time_ms",
    "passed",
    "status",
];

/// A progressive CSV writer that flushes after every row.
pub struct BenchCsvWriter {
    writer: Writer<std::fs::File>,
}

impl BenchCsvWriter {
    /// Creates a new CSV writer at the given path, writing the header row immediately.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be created or the header cannot be written.
    pub fn create(path: &Path) -> std::io::Result<Self> {
        let file = std::fs::File::create(path)?;
        let mut writer = csv::Writer::from_writer(file);
        writer.write_record(HEADER)?;
        writer.flush()?;
        Ok(Self { writer })
    }

    /// Appends a single result row and flushes immediately.
    ///
    /// # Errors
    ///
    /// Returns an error if the row cannot be written or flushed.
    pub fn write_row(&mut self, result: &BenchResult) -> std::io::Result<()> {
        self.writer.write_record([
            result.name.as_str(),
            result.model.as_str(),
            result.turns.to_string().as_str(),
            result.tokens_in.to_string().as_str(),
            result.tokens_out.to_string().as_str(),
            result.cost.to_string().as_str(),
            result.wall_time_ms.to_string().as_str(),
            if result.passed { "true" } else { "false" },
            result.status.as_str(),
        ])?;
        self.writer.flush()?;
        Ok(())
    }
}
