//! The append-only record of what an attendant concluded.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// One thing an attendant reported during one of its runs.
///
/// Reports are never edited or removed by the harness — the log is the audit
/// trail the user reads, and the next run is seeded from its most recent entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttendantReport {
    /// Which run produced this report, counted from one within this attendant.
    pub run: usize,
    /// When the report was published.
    pub published_at: Timestamp,
    /// The report body, as written by the model.
    pub body: String,
}
