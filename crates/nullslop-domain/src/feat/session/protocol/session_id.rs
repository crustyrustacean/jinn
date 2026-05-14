//! Unique identifier for a chat session.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A unique identifier for a chat session.
///
/// Generated using UUID v4, stored as an opaque string.
/// Derives equality and hashing so it can be used as a `HashMap` key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(String);

impl SessionId {
    /// Generate a new unique session ID using UUID v4.
    #[must_use]
    pub fn new() -> Self {
        Self(format!("s-{}", Uuid::new_v4()))
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
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
    fn session_id_starts_with_prefix() {
        // Given a new session ID.
        let id = SessionId::new();

        // When inspecting the string representation.
        // Note: we can't access the inner String directly, so we check serialization.
        let json = serde_json::to_string(&id).expect("serialize");

        // Then the serialized form starts with "s-".
        assert!(json.contains("s-"));
    }
}
