//! Epoch-sourced vocabulary shared by both cells: the parsed ECI index
//! and where the epoch data on display came from.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Parsed epoch capability scores, keyed by ECI display name.
///
/// The join key is the *name*: `eci_scores.csv` carries no OpenRouter
/// ids, so the models actor resolves each listing to an ECI name through
/// strict display-name matching plus the compiled-in alias table, then
/// reads the score here. Normalization metadata rows ride along for the
/// help card's data-sources section.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EciIndex {
    /// ECI display name → `(score, ci_low, ci_high)`.
    pub scores: HashMap<String, EciScore>,
}

/// One model version's capability score with its bootstrap interval.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EciScore {
    /// The Epoch Capabilities Index value.
    pub eci: f64,
    /// Lower bound of the bootstrap confidence interval.
    pub ci_low: f64,
    /// Upper bound of the bootstrap confidence interval.
    pub ci_high: f64,
}

impl EciIndex {
    /// The score for an ECI display name, if the index carries it.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&EciScore> {
        self.scores.get(name)
    }

    /// How many model versions the index holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.scores.len()
    }

    /// Whether the index is empty (no seed, no download, nothing parsed).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.scores.is_empty()
    }
}

/// Where the epoch data currently in the data cell came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EpochSource {
    /// The CSVs compiled into the slice (`include_bytes!`): first run
    /// with no cache dir and no network still shows ECI values.
    #[default]
    Seed,
    /// Extracted from a downloaded `benchmark_data.zip` into the cache
    /// dir.
    Download,
    /// Nothing loaded — the ECI column is `—` everywhere.
    None,
}

impl EpochSource {
    /// The status line's short form.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Seed => "seed",
            Self::Download => "downloaded",
            Self::None => "none",
        }
    }
}
