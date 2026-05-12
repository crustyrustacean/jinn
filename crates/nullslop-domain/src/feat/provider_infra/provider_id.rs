//! Unique identifier for configured providers.
//!
//! [`ProviderId`] holds a `{name}/{model}` string (e.g., `"ollama/llama3"`,
//! `"openrouter/openai/gpt-oss-120b"`). Created during registry expansion,
//! one per model in each provider block. Used in protocol types, app state,
//! and the picker to unambiguously identify which provider+model is in play.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Uniquely identifies a configured provider+model combination.
///
/// Format: `{provider_name}/{model_name}` (e.g., `"ollama/llama3"`).
/// Used in protocol types, app state, and the picker.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProviderId(String);

impl ProviderId {
    /// Create a new provider ID from a string.
    #[must_use]
    pub fn new(name: String) -> Self {
        Self(name)
    }

    /// Returns the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<String> for ProviderId {
    fn from(value: String) -> Self {
        Self(value)
    }
}
