//! Pre-tokenization: split text into pieces before BPE encoding.
//!
//! The [`PreTokenizer`] trait defines a regex-based splitter that partitions
//! input text into byte-range pieces. [`RegexPreTokenizer`] implements this
//! using the `regex` crate (DFA-based), with a custom whitespace lookahead
//! emulation (`adjust_whitespace_end`) that avoids `fancy-regex` entirely.

use regex::Regex;

/// Trait for splitting text into pieces before BPE encoding.
pub trait PreTokenizer: Send + Sync {
    /// Find the next match starting at or after `pos`.
    /// Returns `(start, end)` byte offsets into `text`.
    /// The `end` is already adjusted for whitespace lookahead emulation.
    fn next_match(&self, text: &str, pos: usize) -> Option<(usize, usize)>;
}

/// Selects which ASCII fast-path scanner (if any) a [`RegexPreTokenizer`] tries
/// before falling back to the regex. Chosen by the caller in `encoding.rs`,
/// which owns the pattern definitions — the pre-tokenizer itself stays unaware
/// of any specific encoding's pattern string.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FastPath {
    /// No fast path: always use the regex (for patterns without a scanner).
    /// Only reachable from tests — the two shipped o200k encodings both scan.
    #[cfg(test)]
    None,
    /// o200k_base / o200k_harmony pattern.
    O200k,
}

/// Regex-based pre-tokenizer wrapping the existing regex + whitespace adjustment logic.
pub struct RegexPreTokenizer {
    regex: Regex,
    /// Which ASCII fast-path scanner (if any) to try before the regex.
    fast: FastPath,
}

impl RegexPreTokenizer {
    pub(crate) fn new(pattern: &str, fast: FastPath) -> Self {
        Self {
            regex: Regex::new(pattern).expect("invalid regex pattern"),
            fast,
        }
    }
}

impl PreTokenizer for RegexPreTokenizer {
    #[inline]
    fn next_match(&self, text: &str, pos: usize) -> Option<(usize, usize)> {
        let bytes = text.as_bytes();
        let fast = match self.fast {
            FastPath::O200k => o200k_ascii_next(bytes, pos),
            #[cfg(test)]
            FastPath::None => None,
        };
        if let Some(r) = fast {
            return Some(r);
        }
        let mat = self.regex.find_at(text, pos)?;
        let start = mat.start();
        let end = adjust_whitespace_end(bytes, start, mat.end());
        Some((start, end))
    }
}

/// Consume a run of `\r`/`\n` starting at `k`, returning the new offset.
/// Implements the trailing `[\r\n]*` shared by several pattern rules.
#[inline]
fn take_crlf(b: &[u8], mut k: usize) -> usize {
    while k < b.len() && (b[k] == b'\r' || b[k] == b'\n') {
        k += 1;
    }
    k
}

/// Shared ASCII handler for the digit and punctuation rules. The punctuation
/// rule (` ?[^\s\p{L}\p{N}]+[\r\n]*`) is identical across patterns; the digit
/// rule's repeat cap is `\p{N}{1,3}` (o200k).
///
/// Returns `Some((i, end))` on a match, or `None` to defer to the regex (the
/// start is whitespace, or a non-ASCII byte could extend the run under Unicode
/// semantics). Caller guarantees `i < n` and `b[i] < 0x80`.
#[inline]
fn ascii_num_punct(b: &[u8], i: usize) -> Option<(usize, usize)> {
    const MAX_DIGITS: usize = 3;
    let n = b.len();
    let c0 = b[i];

    // Rule: \p{N}{1,MAX_DIGITS}
    if c0.is_ascii_digit() {
        let mut j = i;
        let mut k = 0;
        while j < n && k < MAX_DIGITS && b[j] < 0x80 && b[j].is_ascii_digit() {
            j += 1;
            k += 1;
        }
        // Fewer than max digits and a non-ASCII byte next: it may be a Unicode
        // \p{N} (superscripts, other-number) the regex would fold in — defer.
        // At the cap the regex stops regardless, so it's safe to return.
        if k < MAX_DIGITS && j < n && b[j] >= 0x80 {
            return None;
        }
        return Some((i, j));
    }

    // Rule: ` ?[^\s\p{L}\p{N}]+[\r\n]*`
    let mut j = i;
    if c0 == b' ' {
        // optional single leading space, but only if a punct run follows
        match b.get(i + 1) {
            Some(&c1)
                if c1 < 0x80
                    && !is_ascii_ws(c1)
                    && !c1.is_ascii_alphabetic()
                    && !c1.is_ascii_digit() =>
            {
                j = i + 1;
            }
            // space not followed by ASCII punct → whitespace rules → defer
            _ => return None,
        }
    }
    let cj = b[j];
    if cj < 0x80 && !is_ascii_ws(cj) && !cj.is_ascii_alphabetic() && !cj.is_ascii_digit() {
        let mut k = j;
        while k < n
            && b[k] < 0x80
            && !is_ascii_ws(b[k])
            && !b[k].is_ascii_alphabetic()
            && !b[k].is_ascii_digit()
        {
            k += 1;
        }
        // non-ASCII symbol/punct could extend the run under the regex — defer.
        if k < n && b[k] >= 0x80 {
            return None;
        }
        k = take_crlf(b, k);
        return Some((i, k));
    }

    // whitespace run (or other) → defer to regex
    None
}

/// ASCII fast-path pre-tokenizer for the o200k pattern.
///
/// The letter rules differ from cl100k: o200k splits on case
/// (`[\p{Lu}…]*[\p{Ll}…]+` then `[\p{Lu}…]+[\p{Ll}…]*`, CamelCase-aware) and
/// attaches an optional contraction suffix to the word. Within ASCII the upper
/// class is `[A-Z]` and the lower class is `[a-z]` (Lt/Lm/Lo/M are empty in
/// ASCII). Digit/punct/whitespace rules are identical to the cl100k-family rules.
#[inline]
fn o200k_ascii_next(b: &[u8], i: usize) -> Option<(usize, usize)> {
    let n = b.len();
    if i >= n {
        return None;
    }
    let c0 = b[i];
    if c0 >= 0x80 {
        return None;
    }

    // Determine the letter start `p`: either c0 itself (a letter), or one
    // leading non-CRLF non-alnum char followed by an ASCII letter.
    let p = if c0.is_ascii_alphabetic() {
        i
    } else if c0 != b'\r' && c0 != b'\n' && !c0.is_ascii_digit() {
        // eligible leading char (punct/space). Letter rule applies only if the
        // next byte is an ASCII letter; otherwise it's a digit/punct/ws piece.
        match b.get(i + 1) {
            Some(&c1) if c1 < 0x80 && c1.is_ascii_alphabetic() => i + 1,
            _ => return ascii_num_punct(b, i),
        }
    } else {
        // digit, or \r\n
        return ascii_num_punct(b, i);
    };

    // Scan the uppercase run from `p`.
    let mut q = p;
    while q < n && b[q] < 0x80 && b[q].is_ascii_uppercase() {
        q += 1;
    }
    // A non-ASCII byte after the uppercase run could be a Unicode letter/mark
    // the regex would include — defer.
    if q < n && b[q] >= 0x80 {
        return None;
    }

    let letters_end = if q > p {
        // started with uppercase(s)
        if q < n && b[q].is_ascii_lowercase() {
            // Rule A: [A-Z]*[a-z]+  (greedy uppercase, then lowercase run)
            let mut r = q;
            while r < n && b[r] < 0x80 && b[r].is_ascii_lowercase() {
                r += 1;
            }
            if r < n && b[r] >= 0x80 {
                return None;
            }
            r
        } else {
            // Rule B: [A-Z]+  (no trailing lowercase)
            q
        }
    } else {
        // b[p] is lowercase (it's a letter but not uppercase): Rule A lowercase+
        let mut r = p;
        while r < n && b[r] < 0x80 && b[r].is_ascii_lowercase() {
            r += 1;
        }
        if r < n && b[r] >= 0x80 {
            return None;
        }
        r
    };

    // Optional contraction suffix attached to the word: (?i:'s|'t|…)?
    let mut end = letters_end;
    if end < n
        && b[end] == b'\''
        && let Some(len) = match_contraction(b, end)
    {
        end += len;
    }
    Some((i, end))
}

/// Match a contraction at `b[i] == '\''`, returning its byte length (2 or 3) or
/// `None`. Case-insensitive, matching `(?i:'s|'t|'re|'ve|'m|'ll|'d)`. Shared by
/// both patterns (standalone alternative in cl100k, word suffix in o200k).
#[inline]
fn match_contraction(b: &[u8], i: usize) -> Option<usize> {
    let c1 = b.get(i + 1).copied()?.to_ascii_lowercase();
    match c1 {
        b's' | b't' | b'm' | b'd' => Some(2),
        b'r' if b.get(i + 2).map(|c| c.to_ascii_lowercase()) == Some(b'e') => Some(3),
        b'v' if b.get(i + 2).map(|c| c.to_ascii_lowercase()) == Some(b'e') => Some(3),
        b'l' if b.get(i + 2).map(|c| c.to_ascii_lowercase()) == Some(b'l') => Some(3),
        _ => None,
    }
}

/// Emulates `\s+(?!\S)|\s+` from original tiktoken patterns.
/// Pure byte-level fast path for ASCII whitespace, char-level fallback for Unicode.
#[inline]
fn adjust_whitespace_end(bytes: &[u8], start: usize, end: usize) -> usize {
    if end - start <= 1 || end >= bytes.len() {
        return end;
    }

    // fast reject: if first byte is printable ASCII (0x21..0x7E), not whitespace
    let first = bytes[start];
    if first > 0x20 && first < 0x7F {
        return end;
    }

    // ASCII fast path
    // safety: end < bytes.len() is guaranteed by the early return above
    let piece = &bytes[start..end];
    if piece.iter().all(|&b| is_ascii_ws(b)) {
        let next = bytes[end];
        if is_ascii_ws(next) {
            return end;
        }
        return end - 1;
    }

    // unicode slow path
    // safety: regex::Match boundaries are always valid UTF-8 since input is &str
    let matched = std::str::from_utf8(&bytes[start..end]).unwrap();
    if !matched.chars().all(|c| c.is_whitespace()) {
        return end;
    }
    let tail = std::str::from_utf8(&bytes[end..]).unwrap();
    let next_char = match tail.chars().next() {
        Some(c) => c,
        None => return end,
    };
    if next_char.is_whitespace() {
        return end;
    }
    let last_len = matched.chars().next_back().unwrap().len_utf8();
    // don't trim if it would make the piece empty (single multi-byte whitespace char)
    if end - last_len <= start {
        return end;
    }
    end - last_len
}

#[inline(always)]
const fn is_ascii_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_matches(pt: &dyn PreTokenizer, text: &str) -> Vec<(usize, usize)> {
        let mut result = vec![];
        let mut pos = 0;
        while let Some((start, end)) = pt.next_match(text, pos) {
            result.push((start, end));
            pos = end;
        }
        result
    }

    // Single source of truth: the real production patterns. Importing them here
    // (rather than copying) guarantees the fast-path equivalence test below
    // validate against exactly the patterns used in production.
    use crate::encoding::O200K_PATTERN;

    // verify RegexPreTokenizer matches the v2 behavior by comparing
    // piece-by-piece with v2's regex.find_at + adjust_whitespace_end
    fn v2_collect_matches(pattern: &str, text: &str) -> Vec<(usize, usize)> {
        let regex = Regex::new(pattern).unwrap();
        let bytes = text.as_bytes();
        let mut result = vec![];
        let mut pos = 0;
        while pos < text.len() {
            let mat = match regex.find_at(text, pos) {
                Some(m) => m,
                None => break,
            };
            let start = mat.start();
            let end = adjust_whitespace_end(bytes, start, mat.end());
            result.push((start, end));
            pos = end;
        }
        result
    }

    #[test]
    fn test_o200k_english() {
        let pt = RegexPreTokenizer::new(O200K_PATTERN, FastPath::O200k);
        let text = "Hello, world! CamelCase mixedScript123";
        let v2 = v2_collect_matches(O200K_PATTERN, text);
        let v3 = collect_matches(&pt, text);
        assert_eq!(v2, v3);
    }

    #[test]
    fn test_adjust_whitespace_single_byte() {
        assert_eq!(adjust_whitespace_end(b"a b", 0, 1), 1);
    }

    #[test]
    fn test_adjust_whitespace_at_end_of_input() {
        assert_eq!(adjust_whitespace_end(b"  ", 0, 2), 2);
    }

    #[test]
    fn test_adjust_whitespace_non_ws_piece() {
        assert_eq!(adjust_whitespace_end(b"hello world", 0, 5), 5);
    }

    #[test]
    fn test_adjust_whitespace_trim_before_nonws() {
        let bytes = b"  x";
        assert_eq!(adjust_whitespace_end(bytes, 0, 2), 1);
    }

    #[test]
    fn test_adjust_whitespace_no_trim_before_ws() {
        let bytes = b"   ";
        assert_eq!(adjust_whitespace_end(bytes, 0, 2), 2);
    }

    #[test]
    fn test_adjust_whitespace_unicode_slow_path() {
        let input = "\u{3000}\u{3000}x";
        let bytes = input.as_bytes();
        assert_eq!(adjust_whitespace_end(bytes, 0, 6), 3);
    }

    #[test]
    fn test_adjust_whitespace_unicode_followed_by_unicode_ws() {
        let input = "\u{3000}\u{3000}\u{3000}";
        let bytes = input.as_bytes();
        assert_eq!(adjust_whitespace_end(bytes, 0, 6), 6);
    }

    #[test]
    fn test_adjust_whitespace_single_multibyte_ws_before_nonws() {
        // U+3000 (ideographic space, 3 bytes) followed by 'x'
        // trimming the last char would make the piece empty, so it should NOT trim
        let input = "\u{3000}x";
        let bytes = input.as_bytes();
        // piece is bytes[0..3] (the ideographic space), next char is 'x' (non-ws)
        // without the protection, this would trim to bytes[0..0] which is empty
        assert_eq!(adjust_whitespace_end(bytes, 0, 3), 3);
    }

    // comprehensive comparison: run many inputs through the o200k pattern
    #[test]
    fn test_all_patterns_match_v2() {
        let texts = vec![
            "Hello, world!",
            "你好世界",
            "fn main() { }",
            "  hello  ",
            "line1\nline2\n",
            "café résumé",
            "100% of $1,000",
            "a@b.com",
            "   \t\n   ",
            "",
            "a",
            "hello world! 你好 🚀 test 123",
        ];

        let pt = RegexPreTokenizer::new(O200K_PATTERN, FastPath::O200k);
        for text in &texts {
            let v2 = v2_collect_matches(O200K_PATTERN, text);
            let v3 = collect_matches(&pt, text);
            assert_eq!(v2, v3, "mismatch for text: {text:?}");
        }
    }

    // whitespace-adjustment sanity through the public path
    #[test]
    fn test_empty_input() {
        let pt = RegexPreTokenizer::new(O200K_PATTERN, FastPath::O200k);
        assert_eq!(collect_matches(&pt, ""), vec![]);
    }
}
