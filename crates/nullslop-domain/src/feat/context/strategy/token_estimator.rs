//! Token estimation for prompt assembly budgeting.
//!
//! Provides a [`TokenEstimator`] trait for estimating token counts from text,
//! [`CharRatioEstimator`] as a simple heuristic implementation (1 token ≈ 4 characters),
//! and [`estimate_entry_tokens`] for estimating the token cost of individual chat entries.

use std::sync::OnceLock;

use crate::feat::tools_actor::tool_types::ToolDefinition;
use crate::protocol::{ChatEntry, ChatEntryKind};

/// Estimates the token count of text.
///
/// Used by budget-based strategies to decide how much history to include.
/// Implementations may range from simple heuristics to real tokenizer calls.
pub trait TokenEstimator: Send + Sync {
    /// Estimate the number of tokens in `text`.
    fn estimate(&self, text: &str) -> usize;

    /// The name of this estimator, for debugging.
    fn name(&self) -> &'static str;
}

/// Estimate the token count for a single chat entry's text content.
///
/// Uses the same fields that `entries_to_messages` would convert to LLM messages.
/// Entries not in context (per `is_in_context()`) contribute 0 tokens.
pub fn estimate_entry_tokens(estimator: &dyn TokenEstimator, entry: &ChatEntry) -> usize {
    // Entries not in context contribute 0 tokens.
    if !entry.is_in_context() {
        return 0;
    }

    match &entry.kind {
        ChatEntryKind::User { expanded, .. } => estimator.estimate(expanded),
        ChatEntryKind::Assistant(text)
        | ChatEntryKind::System(text)
        | ChatEntryKind::Skill { content: text, .. }
        | ChatEntryKind::Compaction { summary: text, .. } => estimator.estimate(text),
        ChatEntryKind::ToolCall {
            name, arguments, ..
        } => estimator.estimate(name) + estimator.estimate(arguments),
        ChatEntryKind::ToolResult { name, content, .. } => {
            estimator.estimate(name) + estimator.estimate(content)
        }
        // Actor entries produce LlmMessage::User with a prefix when in context.
        ChatEntryKind::Actor { source, text } => {
            estimator.estimate(&format!("[Actor: {source}] {text}"))
        }
        // Error entries produce LlmMessage::User with [Error] prefix.
        ChatEntryKind::Error(text) => estimator.estimate(&format!("[Error] {text}")),
        // Thinking entries produce LlmMessage::User with [Thinking] prefix when in context.
        ChatEntryKind::Thinking(text) => estimator.estimate(&format!("[Thinking] {text}")),
        // Transient entries produce LlmMessage::User with [Transient] prefix when in context.
        ChatEntryKind::Transient(text) => estimator.estimate(&format!("[Transient] {text}")),
    }
}

/// Estimates the token cost of tool definitions as serialized JSON schemas.
///
/// Each [`ToolDefinition`] is serialized to JSON via `serde_json::to_string`
/// and estimated using the given estimator. This captures the cost of the
/// name, description, parameters schema, prompt_snippet, and prompt_guidelines
/// as they appear in the API request.
pub fn estimate_tool_schema_tokens(
    estimator: &dyn TokenEstimator,
    tools: &[ToolDefinition],
) -> usize {
    tools
        .iter()
        .map(|td| {
            let json = serde_json::to_string(td).unwrap_or_default();
            estimator.estimate(&json)
        })
        .sum()
}

/// Simple heuristic estimator: 1 token ≈ 4 Unicode characters.
///
/// Good enough for initial use. Uses `text.chars().count()` for Unicode correctness
/// rather than byte length. Always returns at least 1 to avoid zero-token estimates.
pub struct CharRatioEstimator;

impl TokenEstimator for CharRatioEstimator {
    #[expect(
        clippy::integer_division,
        reason = "1 token ≈ 4 characters is intentional rounding"
    )]
    fn estimate(&self, text: &str) -> usize {
        text.chars().count() / 4 + 1
    }

    fn name(&self) -> &'static str {
        "char_ratio"
    }
}

/// Counts tokens in text using a specific tokenizer.
///
/// Unlike [`TokenEstimator`] which is a rough heuristic for budget planning,
/// [`TokenCounter`] produces counts suitable for recording in the session's
/// immutable token ledger. Implementations may use real tokenizers (tiktoken)
/// or simple heuristics.
pub trait TokenCounter: Send + Sync {
    /// Count the number of tokens in `text`.
    fn count(&self, text: &str) -> usize;

    /// The name of this counter, for debugging.
    fn name(&self) -> &'static str;
}

/// Token counter using the `tiktoken` crate with a configurable encoding.
///
/// Wraps a `tiktoken::CoreBPE` encoder. The encoding is chosen at construction
/// time and does not change — counts are deterministic for a given text input.
/// This is important: once a count is recorded in the token ledger, it is
/// immutable regardless of future model/tokenizer changes.
#[derive(Clone, Copy)]
pub struct TiktokenCounter {
    encoder: &'static tiktoken::CoreBpe,
    encoding_name: &'static str,
}

impl TiktokenCounter {
    /// Create a counter using the `o200k_base` encoding (GPT-4o, o1, o3).
    ///
    /// This is a reasonable default for most LLM interactions.
    ///
    /// # Panics
    ///
    /// Panics if the `o200k_base` encoding is unavailable, which should never happen
    /// as it is a built-in tiktoken encoding.
    #[must_use]
    pub fn o200k_base() -> Self {
        static ENCODING: OnceLock<Option<&'static tiktoken::CoreBpe>> = OnceLock::new();
        let encoder = ENCODING
            .get_or_init(|| tiktoken::get_encoding("o200k_base"))
            .expect("o200k_base encoding should always be available");
        Self {
            encoder,
            encoding_name: "o200k_base",
        }
    }
}

impl TokenCounter for TiktokenCounter {
    fn count(&self, text: &str) -> usize {
        self.encoder.count(text)
    }

    fn name(&self) -> &'static str {
        self.encoding_name
    }
}
