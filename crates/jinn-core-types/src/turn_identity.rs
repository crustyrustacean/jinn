//! Turn identity: the per-session counters that name a turn and a stream.
//!
//! These newtypes ride every message that belongs to a turn so consumers can
//! ask "does this event belong to the live turn?" instead of "what phase do I
//! observe?". Both are u64 counters minted by the turn ledger — equality is
//! the only comparison, so a non-monotonic clock can never mint a stale value
//! that sorts as fresh.

use serde::Deserialize;
use serde::Serialize;

/// A turn's identity: one value per fresh turn, monotonic per session.
///
/// Carried by every event that belongs to the turn. The turn ledger is the
/// only minter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct TurnToken(u64);

impl TurnToken {
    /// Wraps a raw counter value.
    ///
    /// The turn ledger is the only intended caller: every other path must
    /// carry a token the ledger minted.
    #[must_use]
    pub fn from_counter(value: u64) -> Self {
        Self(value)
    }

    /// The token's raw counter value, for logs.
    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }
}

/// A provider stream's identity: one value per dispatch, monotonic per
/// session, minted alongside the turn token it belongs to.
///
/// Carried by stream-scoped events (tokens, completions) so a stream swapped
/// in by a retry refuses its predecessor's stragglers. The turn ledger is
/// the only minter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct StreamId(u64);

impl StreamId {
    /// Wraps a raw counter value.
    #[must_use]
    pub fn from_counter(value: u64) -> Self {
        Self(value)
    }

    /// The stream's raw counter value, for logs.
    #[must_use]
    pub fn get(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]

    use super::StreamId;
    use super::TurnToken;

    #[rstest::rstest]
    #[test]
    fn turn_tokens_distinguish_identity_by_value() {
        // Given two tokens with different counters.
        let a = TurnToken(1);
        let b = TurnToken(2);

        // Then they are distinct, and equal only to themselves.
        assert_ne!(a, b);
        assert_eq!(a, TurnToken(1));
        assert_eq!(a.get(), 1);
    }

    #[rstest::rstest]
    #[test]
    fn stream_ids_distinguish_identity_by_value() {
        // Given two stream ids with different counters.
        let a = StreamId(1);
        let b = StreamId(2);

        // Then they are distinct, and equal only to themselves.
        assert_ne!(a, b);
        assert_eq!(a, StreamId(1));
        assert_eq!(a.get(), 1);
    }

    #[rstest::rstest]
    #[test]
    fn unset_values_roundtrip_as_zero() {
        // Given the zero values (the serde default for a missing field).
        let token = TurnToken::default();
        let stream = StreamId::default();

        // When serializing and deserializing.
        let token_back: TurnToken =
            serde_json::from_str(&serde_json::to_string(&token).unwrap()).unwrap();
        let stream_back: StreamId =
            serde_json::from_str(&serde_json::to_string(&stream).unwrap()).unwrap();

        // Then they roundtrip unchanged as the unset value.
        assert_eq!(token_back, TurnToken(0));
        assert_eq!(stream_back, StreamId(0));
    }
}
