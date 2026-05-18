//! Token estimation for prompt assembly budgeting.
//!
//! Provides a [`TokenEstimator`] trait for estimating token counts from text,
//! [`CharRatioEstimator`] as a simple heuristic implementation (1 token ≈ 4 characters),
//! and [`estimate_entry_tokens`] for estimating the token cost of individual chat entries.

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
/// Uses the same fields that `entries_to_messages` would convert to LLM messages:
/// [`ChatEntryKind::User`]/[`ChatEntryKind::Assistant`] content, [`ChatEntryKind::ToolCall`] name+arguments, [`ChatEntryKind::ToolResult`] name+content.
///
/// Unpinned System and Actor entries contribute 0 tokens since they are not
/// sent to the LLM. Pinned System and Actor entries are estimated based on the
/// text that `entries_to_messages` would produce for them.
pub fn estimate_entry_tokens(estimator: &dyn TokenEstimator, entry: &ChatEntry) -> usize {
    // Ignored entries contribute 0 tokens unless pinned (pin overrides ignore).
    if entry.ignored && !entry.is_pinned() {
        return 0;
    }

    match &entry.kind {
        ChatEntryKind::User { expanded, .. } => estimator.estimate(expanded),
        ChatEntryKind::Assistant(text) | ChatEntryKind::Error(text) => estimator.estimate(text),
        ChatEntryKind::ToolCall {
            name, arguments, ..
        } => estimator.estimate(name) + estimator.estimate(arguments),
        ChatEntryKind::ToolResult { name, content, .. } => {
            estimator.estimate(name) + estimator.estimate(content)
        }
        // Pinned System entries produce LlmMessage::System when sent to the LLM.
        ChatEntryKind::System(text) => {
            if entry.is_pinned() {
                estimator.estimate(text)
            } else {
                0
            }
        }
        // Pinned Actor entries produce LlmMessage::User with a prefix when sent to the LLM.
        ChatEntryKind::Actor { source, text } => {
            if entry.is_pinned() {
                estimator.estimate(&format!("[Actor: {source}] {text}"))
            } else {
                0
            }
        }
        // Table entries are ephemeral display data — estimate based on plain-text content.
        ChatEntryKind::Table(data) => estimator.estimate(&data.to_plain_text()),
        // Thinking entries are excluded from context assembly — contribute 0 tokens.
        // Info entries are UI-only — excluded from context assembly.
        ChatEntryKind::Thinking(_) | ChatEntryKind::Info(_) => 0,
        // Skill entries produce LlmMessage::System with XML wrapping.
        ChatEntryKind::Skill { content, .. } => {
            if entry.is_pinned() {
                estimator.estimate(content)
            } else {
                0
            }
        }
        // Compaction entries carry an LLM-generated summary that replaces compacted history.
        ChatEntryKind::Compaction { summary, .. } => estimator.estimate(summary),
    }
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
        let encoder = tiktoken::get_encoding("o200k_base")
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
