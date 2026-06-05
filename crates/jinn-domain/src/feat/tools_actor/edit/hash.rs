//! Hashline hash computation, anchor parsing, and formatting.
//!
//! Provides the xxHash32-based line hash algorithm used to generate `LINE#HASH`
//! anchors for the hashline edit protocol. Each line gets a 2-character hash
//! computed from its content (after normalization). Lines with no alphanumeric
//! characters also mix in the line number as a seed to avoid collisions between
//! structurally identical lines (e.g., blank lines or bare braces).

use std::fmt::Write as _;

use std::hash::Hasher;

use twox_hash::XxHash32;
use unicode_segmentation::UnicodeSegmentation;

// ─── Constants ──────────────────────────────────────────────────────────

/// Custom 16-character hash alphabet. Deliberately excludes:
/// - Hex digits A–F (prevents confusion with hex literals)
/// - Visually confusable letters: D, G, I, L, O (look like digits)
/// - Common vowels A, E, I, O, U (prevents accidental English words)
///
/// This makes hash references like "5#MQ" unambiguous.
pub(crate) const NIBBLE_STR: &str = "ZPMQVRWSNKTXJBYH";

/// Pre-computed lookup table: byte value → 2-character hash string.
/// Built at runtime from the nibble alphabet.
pub(crate) static DICT: std::sync::LazyLock<[String; 256]> = std::sync::LazyLock::new(|| {
    let nibble_bytes = b"ZPMQVRWSNKTXJBYH";
    let mut table: [String; 256] = core::array::from_fn(|_| String::new());
    let mut i = 0;
    while i < 256 {
        let hi = nibble_bytes[i >> 4] as char;
        let lo = nibble_bytes[i & 0x0f] as char;
        table[i] = format!("{hi}{lo}");
        i += 1;
    }
    table
});

// ─── Types ──────────────────────────────────────────────────────────────

/// A validated line reference: a 1-indexed line number paired with an expected
/// 2-character hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    /// 1-indexed line number.
    pub line: usize,
    /// Expected 2-character hash from the `NIBBLE_STR` alphabet.
    pub hash: String,
}

/// One stale anchor in a mismatch error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashMismatch {
    /// The line number that mismatched.
    pub line: usize,
    /// The hash the caller expected.
    pub expected: String,
    /// The hash the file actually has.
    pub actual: String,
}

// ─── Hash computation ───────────────────────────────────────────────────

/// Computes a 2-character hash tag for a single file line.
///
/// # Algorithm
///
/// 1. Strip trailing CR and trailing whitespace (normalize line endings).
/// 2. If the line contains no letter or digit (blank lines, punctuation-only),
///    mix the 1-indexed line number into the xxHash32 seed so structurally
///    identical lines at different positions get different hashes.
/// 3. Compute xxHash32 of the normalized UTF-8 bytes using the seed.
/// 4. Take the lowest byte, split into two 4-bit nibbles, map each through
///    `NIBBLE_STR`.
///
/// The result is always exactly 2 characters from `NIBBLE_STR`.
pub fn compute_line_hash(line_num: usize, line: &str) -> &str {
    // Strip trailing CR and trailing whitespace
    let normalized = line.trim_end_matches('\r').trim_end();

    let seed = if has_significant_char(normalized) {
        0u32
    } else {
        // Mix line number as seed for non-alphanumeric lines
        line_num as u32
    };

    let mut hasher = XxHash32::with_seed(seed);
    hasher.write(normalized.as_bytes());
    let sum = hasher.finish();
    let lo = sum as u8;
    &DICT[lo as usize]
}

/// Returns `true` if the line contains at least one letter or digit.
fn has_significant_char(s: &str) -> bool {
    s.graphemes(true)
        .any(|g| g.chars().any(char::is_alphanumeric))
}

/// Formats a line reference as `"LINE#HASH"` (e.g., `"5#WS"`).
pub fn format_tag(line_num: usize, line: &str) -> String {
    let hash = compute_line_hash(line_num, line);
    format!("{line_num}#{hash}")
}

/// Formats a hashline-annotated region of lines for display.
///
/// Each line is formatted as `LINE#HASH|content` with line numbers padded to
/// `line_number_width` characters.
pub fn format_hashline_region(lines: &[&str], start_line: usize) -> String {
    let line_number_width = format!("{}", start_line + lines.len().saturating_sub(1)).len();
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        let line_num = start_line + i;
        let hash = compute_line_hash(line_num, line);
        let _ = writeln!(out, "{line_num:>line_number_width$}#{hash}|{line}");
    }
    out
}

// ─── Anchor parsing ─────────────────────────────────────────────────────

/// Parses a `LINE#HASH` anchor string (e.g., `"5#WS"`).
///
/// Lenient: tolerates leading `>`, `+`, `-` and whitespace (from diff/mismatch
/// display), and an optional trailing display suffix (`:...`).
///
/// # Errors
///
/// Returns a descriptive error if the anchor is malformed.
pub fn parse_anchor(ref_str: &str) -> Result<Anchor, String> {
    // Strip leading diff markers and whitespace
    let core = ref_str
        .trim_start()
        .trim_start_matches(['>', '+', '-'])
        .trim_start()
        .trim_end();

    // Match "LINE#HASH" with optional display suffix
    let re = regex::Regex::new(r"^(\d+)\s*#([^\s|]+)(?:\s*\|(.*))?$").map_err(|e| e.to_string())?;
    let caps = re
        .captures(core)
        .ok_or_else(|| diagnose_line_ref(ref_str))?;

    let line_str = caps
        .get(1)
        .ok_or_else(|| format!("[E_BAD_REF] capture group 1 missing in \"{ref_str}\""))?
        .as_str();
    let hash_str = caps
        .get(2)
        .ok_or_else(|| format!("[E_BAD_REF] capture group 2 missing in \"{ref_str}\""))?
        .as_str();

    let line: usize = line_str
        .parse()
        .map_err(|_| format!("[E_BAD_REF] Line number must be >= 1 in \"{ref_str}\"."))?;
    if line < 1 {
        return Err(format!(
            "[E_BAD_REF] Line number must be >= 1 in \"{ref_str}\"."
        ));
    }

    if hash_str.len() != 2 {
        return Err(format!(
            "[E_BAD_REF] Invalid line reference \"{ref_str}\": hash must be exactly 2 characters from {NIBBLE_STR}."
        ));
    }

    let hash_alpha_re =
        regex::Regex::new(&format!("^[{NIBBLE_STR}]+$")).map_err(|e| e.to_string())?;
    if !hash_alpha_re.is_match(hash_str) {
        return Err(format!(
            "[E_BAD_REF] Invalid line reference \"{ref_str}\": hash uses invalid characters, hashes use alphabet {NIBBLE_STR} only."
        ));
    }

    Ok(Anchor {
        line,
        hash: hash_str.to_owned(),
    })
}

/// Produces a diagnostic error for an unparseable line reference.
fn diagnose_line_ref(ref_str: &str) -> String {
    let trimmed = ref_str.trim();
    let core = ref_str
        .trim_start()
        .trim_start_matches(['>', '+', '-'])
        .trim_start()
        .trim_end();

    if core.is_empty() {
        return format!(
            "[E_BAD_REF] Invalid line reference \"{ref_str}\". Expected \"LINE#HASH\" (e.g. \"5#MQ\")."
        );
    }

    // Check for common mistakes
    let digits_only = regex::Regex::new(r"^\d+\s*$").unwrap();
    if digits_only.is_match(core) {
        return format!(
            "[E_BAD_REF] Invalid line reference \"{ref_str}\": missing hash, use \"LINE#HASH\" from read output (e.g. \"5#MQ\")."
        );
    }

    let colon_sep = regex::Regex::new(r"^\d+\s*:").unwrap();
    if colon_sep.is_match(core) {
        return format!(
            "[E_BAD_REF] Invalid line reference \"{ref_str}\": wrong separator, use \"LINE#HASH\" instead of \"LINE:...\"."
        );
    }

    format!(
        "[E_BAD_REF] Invalid line reference \"{trimmed}\". Expected \"LINE#HASH\" (e.g. \"5#MQ\")."
    )
}

// ─── Display prefix rejection ───────────────────────────────────────────

/// Regex detecting hashline display prefixes in edit payloads.
///
/// The model must send literal file content for `lines`, not the rendered
/// read/diff form.
static DISPLAY_PREFIX_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^\s*(?:>>>|>>)?\s*(?:\d+\s*#\s*|#\s*)[ZPMQVRWSNKTXJBYH]{2}\|")
        .expect("valid regex")
});

/// Regex detecting diff-plus display prefixes.
static DISPLAY_PREFIX_PLUS_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^\+\s*(?:\d+\s*#\s*|#\s*)[ZPMQVRWSNKTXJBYH]{2}\|").expect("valid regex")
});

/// Regex detecting diff-minus display prefixes.
static DIFF_MINUS_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"^-\s*\d+\s{4}").expect("valid regex"));

/// Rejects hashline display prefixes in edit payloads.
///
/// Returns an error message if any line contains a display prefix.
pub fn assert_no_display_prefixes(lines: &[String]) -> Result<(), String> {
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if DISPLAY_PREFIX_RE.is_match(line)
            || DISPLAY_PREFIX_PLUS_RE.is_match(line)
            || DIFF_MINUS_RE.is_match(line)
        {
            return Err(format!(
                "[E_INVALID_PATCH] \"lines\" must contain literal file content, not rendered \"LINE#HASH|\" or diff \"+/-\" prefixes. Offending line: {line}"
            ));
        }
    }
    Ok(())
}

// ─── Visible lines helper ───────────────────────────────────────────────

/// Returns the visible (non-sentinel) lines of a file.
///
/// Files ending with `\n` produce an empty trailing element when split;
/// this strips that sentinel. Returns an empty vec for empty content.
pub fn get_visible_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let lines: Vec<&str> = text.split('\n').collect();
    if text.ends_with('\n') {
        lines[..lines.len() - 1].to_vec()
    } else {
        lines
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;

    #[rstest::rstest]
    fn hash_returns_two_chars_from_alphabet() {
        // Given a line with content.
        let hash = compute_line_hash(1, "hello");

        // Then it's exactly 2 chars from the alphabet.
        assert_eq!(hash.len(), 2);
        assert!(
            hash.chars().all(|c| NIBBLE_STR.contains(c)),
            "hash '{hash}' contains characters outside the alphabet"
        );
    }

    #[rstest::rstest]
    fn hash_stability() {
        // Given the same input.
        // When computing hash multiple times.
        // Then it always produces the same result.
        for _ in 0..10 {
            let h1 = compute_line_hash(1, "func main() {");
            let h2 = compute_line_hash(1, "func main() {");
            assert_eq!(h1, h2);
            assert_eq!(h1.len(), 2);
        }
    }

    #[rstest::rstest]
    fn hash_trailing_whitespace_stripped() {
        // Given lines that differ only in trailing whitespace.
        let h1 = compute_line_hash(1, "hello");
        let h2 = compute_line_hash(1, "hello   ");
        let h3 = compute_line_hash(1, "hello\r");

        // Then they produce the same hash.
        assert_eq!(h1, h2, "trailing spaces changed hash");
        assert_eq!(h1, h3, "trailing CR changed hash");
    }

    #[rstest::rstest]
    fn hash_internal_spaces_preserved() {
        // Given lines that differ in internal spacing.
        let h1 = compute_line_hash(1, "a b");
        let h2 = compute_line_hash(1, "ab");

        // Then they produce different hashes.
        assert_ne!(h1, h2);
    }

    #[rstest::rstest]
    fn hash_non_alpha_seeding() {
        // Given blank lines at different positions.
        let h1 = compute_line_hash(1, "");
        let h10 = compute_line_hash(10, "");

        // Then they produce different hashes (seeded by line number).
        assert_ne!(h1, h10);
    }

    #[rstest::rstest]
    fn hash_alpha_lines_not_seeded() {
        // Given lines with alpha content at different positions.
        let h1 = compute_line_hash(1, "function foo()");
        let h99 = compute_line_hash(99, "function foo()");

        // Then they produce the same hash (not seeded by line number).
        assert_eq!(h1, h99);
    }

    #[rstest::rstest]
    fn hash_alphabet_only() {
        // Given various line types.
        let lines = ["", "  ", "{", "}", "// comment", "func foo() {"];
        for (i, line) in lines.iter().enumerate() {
            let h = compute_line_hash(i + 1, line);
            for c in h.chars() {
                assert!(
                    NIBBLE_STR.contains(c),
                    "hash '{h}' for line {line:?} contains '{c}' not in alphabet"
                );
            }
        }
    }

    #[rstest::rstest]
    fn hash_non_alpha_seeding_gives_diverse_hashes() {
        // Given 20 blank lines.
        let mut seen = std::collections::HashSet::new();
        for i in 1..=20 {
            seen.insert(compute_line_hash(i, ""));
        }

        // Then we get at least 5 distinct hashes (avoiding all-same collision).
        assert!(
            seen.len() >= 5,
            "too many blank-line hash collisions: only {} distinct hashes",
            seen.len()
        );
    }

    // ─── Anchor parsing tests ───────────────────────────────────────

    #[rstest::rstest]
    fn parse_anchor_valid() {
        // Given a valid anchor.
        let a = parse_anchor("5#WS").expect("should parse");

        // Then it extracts line and hash.
        assert_eq!(a.line, 5);
        assert_eq!(a.hash, "WS");
    }

    #[rstest::rstest]
    fn parse_anchor_with_display_suffix() {
        // Given an anchor with a display suffix.
        let a = parse_anchor("5#WS|func main() {").expect("should parse");

        // Then it extracts line and hash, ignoring the suffix.
        assert_eq!(a.line, 5);
        assert_eq!(a.hash, "WS");
    }

    #[rstest::rstest]
    fn parse_anchor_tolerates_diff_prefix() {
        // Given an anchor with a diff prefix.
        let a = parse_anchor(">>> 5#WS|content").expect("should parse");

        // Then it still parses correctly.
        assert_eq!(a.line, 5);
        assert_eq!(a.hash, "WS");
    }

    #[rstest::rstest]
    fn parse_anchor_rejects_invalid() {
        // Given an invalid anchor.
        let result = parse_anchor("abc");

        // Then it fails.
        assert!(result.is_err());
    }

    #[rstest::rstest]
    fn parse_anchor_rejects_colon_separator() {
        // Given a colon-separated reference.
        let result = parse_anchor("5:WS");

        // Then it fails with a helpful message.
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("wrong separator"));
    }

    #[rstest::rstest]
    fn parse_anchor_rejects_zero_line() {
        // Given a zero line number.
        let result = parse_anchor("0#WS");

        // Then it fails.
        assert!(result.is_err());
    }

    #[rstest::rstest]
    fn parse_anchor_rejects_invalid_hash_chars() {
        // Given a hash with invalid characters.
        let result = parse_anchor("5#ab");

        // Then it fails.
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid characters"));
    }

    // ─── Display prefix rejection tests ─────────────────────────────

    #[rstest::rstest]
    fn display_prefix_rejected() {
        // Given lines with a display prefix.
        let lines = vec!["5#WS|content".to_owned()];

        // Then they are rejected.
        assert!(assert_no_display_prefixes(&lines).is_err());
    }

    #[rstest::rstest]
    fn literal_content_accepted() {
        // Given lines with literal content.
        let lines = vec!["normal code".to_owned()];

        // Then they are accepted.
        assert!(assert_no_display_prefixes(&lines).is_ok());
    }

    // ─── Visible lines tests ────────────────────────────────────────

    #[rstest::rstest]
    fn get_visible_lines_trailing_newline() {
        // Given content with a trailing newline.
        let lines = get_visible_lines("a\nb\nc\n");

        // Then the sentinel is stripped.
        assert_eq!(lines, vec!["a", "b", "c"]);
    }

    #[rstest::rstest]
    fn get_visible_lines_no_trailing_newline() {
        // Given content without a trailing newline.
        let lines = get_visible_lines("a\nb\nc");

        // Then all lines are returned.
        assert_eq!(lines, vec!["a", "b", "c"]);
    }

    #[rstest::rstest]
    fn get_visible_lines_empty() {
        // Given empty content.
        let lines = get_visible_lines("");

        // Then no lines are returned.
        assert!(lines.is_empty());
    }
}
