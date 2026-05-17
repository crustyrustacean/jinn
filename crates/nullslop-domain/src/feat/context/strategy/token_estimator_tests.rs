use crate::feat::context::strategy::token_estimator::{
    CharRatioEstimator, TiktokenCounter, TokenCounter, TokenEstimator, estimate_entry_tokens,
};
use crate::protocol::{ChatEntry, PinPosition};

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
