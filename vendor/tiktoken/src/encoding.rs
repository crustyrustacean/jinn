//! Encoding definitions and data parsing for tiktoken-compatible BPE vocabularies.
//!
//! Each encoding consists of:
//! - A `.tiktoken.zst` data file (zstd-compressed, base64-encoded token → rank lines, embedded at compile time)
//! - A regex pattern that splits input text into pieces before BPE processing
//! - A set of special tokens (e.g. `<|endoftext|>`) with designated token ids
//!
//! Pattern source: <https://github.com/openai/tiktoken/blob/main/tiktoken_ext/openai_public.py>

use base64::Engine;
use rustc_hash::FxHashMap;

use crate::bpe::CoreBpe;
use crate::pretokenize::FastPath;

// embedded encoding data files — zstd-compressed, decompressed on first use via OnceLock in lib.rs
const O200K_BASE_DATA: &[u8] = include_bytes!("encodings/o200k_base.tiktoken.zst");

// o200k pattern: similar to cl100k but with finer Unicode category distinctions
// (Lu/Lt/Lm/Lo/M vs plain \p{L}), supporting better CamelCase and mixed-script splitting
pub(crate) const O200K_PATTERN: &str = concat!(
    r"[^\r\n\p{L}\p{N}]?[\p{Lu}\p{Lt}\p{Lm}\p{Lo}\p{M}]*[\p{Ll}\p{Lm}\p{Lo}\p{M}]+",
    r"(?i:'s|'t|'re|'ve|'m|'ll|'d)?",
    r"|[^\r\n\p{L}\p{N}]?[\p{Lu}\p{Lt}\p{Lm}\p{Lo}\p{M}]+[\p{Ll}\p{Lm}\p{Lo}\p{M}]*",
    r"(?i:'s|'t|'re|'ve|'m|'ll|'d)?",
    r"|\p{N}{1,3}",
    r"| ?[^\s\p{L}\p{N}]+[\r\n]*",
    r"|\s*[\r\n]+",
    r"|\s+",
);

/// Parse a zstd-compressed `.tiktoken` file into a rank map.
///
/// The compressed data is first decompressed, then parsed line by line.
/// Each line is: `<base64-encoded token bytes> <integer rank>`
/// The rank determines merge priority in the BPE algorithm (lower = merged first).
pub(crate) fn parse_tiktoken_data(compressed: &[u8]) -> FxHashMap<Vec<u8>, u32> {
    let mut decoder =
        ruzstd::decoding::StreamingDecoder::new(compressed).expect("zstd decompression failed");
    let mut data = Vec::new();
    std::io::Read::read_to_end(&mut decoder, &mut data).expect("zstd decompression failed");
    parse_tiktoken_lines(&data)
}

/// Parse raw (uncompressed) `.tiktoken` lines into a rank map.
fn parse_tiktoken_lines(data: &[u8]) -> FxHashMap<Vec<u8>, u32> {
    let engine = base64::engine::general_purpose::STANDARD;
    let content = std::str::from_utf8(data).expect("tiktoken data must be valid UTF-8");

    let mut ranks = FxHashMap::default();
    ranks.reserve(data.len() / 20); // rough estimate: ~20 bytes per line

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, ' ');
        let token_b64 = parts.next().expect("missing token");
        let rank_str = parts.next().expect("missing rank");
        let token_bytes = engine
            .decode(token_b64)
            .expect("invalid base64 in tiktoken data");
        let rank: u32 = rank_str.parse().expect("invalid rank in tiktoken data");
        ranks.insert(token_bytes, rank);
    }

    ranks
}

/// Build a special token map from `(text, id)` pairs.
fn special_tokens(pairs: &[(&str, u32)]) -> FxHashMap<Vec<u8>, u32> {
    pairs
        .iter()
        .map(|&(s, v)| (s.as_bytes().to_vec(), v))
        .collect()
}

/// Construct the o200k_base encoding (GPT-4o, o1, o3, o4-mini).
/// Vocabulary size: 199,998 regular tokens + 2 special tokens.
pub fn o200k_base() -> CoreBpe {
    let encoder = parse_tiktoken_data(O200K_BASE_DATA);
    let special = special_tokens(&[("<|endoftext|>", 199999), ("<|endofprompt|>", 200018)]);
    CoreBpe::new(encoder, special, O200K_PATTERN, FastPath::O200k)
}

/// Construct the o200k_harmony encoding (gpt-oss family / harmony chat format).
///
/// Shares merge ranks and regex with [`o200k_base`]; the only delta is the
/// special-token table — 15 named tokens (199998..=200012) plus 1075 reserved
/// placeholders (`<|reserved_200013|>`..=`<|reserved_201087|>`).
///
/// Note: `<|reserved_200018|>` shadows `<|endofprompt|>` from o200k_base
/// at the same id; this mirrors the upstream Python implementation.
pub fn o200k_harmony() -> CoreBpe {
    let encoder = parse_tiktoken_data(O200K_BASE_DATA);
    let mut special: FxHashMap<Vec<u8>, u32> = FxHashMap::default();
    for (name, id) in [
        ("<|startoftext|>", 199998_u32),
        ("<|endoftext|>", 199999),
        ("<|reserved_200000|>", 200000),
        ("<|reserved_200001|>", 200001),
        ("<|return|>", 200002),
        ("<|constrain|>", 200003),
        ("<|reserved_200004|>", 200004),
        ("<|channel|>", 200005),
        ("<|start|>", 200006),
        ("<|end|>", 200007),
        ("<|message|>", 200008),
        ("<|reserved_200009|>", 200009),
        ("<|reserved_200010|>", 200010),
        ("<|reserved_200011|>", 200011),
        ("<|call|>", 200012),
    ] {
        special.insert(name.as_bytes().to_vec(), id);
    }
    for id in 200013..=201087_u32 {
        special.insert(format!("<|reserved_{id}|>").into_bytes(), id);
    }
    CoreBpe::new(encoder, special, O200K_PATTERN, FastPath::O200k)
}

/// Expose o200k rank map for internal tests (e.g. Vocab equivalence)
#[cfg(test)]
pub(crate) fn parse_tiktoken_data_for_test() -> FxHashMap<Vec<u8>, u32> {
    parse_tiktoken_data(O200K_BASE_DATA)
}
