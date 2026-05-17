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

#[cfg(test)]
mod tests {
    use crate::protocol::{ChatEntry, PinPosition};

    use super::*;

    #[rstest::rstest]
    fn char_ratio_returns_nonzero_for_empty_string() {
        // Given a char ratio estimator.
        let estimator = CharRatioEstimator;

        // When estimating an empty string.
        let tokens = estimator.estimate("");

        // Then at least 1 token is returned.
        assert!(tokens >= 1);
    }

    #[rstest::rstest]
    fn char_ratio_estimates_reasonably() {
        // Given a char ratio estimator and a 100-character string.
        let estimator = CharRatioEstimator;
        let text = "a".repeat(100);

        // When estimating tokens.
        let tokens = estimator.estimate(&text);

        // Then approximately 25 tokens are returned (100/4 + 1 = 26).
        assert_eq!(tokens, 26);
    }

    #[rstest::rstest]
    fn char_ratio_name() {
        // Given a char ratio estimator.
        let estimator = CharRatioEstimator;

        // Then its name is "char_ratio".
        assert_eq!(estimator.name(), "char_ratio");
    }

    #[rstest::rstest]
    fn char_ratio_handles_unicode_correctly() {
        // Given a char ratio estimator and a string with multi-byte characters.
        let estimator = CharRatioEstimator;
        // "日本語" is 3 Unicode characters but 9 bytes in UTF-8.
        let text = "日本語";

        // When estimating tokens.
        let tokens = estimator.estimate(text);

        // Then it uses character count (3), not byte count (3 * 3/4 = 2, rounded = 1).
        assert_eq!(tokens, 1);
    }

    #[rstest::rstest]
    fn estimate_entry_tokens_for_user() {
        // Given a char ratio estimator and a user entry.
        let estimator = CharRatioEstimator;
        let entry = ChatEntry::user("hello world");

        // When estimating entry tokens.
        let tokens = estimate_entry_tokens(&estimator, &entry);

        // Then it matches estimating the user text directly.
        assert_eq!(tokens, estimator.estimate("hello world"));
    }

    #[rstest::rstest]
    fn estimate_entry_tokens_for_tool_call() {
        // Given a char ratio estimator and a tool call entry.
        let estimator = CharRatioEstimator;
        let entry = ChatEntry::tool_call("call_1", "echo", r#"{"input":"hi"}"#);

        // When estimating entry tokens.
        let tokens = estimate_entry_tokens(&estimator, &entry);

        // Then it estimates name + arguments combined.
        assert_eq!(
            tokens,
            estimator.estimate("echo") + estimator.estimate(r#"{"input":"hi"}"#)
        );
    }

    #[rstest::rstest]
    fn estimate_entry_tokens_for_system_is_zero() {
        // Given a char ratio estimator and an unpinned system entry.
        let estimator = CharRatioEstimator;
        let entry = ChatEntry::system("some status message");

        // When estimating entry tokens.
        let tokens = estimate_entry_tokens(&estimator, &entry);

        // Then unpinned system entries contribute 0 tokens.
        assert_eq!(tokens, 0);
    }

    #[rstest::rstest]
    fn estimate_entry_tokens_for_pinned_system_is_nonzero() {
        // Given a char ratio estimator and a pinned system entry.
        let estimator = CharRatioEstimator;
        let entry = ChatEntry::system("important instruction").with_pin(PinPosition::Top);

        // When estimating entry tokens.
        let tokens = estimate_entry_tokens(&estimator, &entry);

        // Then pinned system entries contribute tokens equal to their text.
        assert_eq!(tokens, estimator.estimate("important instruction"));
        assert!(tokens > 0);
    }

    #[rstest::rstest]
    fn estimate_entry_tokens_for_unpinned_actor_is_zero() {
        // Given a char ratio estimator and an unpinned actor entry.
        let estimator = CharRatioEstimator;
        let entry = ChatEntry::actor("echo", "HELLO");

        // When estimating entry tokens.
        let tokens = estimate_entry_tokens(&estimator, &entry);

        // Then unpinned actor entries contribute 0 tokens.
        assert_eq!(tokens, 0);
    }

    #[rstest::rstest]
    fn estimate_entry_tokens_for_pinned_actor_is_nonzero() {
        // Given a char ratio estimator and a pinned actor entry.
        let estimator = CharRatioEstimator;
        let entry = ChatEntry::actor("echo", "HELLO").with_pin(PinPosition::Relative);

        // When estimating entry tokens.
        let tokens = estimate_entry_tokens(&estimator, &entry);

        // Then pinned actor entries contribute tokens matching the formatted output.
        assert_eq!(tokens, estimator.estimate("[Actor: echo] HELLO"));
        assert!(tokens > 0);
    }

    // --- TiktokenCounter tests ---

    #[rstest::rstest]
    fn tiktoken_counter_counts_hello_world() {
        // Given a tiktoken counter with o200k_base.
        let counter = TiktokenCounter::o200k_base();

        // When counting "hello world".
        let count = counter.count("hello world");

        // Then it returns 2 tokens.
        assert_eq!(count, 2);
    }

    #[rstest::rstest]
    fn tiktoken_counter_returns_nonzero_for_empty_string() {
        // Given a tiktoken counter.
        let counter = TiktokenCounter::o200k_base();

        // When counting an empty string.
        let count = counter.count("");

        // Then it returns 0.
        assert_eq!(count, 0);
    }

    #[rstest::rstest]
    fn tiktoken_counter_name_is_o200k_base() {
        // Given a tiktoken counter.
        let counter = TiktokenCounter::o200k_base();

        // Then its name is "o200k_base".
        assert_eq!(counter.name(), "o200k_base");
    }

    #[rstest::rstest]
    fn tiktoken_counter_counts_multibyte_characters() {
        // Given a tiktoken counter.
        let counter = TiktokenCounter::o200k_base();

        // When counting Japanese text.
        let count = counter.count("日本語テスト");

        // Then it returns a nonzero count.
        assert!(count > 0);
    }

    #[rstest::rstest]
    fn estimate_entry_tokens_for_thinking_is_zero() {
        // Given a char ratio estimator and a thinking entry.
        let estimator = CharRatioEstimator;
        let entry = ChatEntry::thinking("a long reasoning text that would normally cost tokens");

        // When estimating entry tokens.
        let tokens = estimate_entry_tokens(&estimator, &entry);

        // Then thinking entries contribute 0 tokens.
        assert_eq!(tokens, 0);
    }

    #[rstest::rstest]
    fn estimate_entry_tokens_for_info_is_zero() {
        // Given a char ratio estimator and an info entry.
        let estimator = CharRatioEstimator;
        let entry = ChatEntry::info("Welcome to nullslop! Press i to start typing.");

        // When estimating entry tokens.
        let tokens = estimate_entry_tokens(&estimator, &entry);

        // Then info entries contribute 0 tokens.
        assert_eq!(tokens, 0);
    }
}
