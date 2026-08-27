//! Code-fence-aware message splitting for Discord's 2000-char limit.
//!
//! Discord rejects messages over 2000 characters. Assistant replies can be much
//! longer, so the gateway splits them into a sequence of chunks each suffixed
//! with `(Message x of y)`.
//!
//! Splitting respects two rules:
//! 1. **Line boundaries** — a chunk never cuts a line in half.
//! 2. **Code fences** — when a chunk boundary falls *inside* an open code fence,
//!    the splitter closes the fence at the end of the chunk and reopens it at
//!    the start of the next, so every chunk is individually valid Markdown.
//!
//! A single line longer than the per-chunk capacity is still emitted whole
//! (Discord would reject a mid-line split anyway); such oversized lines are
//! rare for prose and acceptable for v1.

/// Discord's hard message length cap.
pub const DISCORD_MAX_LEN: usize = 2000;

/// Characters reserved for the `(Message x of y)` suffix plus a newline.
///
/// `(Message 999 of 999)` is the longest realistic suffix at 20 chars; reserving
/// a few extra keeps the budget conservative.
const SUFFIX_RESERVE: usize = 24;

/// Characters reserved for a close+reopen code-fence pair when a chunk boundary
/// falls inside an open fence (one trailing ```` ``` ```` + newline, one leading
/// ```` ``` ```` + newline).
const FENCE_RESERVE: usize = 8;

/// Split `text` into chunks of at most [`DISCORD_MAX_LEN`] characters, each
/// suffixed `(Message x of y)`.
///
/// See the module docs for the fence and line-boundary rules.
#[must_use]
pub fn split_message(text: &str) -> Vec<String> {
    let cap = DISCORD_MAX_LEN
        .saturating_sub(SUFFIX_RESERVE)
        .saturating_sub(FENCE_RESERVE);

    if text.is_empty() {
        return vec!["(Message 1 of 1)".to_owned()];
    }

    // Greedy line-grouping: accumulate lines into raw chunks within `cap`,
    // tracking whether each chunk ends inside an open fence.
    let mut raw_chunks: Vec<RawChunk> = Vec::new();
    let mut cur = RawChunk::new();
    for line in text.split('\n') {
        let cost = line.len() + 1; // +1 newline
        if !cur.lines.is_empty() && cur.len + cost > cap {
            raw_chunks.push(std::mem::take(&mut cur));
        }
        cur.lines.push(line);
        cur.len += cost;
    }
    if !cur.lines.is_empty() || raw_chunks.is_empty() {
        raw_chunks.push(cur);
    }

    let total = raw_chunks.len();
    let mut out: Vec<String> = Vec::with_capacity(total);
    // Whether the previous rendered chunk ended inside an open fence — the
    // current chunk must reopen it at the top.
    let mut prev_in_fence = false;
    for (i, chunk) in raw_chunks.iter().enumerate() {
        // A chunk ends inside an open fence when the fence state toggled
        // by its own delimiters, combined with whether it started in one,
        // leaves it open at the end. We compute the rendered end-state from
        // prev_in_fence and the chunk's own toggle count.
        let toggled = chunk.lines.iter().filter(|l| is_fence_delimiter(l)).count() % 2 == 1;
        let ends_in_fence = prev_in_fence ^ toggled;
        out.push(render_chunk(
            chunk,
            i + 1,
            total,
            prev_in_fence,
            ends_in_fence,
        ));
        prev_in_fence = ends_in_fence;
    }
    out
}

/// A raw group of lines before fence continuity and suffix are applied.
struct RawChunk<'a> {
    lines: Vec<&'a str>,
    len: usize,
}

impl RawChunk<'_> {
    fn new() -> Self {
        Self {
            lines: Vec::new(),
            len: 0,
        }
    }
}

impl Default for RawChunk<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// Render one [`RawChunk`] into a final message with fence continuity and the
/// `(Message x of y)` suffix.
///
/// - `prev_in_fence`: if the previous chunk ended inside an open fence, prepend
///   a fence delimiter to reopen it.
/// - `ends_in_fence`: if this chunk ends inside an open fence (after accounting
///   for prev), append a fence delimiter to close it.
fn render_chunk(
    chunk: &RawChunk,
    index: usize,
    total: usize,
    prev_in_fence: bool,
    ends_in_fence: bool,
) -> String {
    let suffix = format!("(Message {index} of {total})");

    let mut out = String::with_capacity(chunk.len + suffix.len() + FENCE_RESERVE);
    if prev_in_fence {
        out.push_str("```\n");
    }
    for line in &chunk.lines {
        out.push_str(line);
        out.push('\n');
    }
    if ends_in_fence {
        out.push_str("```\n");
    }
    out.push_str(&suffix);
    out
}

/// True if `line` is a Markdown code-fence delimiter (three or more backticks,
/// possibly with a language tag — the language only matters on opening).
fn is_fence_delimiter(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

#[cfg(test)]
mod split_tests {
    use super::split_message;
    use crate::feat::discord::message_split::DISCORD_MAX_LEN;

    #[rstest::rstest]
    #[test]
    fn short_message_is_single_chunk_with_suffix() {
        // Given a short message.
        // When splitting.
        let chunks = split_message("hi");
        // Then one chunk suffixed (Message 1 of 1).
        assert_eq!(chunks, vec!["hi\n(Message 1 of 1)".to_owned()]);
    }

    #[rstest::rstest]
    #[test]
    fn empty_message_is_just_the_suffix() {
        // Given an empty message.
        // When splitting.
        let chunks = split_message("");
        // Then one chunk is the suffix alone.
        assert_eq!(chunks, vec!["(Message 1 of 1)".to_owned()]);
    }

    #[rstest::rstest]
    #[test]
    fn long_message_splits_into_chunks_under_max() {
        // Given a message well over the limit.
        let line = "a".to_owned();
        let many: String = std::iter::repeat_n(line.as_str(), 4000)
            .collect::<Vec<_>>()
            .join("\n");
        // When splitting.
        let chunks = split_message(&many);
        // Then there is more than one chunk.
        assert!(chunks.len() > 1, "expected multiple chunks");
        // And every chunk is within the limit.
        for c in &chunks {
            assert!(
                c.len() <= DISCORD_MAX_LEN,
                "chunk len {} over limit",
                c.len()
            );
        }
        // And each chunk has a correctly-numbered suffix.
        let total = chunks.len();
        for (i, c) in chunks.iter().enumerate() {
            assert!(
                c.ends_with(&format!("(Message {} of {total})", i + 1)),
                "chunk {i} missing suffix"
            );
        }
    }

    #[rstest::rstest]
    #[test]
    fn code_fence_splitting_keeps_chunks_balanced() {
        // Given a fenced block that straddles the chunk boundary.
        let one_line = "x".to_owned();
        let mut text = String::from("```rust\n");
        for _ in 0..1900 {
            text.push_str(&one_line);
            text.push('\n');
        }
        text.push_str("```");
        // When splitting.
        let chunks = split_message(&text);
        assert!(chunks.len() > 1, "expected multiple chunks");
        // Then every rendered chunk is individually balanced (ends outside any
        // open fence), which is the real guarantee.
        for c in &chunks {
            let mut in_fence = false;
            for line in c.lines() {
                if line.trim_start().starts_with("```") {
                    in_fence = !in_fence;
                }
            }
            assert!(!in_fence, "chunk ends inside an open fence");
        }
    }
}
