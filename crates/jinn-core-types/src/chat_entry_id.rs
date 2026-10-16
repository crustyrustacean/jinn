//! Unique identifier for a chat entry.

use serde::{Deserialize, Serialize};

/// A unique identifier for a [`ChatEntry`](https://docs.rs/jinn-kernel) entry.
///
/// Auto-generated as a UUID (v7 via `now_v7`). Used by prompt assembly
/// strategies to reference specific entries without positional coupling, and
/// as the per-session map key for chat-log view state.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChatEntryId(uuid::Uuid);

impl ChatEntryId {
    /// Generate a new unique ID.
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7())
    }

    /// Returns the underlying UUID value.
    #[must_use]
    pub fn as_uuid(&self) -> &uuid::Uuid {
        &self.0
    }

    /// Parses an entry ID from its string form without panicking and
    /// without fabricating.
    ///
    /// For untrusted input (e.g. a tool argument): an unparseable string
    /// yields `None` rather than a fresh id that silently resolves to no
    /// entry.
    #[must_use]
    pub fn try_from_string(id: &str) -> Option<Self> {
        uuid::Uuid::parse_str(id).ok().map(Self)
    }

    /// Derives a deterministic entry id from a stable tag.
    ///
    /// Tests (and any consumer needing a stable, valid id from a label) use
    /// this instead of parsing an arbitrary tag, which would fail on a
    /// non-UUID string.
    #[must_use]
    pub fn new_v5_tag(tag: &str) -> Self {
        Self(uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            tag.as_bytes(),
        ))
    }
}

impl Default for ChatEntryId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ChatEntryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use super::*;

    #[rstest::rstest]
    fn chat_entry_id_new_generates_unique_ids() {
        // Given nothing.
        // When generating two entry IDs.
        let a = ChatEntryId::new();
        let b = ChatEntryId::new();

        // Then they are different.
        assert_ne!(a, b);
    }

    #[rstest::rstest]
    fn chat_entry_id_serializes_as_bare_uuid() {
        // Given a new entry ID.
        let id = ChatEntryId::new();

        // When serializing to JSON and displaying.
        let json = serde_json::to_string(&id).expect("serialize");
        let display = id.to_string();

        // Then both forms are the bare UUID string.
        assert_eq!(json, format!("\"{display}\""));
    }

    #[rstest::rstest]
    fn chat_entry_id_try_from_string_roundtrips() {
        // Given a new entry ID rendered as a string.
        let id = ChatEntryId::new();
        let rendered = id.to_string();

        // When parsing it back.
        let parsed = ChatEntryId::try_from_string(&rendered).expect("valid id parses");

        // Then it equals the original.
        assert_eq!(parsed, id);
    }

    #[rstest::rstest]
    fn chat_entry_id_try_from_string_rejects_garbage() {
        // Given a string that is not a UUID.
        // When parsing it.
        // Then the parse fails instead of fabricating an id.
        assert!(ChatEntryId::try_from_string("not-a-uuid").is_none());
    }
}
