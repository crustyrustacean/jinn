//! Unique identifier for one LLM tool call.

use serde::{Deserialize, Serialize};

/// The identifier a provider assigns to one tool call.
///
/// Provider-minted, so String-backed rather than UUID-backed: OpenAI emits
/// `call_…`, Anthropic emits `toolu_…`, and any format they choose must
/// round-trip untouched. Calls and results are paired by equality of this
/// id across the whole request/response cycle.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct ToolCallId(String);

impl ToolCallId {
    /// Wraps a provider-assigned id string.
    #[must_use]
    pub fn new<S: Into<String>>(id: S) -> Self {
        Self(id.into())
    }

    /// Returns the id in its provider form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ToolCallId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl From<&str> for ToolCallId {
    fn from(id: &str) -> Self {
        Self(id.to_owned())
    }
}

impl std::fmt::Display for ToolCallId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "test code")]

    use super::ToolCallId;

    #[rstest::rstest]
    #[test]
    fn serde_roundtrips_a_non_uuid_provider_id() {
        // Given an id in OpenAI's `call_` form.
        let id = ToolCallId::new("call_9xQo8f2nAbCd");

        // When serializing to JSON and back.
        let json = serde_json::to_string(&id).expect("serialize");
        let parsed: ToolCallId = serde_json::from_str(&json).expect("deserialize");

        // Then the id survives verbatim.
        assert_eq!(json, "\"call_9xQo8f2nAbCd\"");
        assert_eq!(parsed, id);
    }

    #[rstest::rstest]
    #[test]
    fn display_shows_the_provider_form() {
        // Given an Anthropic-shaped id.
        let id = ToolCallId::new("toolu_01A");

        // When displaying.
        // Then the output is the bare provider string.
        assert_eq!(id.to_string(), "toolu_01A");
    }
}
