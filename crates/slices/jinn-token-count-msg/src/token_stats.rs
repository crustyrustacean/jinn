//! Session token-ledger values and pure aggregation.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// One request/response accounting record in a session token ledger.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenRecord {
    /// When the request was made.
    pub timestamp: Timestamp,
    /// Locally estimated input tokens.
    pub tokens_sent: u32,
    /// Provider-reported output tokens, or zero until completion.
    pub tokens_received: u32,
    /// Provider-reported cost in USD, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    /// Concrete model selected for this request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_used: Option<String>,
    /// Provider-reported input-token count, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u32>,
    /// Provider-reported cache-hit count, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u32>,
}

/// Summary statistics derived from a token ledger.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenStats {
    /// Sum of local input-token estimates.
    pub total_sent: u64,
    /// Sum of provider-reported output tokens.
    pub total_received: u64,
    /// Number of request/response records.
    pub request_count: u64,
    /// Provider-reported input tokens with local estimates as fallback.
    pub effective_sent: u64,
    /// Provider-reported input tokens over measured records only.
    pub measured_sent: u64,
    /// Sum of provider-reported cache hits.
    pub cached_total: u64,
}

impl TokenStats {
    /// Derives statistics from a token ledger.
    #[must_use]
    pub fn from_ledger(records: &[TokenRecord]) -> Self {
        let mut stats = Self::default();
        for record in records {
            stats.total_sent += u64::from(record.tokens_sent);
            stats.total_received += u64::from(record.tokens_received);
            stats.request_count += 1;
            stats.effective_sent += u64::from(record.prompt_tokens.unwrap_or(record.tokens_sent));
            if let Some(prompt) = record.prompt_tokens {
                stats.measured_sent += u64::from(prompt);
            }
            stats.cached_total += u64::from(record.cached_tokens.unwrap_or(0));
        }
        stats
    }

    /// Sums all provider-reported costs in a ledger.
    #[must_use]
    pub fn total_cost(records: &[TokenRecord]) -> f64 {
        records.iter().filter_map(|record| record.cost).sum()
    }
}

/// Token totals for a session and its descendants.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AggregatedTokenStats {
    /// Statistics for the target session.
    pub own: TokenStats,
    /// Statistics summed across descendant sessions.
    pub children: TokenStats,
    /// Cost for the target session.
    pub own_cost: f64,
    /// Cost across descendant sessions.
    pub children_cost: f64,
}

impl AggregatedTokenStats {
    /// Returns combined sent tokens.
    #[must_use]
    pub fn total_sent(&self) -> u64 {
        self.own.total_sent + self.children.total_sent
    }

    /// Returns combined received tokens.
    #[must_use]
    pub fn total_received(&self) -> u64 {
        self.own.total_received + self.children.total_received
    }

    /// Returns combined cost.
    #[must_use]
    pub fn total_cost(&self) -> f64 {
        self.own_cost + self.children_cost
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::float_cmp,
        clippy::panic,
        clippy::unreachable,
        reason = "test code"
    )]

    use super::*;

    fn record(
        tokens_sent: u32,
        tokens_received: u32,
        prompt_tokens: Option<u32>,
        cached_tokens: Option<u32>,
    ) -> TokenRecord {
        TokenRecord {
            timestamp: Timestamp::UNIX_EPOCH,
            tokens_sent,
            tokens_received,
            cost: None,
            model_used: None,
            prompt_tokens,
            cached_tokens,
        }
    }

    #[rstest::rstest]
    fn ledger_aggregation_sums_raw_and_measured_values() {
        // Given measured and unmeasured ledger records.
        let records = [record(100, 10, Some(120), None), record(50, 0, None, None)];

        // When aggregating the ledger.
        let stats = TokenStats::from_ledger(&records);

        // Then raw, effective, and measured totals use their own definitions.
        assert_eq!(stats.total_sent, 150);
        assert_eq!(stats.total_received, 10);
        assert_eq!(stats.effective_sent, 170);
        assert_eq!(stats.measured_sent, 120);
        assert_eq!(stats.cached_total, 0);
    }

    #[rstest::rstest]
    fn aggregated_totals_include_own_and_descendants() {
        // Given separate own and descendant statistics.
        let stats = AggregatedTokenStats {
            own: TokenStats {
                total_sent: 100,
                total_received: 50,
                ..TokenStats::default()
            },
            children: TokenStats {
                total_sent: 200,
                total_received: 100,
                ..TokenStats::default()
            },
            own_cost: 0.01,
            children_cost: 0.02,
        };

        // When reading combined totals.
        let sent = stats.total_sent();
        let received = stats.total_received();
        let cost = stats.total_cost();

        // Then both scopes contribute.
        assert_eq!(sent, 300);
        assert_eq!(received, 150);
        assert!((cost - 0.03).abs() < f64::EPSILON);
    }

    #[rstest::rstest]
    fn token_record_legacy_fields_default_to_none() {
        // Given legacy JSON without provider-usage fields.
        let json = r#"{"timestamp":"2024-01-01T00:00:00Z","tokens_sent":100,"tokens_received":50}"#;

        // When deserializing.
        let record: TokenRecord = serde_json::from_str(json).expect("deserialize");

        // Then optional fields retain compatibility defaults.
        assert!(record.cost.is_none());
        assert!(record.model_used.is_none());
        assert!(record.prompt_tokens.is_none());
        assert!(record.cached_tokens.is_none());
    }
}
