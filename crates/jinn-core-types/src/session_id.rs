//! Unique identifier for a chat session.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A unique identifier for a chat session.
///
/// Generated using UUID v7, stored as a bare `Uuid`. The serialized/displayed
/// form is the bare UUID string (no prefix). Derives equality and hashing so
/// it can be used as a `HashMap` key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct SessionId(Uuid);

impl SessionId {
    /// Generate a new unique session ID using UUID v7.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Parses a session ID from its string form without panicking.
    ///
    /// For untrusted input (e.g. plugin wire payloads): an unparseable
    /// string yields `None` rather than the panic [`From::from`] would
    /// produce.
    #[must_use]
    pub fn try_from_string(id: &str) -> Option<Self> {
        Uuid::parse_str(id).ok().map(Self)
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<String> for SessionId {
    #[expect(
        clippy::expect_used,
        reason = "From is infallible. Should be caught by testing."
    )]
    fn from(id: String) -> Self {
        Self(Uuid::parse_str(&id).expect("invalid UUID format (programming error)"))
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::*;

    #[rstest::rstest]
    fn session_id_new_generates_unique_ids() {
        // Given nothing.
        // When generating two session IDs.
        let a = SessionId::new();
        let b = SessionId::new();

        // Then they are different.
        assert_ne!(a, b);
    }

    #[rstest::rstest]
    fn session_id_serializes_as_bare_uuid() {
        // Given a new session ID.
        let id = SessionId::new();

        // When serializing to JSON and displaying.
        let json = serde_json::to_string(&id).expect("serialize");
        let display = id.to_string();

        // Then both forms are the bare UUID with no 's-' prefix.
        assert!(!json.contains("s-"), "json should be bare uuid: {json}");
        assert!(
            !display.starts_with("s-"),
            "display should be bare uuid: {display}"
        );
    }
}
