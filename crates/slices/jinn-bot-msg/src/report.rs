//! The append-only record of what a bot concluded.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// One thing a bot reported during one of its runs.
///
/// Reports are never edited or removed by the harness — the log is the audit
/// trail the user reads, and the next run is seeded from its most recent entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotReport {
    /// Which run produced this report, counted from one within this bot.
    pub run: usize,
    /// When the report was published.
    pub published_at: Timestamp,
    /// The report body, as written by the model.
    pub body: String,
}

impl BotReport {
    /// Marker shown for a bot that has published nothing yet.
    pub const EMPTY_MARKER: &'static str = "· no reports yet";
}
