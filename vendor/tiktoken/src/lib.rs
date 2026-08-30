//! High-performance pure-Rust BPE tokenizer compatible with OpenAI's tiktoken
//! and all mainstream LLM tokenizers.
//!
//! This vendored build embeds only the o200k family: `o200k_base` and
//! `o200k_harmony` (which shares the o200k vocabulary). Other upstream
//! encodings are intentionally compiled out to reduce binary size.
//!
//! Includes token encoding, decoding, counting, and multi-provider pricing.
//!
//! # Quick Start
//!
//! ```
//! // by encoding name
//! let enc = tiktoken::get_encoding("o200k_base").unwrap();
//! let tokens = enc.encode("hello world");
//! let text = enc.decode_to_string(&tokens).unwrap();
//! assert_eq!(text, "hello world");
//!
//! // by model name
//! let enc = tiktoken::encoding_for_model("gpt-4o").unwrap();
//! let count = enc.count("hello world");
//! assert_eq!(count, 2);
//! ```

mod bpe;
pub mod encoding;
mod merge;
mod pretokenize;
pub mod pricing;
mod vocab;

pub use bpe::CoreBpe;

use std::sync::OnceLock;

static O200K_BASE: OnceLock<CoreBpe> = OnceLock::new();
static O200K_HARMONY: OnceLock<CoreBpe> = OnceLock::new();

/// All available encoding names.
///
/// Returns the list of encoding names that can be passed to [`get_encoding`].
///
/// # Examples
///
/// ```
/// let names = tiktoken::list_encodings();
/// assert!(names.contains(&"o200k_base"));
/// assert!(names.contains(&"o200k_harmony"));
/// assert_eq!(names.len(), 2);
/// ```
pub fn list_encodings() -> &'static [&'static str] {
    &["o200k_base", "o200k_harmony"]
}

/// Get a cached tokenizer by encoding name.
///
/// Supported encodings (this vendored build embeds only the o200k family):
/// - `o200k_base`
/// - `o200k_harmony`
pub fn get_encoding(name: &str) -> Option<&'static CoreBpe> {
    match name {
        "o200k_base" => Some(O200K_BASE.get_or_init(encoding::o200k_base)),
        "o200k_harmony" => Some(O200K_HARMONY.get_or_init(encoding::o200k_harmony)),
        _ => None,
    }
}

/// Get a cached tokenizer by model name.
///
/// Supports OpenAI, Meta, DeepSeek, Qwen, and Mistral models.
/// Maps model name prefixes to their encoding.
/// Returns `None` for unknown models or models whose encoding is not
/// embedded in this build.
pub fn encoding_for_model(model: &str) -> Option<&'static CoreBpe> {
    model_to_encoding(model).and_then(get_encoding)
}

/// Map a model name to its encoding name.
///
/// Returns the encoding name (e.g. `"o200k_base"`) for the given model,
/// or `None` for unknown models. Supports OpenAI, Meta, DeepSeek, Qwen, and Mistral models.
pub fn model_to_encoding(model: &str) -> Option<&'static str> {
    // Strip the `ft:` prefix used for fine-tuned model IDs
    // (e.g. `ft:gpt-4o:my-org::abc123` → `gpt-4o:my-org::abc123`).
    let model = model.strip_prefix("ft:").unwrap_or(model);

    // order matters: more specific prefixes must come before less specific ones.
    // e.g. "gpt-4o" must be checked before "gpt-4" since starts_with("gpt-4")
    // would also match "gpt-4o".

    // o200k_harmony — OpenAI open-source gpt-oss family (harmony chat format).
    // Must precede the o200k_base block to avoid being shadowed by an unrelated
    // prefix match.
    if model.starts_with("gpt-oss") {
        return Some("o200k_harmony");
    }

    // o200k_base models (newest first)
    if model.starts_with("o4-mini")
        || model.starts_with("o3")
        || model.starts_with("o1")
        || model.starts_with("gpt-5")
        || model.starts_with("gpt-4.5")
        || model.starts_with("gpt-4.1")
        || model.starts_with("gpt-4o")
        || model.starts_with("chatgpt-4o")
    {
        return Some("o200k_base");
    }

    // cl100k_base models
    // davinci-002 / babbage-002 must be checked here (before the r50k_base block)
    // since they use the cl100k tokenizer despite sharing a name root with the
    // r50k davinci/babbage models.
    if model.starts_with("gpt-4")
        || model.starts_with("gpt-3.5")
        || model.starts_with("gpt-35-turbo")
        || model.starts_with("davinci-002")
        || model.starts_with("babbage-002")
        || model.starts_with("text-embedding-ada")
        || model.starts_with("text-embedding-3")
    {
        return Some("cl100k_base");
    }

    // p50k_base models
    if model.starts_with("text-davinci-003")
        || model.starts_with("text-davinci-002")
        || model.starts_with("code-davinci")
        || model.starts_with("code-cushman")
    {
        return Some("p50k_base");
    }

    // r50k_base models (gpt2 / gpt-2 share the same encoding)
    if model.starts_with("text-davinci-001")
        || model.starts_with("text-curie")
        || model.starts_with("text-babbage")
        || model.starts_with("text-ada")
        || model.starts_with("davinci")
        || model.starts_with("curie")
        || model.starts_with("babbage")
        || model.starts_with("ada")
        || model.starts_with("gpt-2")
        || model.starts_with("gpt2")
    {
        return Some("r50k_base");
    }

    // llama models (llama3 encoding covers all llama 3.x and 4.x)
    if model.starts_with("llama-")
        || model.starts_with("llama3")
        || model.starts_with("llama4")
        || model.starts_with("Llama-")
        || model.starts_with("Meta-Llama-")
    {
        return Some("llama3");
    }

    // deepseek models
    if model.starts_with("deepseek") || model.starts_with("DeepSeek") {
        return Some("deepseek_v3");
    }

    // qwen models
    if model.starts_with("qwen") || model.starts_with("Qwen") {
        return Some("qwen2");
    }

    // mistral / mixtral / codestral / pixtral models
    if model.starts_with("mistral")
        || model.starts_with("Mistral")
        || model.starts_with("mixtral")
        || model.starts_with("Mixtral")
        || model.starts_with("codestral")
        || model.starts_with("Codestral")
        || model.starts_with("pixtral")
        || model.starts_with("Pixtral")
    {
        return Some("mistral_v3");
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // encoding lookup

    #[test]
    fn test_get_encoding_known() {
        for name in ["o200k_base", "o200k_harmony"] {
            assert!(get_encoding(name).is_some(), "missing encoding: {name}");
        }
    }

    #[test]
    fn test_get_encoding_unknown() {
        assert!(get_encoding("nonexistent").is_none());
    }

    #[test]
    fn test_o200k_harmony_roundtrip() {
        let enc = get_encoding("o200k_harmony").unwrap();
        let text = "hello world, 你好世界 🚀";
        let decoded = enc.decode(&enc.encode(text));
        assert_eq!(std::str::from_utf8(&decoded).unwrap(), text);
    }

    #[test]
    fn test_o200k_harmony_matches_base_for_plain_text() {
        // Same merge ranks + regex → ordinary text encodes identically.
        let base = get_encoding("o200k_base").unwrap();
        let harmony = get_encoding("o200k_harmony").unwrap();
        for text in ["hello world", "the quick brown fox", "你好世界 🚀"] {
            assert_eq!(base.encode(text), harmony.encode(text), "{text}");
        }
    }

    #[test]
    fn test_encoding_for_gpt_oss() {
        assert_eq!(model_to_encoding("gpt-oss-20b"), Some("o200k_harmony"));
        assert_eq!(model_to_encoding("gpt-oss-120b"), Some("o200k_harmony"));
        assert_ne!(model_to_encoding("gpt-oss-20b"), Some("o200k_base"));
    }

    // model mapping

    #[test]
    fn test_encoding_for_latest_openai_models() {
        for model in [
            "gpt-4o",
            "gpt-4o-mini",
            "o1",
            "o1-mini",
            "o3",
            "o3-mini",
            "o4-mini",
        ] {
            let enc = encoding_for_model(model);
            assert!(enc.is_some(), "no encoding for {model}");
        }
    }

    #[test]
    fn test_encoding_for_gpt5_family() {
        for m in ["gpt-5", "gpt-5-turbo", "gpt-4.5", "gpt-4.5-preview"] {
            assert_eq!(model_to_encoding(m), Some("o200k_base"), "{m}");
        }
    }

    #[test]
    fn test_encoding_for_davinci_babbage_002() {
        // Regression: these were incorrectly routed to r50k_base by
        // starts_with("davinci")/("babbage"). They use cl100k_base upstream.
        assert_eq!(model_to_encoding("davinci-002"), Some("cl100k_base"));
        assert_eq!(model_to_encoding("babbage-002"), Some("cl100k_base"));
    }

    #[test]
    fn test_encoding_for_finetuned_models() {
        assert_eq!(
            model_to_encoding("ft:gpt-4o:my-org::abc123"),
            Some("o200k_base")
        );
        assert_eq!(model_to_encoding("ft:gpt-4:org::xyz"), Some("cl100k_base"));
    }

    #[test]
    fn test_encoding_for_model_unknown() {
        assert!(encoding_for_model("unknown-model").is_none());
    }

    // encode/decode roundtrip

    #[test]
    fn test_o200k_roundtrip() {
        let enc = get_encoding("o200k_base").unwrap();
        let text = "hello world, 你好世界 🚀";
        let decoded = enc.decode(&enc.encode(text));
        assert_eq!(std::str::from_utf8(&decoded).unwrap(), text);
    }

    #[test]
    fn test_unicode_roundtrip() {
        let enc = get_encoding("o200k_base").unwrap();
        let text = "café résumé naïve über 日本語 한국어 العربية";
        let decoded = enc.decode(&enc.encode(text));
        assert_eq!(std::str::from_utf8(&decoded).unwrap(), text);
    }

    // count

    #[test]
    fn test_count_equals_encode_len() {
        let enc = get_encoding("o200k_base").unwrap();
        for text in [
            "hello world",
            "The quick brown fox.",
            "你好世界",
            "",
            "a",
            "  \n\n  ",
        ] {
            assert_eq!(
                enc.count(text),
                enc.encode(text).len(),
                "mismatch for {text:?}"
            );
        }
    }

    // special tokens

    #[test]
    fn test_o200k_special_tokens() {
        let enc = get_encoding("o200k_base").unwrap();
        let text = "hello<|endoftext|>world";
        let with = enc.encode_with_special_tokens(text);
        assert!(with.contains(&199999)); // o200k endoftext id
        let without = enc.encode(text);
        assert!(!without.contains(&199999));
    }

    // edge cases

    #[test]
    fn test_empty_input() {
        let enc = get_encoding("o200k_base").unwrap();
        assert!(enc.encode("").is_empty());
        assert_eq!(enc.count(""), 0);
    }

    #[test]
    fn test_cached_instance_is_same() {
        let a = get_encoding("o200k_base").unwrap() as *const CoreBpe;
        let b = get_encoding("o200k_base").unwrap() as *const CoreBpe;
        assert_eq!(a, b);
    }

    #[test]
    fn test_long_text_roundtrip() {
        let enc = get_encoding("o200k_base").unwrap();
        let text = "word ".repeat(10000);
        let decoded = enc.decode(&enc.encode(&text));
        assert_eq!(std::str::from_utf8(&decoded).unwrap(), text);
    }

    #[test]
    fn test_whitespace_roundtrip() {
        let enc = get_encoding("o200k_base").unwrap();
        for text in [" ", "  ", "\n", "\t", "  \n  \n  "] {
            let decoded = enc.decode(&enc.encode(text));
            assert_eq!(
                std::str::from_utf8(&decoded).unwrap(),
                text,
                "failed for {text:?}"
            );
        }
    }

    #[test]
    fn test_encoding_deterministic() {
        let enc = get_encoding("o200k_base").unwrap();
        let text = "deterministic check";
        assert_eq!(enc.encode(text), enc.encode(text));
    }

    // exact token sequence tests verified against the upstream test fixtures
    #[test]
    fn test_exact_tokens_hello_world() {
        let enc = get_encoding("o200k_base").unwrap();
        assert_eq!(enc.encode("hello world"), vec![24912, 2375]);
    }

    #[test]
    fn test_exact_tokens_unicode() {
        let enc = get_encoding("o200k_base").unwrap();
        assert_eq!(enc.encode("你好世界"), vec![177519, 28428]);
    }

    #[test]
    fn test_exact_tokens_empty() {
        let enc = get_encoding("o200k_base").unwrap();
        assert_eq!(enc.encode(""), Vec::<u32>::new());
    }

    // decode_to_string

    #[test]
    fn test_decode_to_string_valid() {
        let enc = get_encoding("o200k_base").unwrap();
        let tokens = enc.encode("hello world");
        assert_eq!(enc.decode_to_string(&tokens).unwrap(), "hello world");
    }

    #[test]
    fn test_decode_to_string_empty() {
        let enc = get_encoding("o200k_base").unwrap();
        assert_eq!(enc.decode_to_string(&[]).unwrap(), "");
    }

    #[test]
    fn test_decode_to_string_unicode() {
        let enc = get_encoding("o200k_base").unwrap();
        let text = "日本語テスト 🎉";
        let tokens = enc.encode(text);
        assert_eq!(enc.decode_to_string(&tokens).unwrap(), text);
    }

    // model_to_encoding

    #[test]
    fn test_model_to_encoding_o200k() {
        for model in [
            "gpt-4o",
            "gpt-4.1",
            "gpt-4.1-mini",
            "gpt-4.1-nano",
            "o1",
            "o3",
            "o3-pro",
            "o4-mini",
            "chatgpt-4o",
        ] {
            assert_eq!(
                model_to_encoding(model),
                Some("o200k_base"),
                "wrong encoding for {model}"
            );
        }
    }

    #[test]
    fn test_model_to_encoding_cl100k() {
        for model in [
            "gpt-4",
            "gpt-3.5-turbo",
            "text-embedding-ada-002",
            "text-embedding-3-small",
        ] {
            assert_eq!(
                model_to_encoding(model),
                Some("cl100k_base"),
                "wrong encoding for {model}"
            );
        }
    }

    #[test]
    fn test_model_to_encoding_p50k() {
        for model in [
            "text-davinci-003",
            "text-davinci-002",
            "code-davinci-002",
            "code-cushman-001",
        ] {
            assert_eq!(
                model_to_encoding(model),
                Some("p50k_base"),
                "wrong encoding for {model}"
            );
        }
    }

    #[test]
    fn test_model_to_encoding_r50k() {
        for model in ["text-davinci-001", "davinci", "curie", "babbage", "ada"] {
            assert_eq!(
                model_to_encoding(model),
                Some("r50k_base"),
                "wrong encoding for {model}"
            );
        }
    }

    #[test]
    fn test_model_to_encoding_llama3() {
        for model in [
            "llama-3.1-70b",
            "llama3-8b",
            "Meta-Llama-3.1-8B",
            "llama-4-scout",
            "llama-4-maverick",
        ] {
            assert_eq!(
                model_to_encoding(model),
                Some("llama3"),
                "wrong encoding for {model}"
            );
        }
    }

    #[test]
    fn test_model_to_encoding_deepseek() {
        for model in ["deepseek-v3", "DeepSeek-R1", "deepseek-chat"] {
            assert_eq!(
                model_to_encoding(model),
                Some("deepseek_v3"),
                "wrong encoding for {model}"
            );
        }
    }

    #[test]
    fn test_model_to_encoding_qwen() {
        for model in [
            "qwen2.5-72b",
            "Qwen2.5-7B",
            "qwen3-32b",
            "qwen3-max",
            "qwen3-coder",
        ] {
            assert_eq!(
                model_to_encoding(model),
                Some("qwen2"),
                "wrong encoding for {model}"
            );
        }
    }

    #[test]
    fn test_model_to_encoding_mistral() {
        for model in [
            "mistral-small-latest",
            "Mistral-Small-24B",
            "mixtral-8x7b",
            "codestral",
            "Codestral",
            "pixtral-large",
            "Pixtral-Large",
        ] {
            assert_eq!(
                model_to_encoding(model),
                Some("mistral_v3"),
                "wrong encoding for {model}"
            );
        }
    }

    #[test]
    fn test_model_to_encoding_unknown() {
        assert_eq!(model_to_encoding("unknown-model"), None);
    }

    // vocab_size / num_special_tokens

    #[test]
    fn test_vocab_sizes() {
        let cases: &[(&str, usize)] = &[("o200k_base", 199998)];
        for &(name, expected) in cases {
            let enc = get_encoding(name).unwrap();
            assert_eq!(enc.vocab_size(), expected, "vocab_size mismatch for {name}");
        }
    }

    // regression: gpt-4o must resolve to o200k, not cl100k (prefix order matters)
    #[test]
    fn test_model_to_encoding_gpt4o_vs_gpt4() {
        assert_eq!(model_to_encoding("gpt-4o"), Some("o200k_base"));
        assert_eq!(model_to_encoding("gpt-4o-mini"), Some("o200k_base"));
        assert_eq!(model_to_encoding("gpt-4"), Some("cl100k_base"));
        assert_eq!(model_to_encoding("gpt-4-turbo"), Some("cl100k_base"));
    }

    // decode unknown token id: should silently skip
    #[test]
    fn test_decode_unknown_token_id() {
        let enc = get_encoding("o200k_base").unwrap();
        let result = enc.decode(&[u32::MAX]);
        assert!(
            result.is_empty(),
            "unknown token should be silently skipped"
        );
    }

    #[test]
    fn test_decode_mixed_known_and_unknown() {
        let enc = get_encoding("o200k_base").unwrap();
        let tokens = enc.encode("hello");
        let mut with_unknown = tokens.clone();
        with_unknown.push(u32::MAX);
        with_unknown.extend_from_slice(&enc.encode(" world"));
        let decoded = enc.decode_to_string(&with_unknown).unwrap();
        assert_eq!(decoded, "hello world");
    }

    // decode special tokens

    #[test]
    fn test_decode_special_token_o200k() {
        let enc = get_encoding("o200k_base").unwrap();
        let decoded = enc.decode(&[199999]); // <|endoftext|>
        assert_eq!(&decoded, b"<|endoftext|>");
    }

    #[test]
    fn test_decode_special_token_roundtrip() {
        let enc = get_encoding("o200k_base").unwrap();
        let text = "hello<|endoftext|>world";
        let tokens = enc.encode_with_special_tokens(text);
        let decoded = enc.decode_to_string(&tokens).unwrap();
        assert_eq!(decoded, text);
    }

    // count consistency

    #[test]
    fn test_count_consistency() {
        let text = "Hello, 世界! This is a test with mixed content 🚀 and numbers 12345.";
        let enc = get_encoding("o200k_base").unwrap();
        assert_eq!(enc.count(text), enc.encode(text).len());
    }
}
