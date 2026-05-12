//! Word-wrap computation for the chat input box.
//!
//! Maps buffer text + available width → a list of visual lines with grapheme
//! index ranges. Each visual line tracks its start/end grapheme position in the
//! original text, whether it's a wrapped continuation, and which logical line
//! it belongs to.

use unicode_segmentation::UnicodeSegmentation;

/// A single visual line produced by word-wrapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedLine {
    /// Grapheme index in the original text where this visual line starts.
    pub grapheme_start: usize,
    /// Grapheme index in the original text where this visual line ends (exclusive).
    pub grapheme_end: usize,
    /// Whether this is a wrapped continuation (not starting from a `\n`).
    pub is_continuation: bool,
    /// Which logical line (0-indexed, separated by `\n`) this belongs to.
    pub logical_line_index: usize,
}

/// Wraps `text` to the given `width` (in grapheme columns), returning visual lines.
///
/// Splits on `\n` first, then word-wraps each logical line. Each grapheme is
/// assumed to be 1 column wide (which matches how the input element positions
/// characters). Words are broken if they exceed `width`.
///
/// Returns at least one [`WrappedLine`] even for empty text.
///
/// When `width` is 0, each logical line maps to one visual line (no wrapping).
pub fn wrap_text(text: &str, width: usize) -> Vec<WrappedLine> {
    let graphemes: Vec<&str> = text.graphemes(true).collect();
    let mut lines = Vec::new();
    let mut grapheme_offset = 0;
    let mut logical_index = 0;

    // Split into logical lines by `\n`.
    let mut logical_start = 0;
    for (i, g) in graphemes.iter().enumerate() {
        if *g == "\n" {
            let logical_len = i - logical_start;
            wrap_logical_line(
                &graphemes[logical_start..i],
                width,
                grapheme_offset,
                logical_index,
                &mut lines,
            );
            grapheme_offset += logical_len + 1; // +1 for `\n`
            logical_start = i + 1;
            logical_index += 1;
        }
    }

    // Handle the last logical line (no trailing `\n`).
    wrap_logical_line(
        &graphemes[logical_start..],
        width,
        grapheme_offset,
        logical_index,
        &mut lines,
    );

    if lines.is_empty() {
        lines.push(WrappedLine {
            grapheme_start: 0,
            grapheme_end: 0,
            is_continuation: false,
            logical_line_index: 0,
        });
    }

    lines
}

/// Wraps a single logical line (no `\n` characters) into visual lines.
///
/// Each grapheme is counted as 1 column. Word boundaries are at whitespace
/// graphemes. Long words that exceed `width` are broken at grapheme boundaries.
fn wrap_logical_line(
    graphemes: &[&str],
    width: usize,
    grapheme_offset: usize,
    logical_index: usize,
    out: &mut Vec<WrappedLine>,
) {
    if graphemes.is_empty() {
        out.push(WrappedLine {
            grapheme_start: grapheme_offset,
            grapheme_end: grapheme_offset,
            is_continuation: false,
            logical_line_index: logical_index,
        });
        return;
    }

    if width == 0 {
        // Degenerate: no wrapping.
        out.push(WrappedLine {
            grapheme_start: grapheme_offset,
            grapheme_end: grapheme_offset + graphemes.len(),
            is_continuation: false,
            logical_line_index: logical_index,
        });
        return;
    }

    let mut line_start = 0; // grapheme index within this logical line
    let mut col = 0; // current column position
    let mut last_word_break = None; // grapheme index of last whitespace position
    let mut first_line = true;

    for (i, g) in graphemes.iter().enumerate() {
        let is_whitespace = g.chars().all(|c| c.is_whitespace());

        if is_whitespace && col > 0 {
            // Record potential break point: break *before* this whitespace,
            // so the whitespace goes to the next line. But actually, for
            // visual wrapping, we want to break *after* the whitespace so
            // the word stays on the current line. Let's think...
            //
            // "hello world" at width 6:
            //   "hello " (6 cols) — break after space
            //   "world" (5 cols)
            //
            // So the break point is i+1 (the grapheme after the whitespace).
            // But only if i+1 < graphemes.len() (not at end of line).
            if i + 1 < graphemes.len() {
                last_word_break = Some(i + 1);
            }
        }

        col += 1;

        if col > width {
            // Need to wrap.
            if let Some(break_pos) = last_word_break {
                // Break at the recorded word boundary.
                out.push(WrappedLine {
                    grapheme_start: grapheme_offset + line_start,
                    grapheme_end: grapheme_offset + break_pos,
                    is_continuation: !first_line,
                    logical_line_index: logical_index,
                });
                line_start = break_pos;
                first_line = false;
                // Recompute col from the remaining graphemes.
                // i+1 graphemes have been processed; break_pos started the new line.
                col = (i + 1).saturating_sub(break_pos);
                last_word_break = None;
            } else {
                // No word break found — break at the current position
                // (forced break in the middle of a long word).
                out.push(WrappedLine {
                    grapheme_start: grapheme_offset + line_start,
                    grapheme_end: grapheme_offset + i,
                    is_continuation: !first_line,
                    logical_line_index: logical_index,
                });
                line_start = i;
                first_line = false;
                col = 1;
                last_word_break = None;
            }
        }
    }

    // Remaining text after the last line break.
    if line_start < graphemes.len() {
        out.push(WrappedLine {
            grapheme_start: grapheme_offset + line_start,
            grapheme_end: grapheme_offset + graphemes.len(),
            is_continuation: !first_line,
            logical_line_index: logical_index,
        });
    } else if line_start == graphemes.len() {
        // Edge case: the last line ended exactly at the boundary.
        // Push an empty line to represent the cursor position after the last char.
        out.push(WrappedLine {
            grapheme_start: grapheme_offset + graphemes.len(),
            grapheme_end: grapheme_offset + graphemes.len(),
            is_continuation: !first_line,
            logical_line_index: logical_index,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn empty_text_returns_single_empty_line() {
        // Given empty text and width 20.
        // When wrapping.
        let lines = wrap_text("", 20);

        // Then there is exactly 1 line with start=0, end=0.
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].grapheme_start, 0);
        assert_eq!(lines[0].grapheme_end, 0);
        assert!(!lines[0].is_continuation);
        assert_eq!(lines[0].logical_line_index, 0);
    }

    #[rstest::rstest]
    fn short_text_returns_single_line() {
        // Given "hello" and width 20.
        // When wrapping.
        let lines = wrap_text("hello", 20);

        // Then there is 1 line spanning graphemes 0..5.
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].grapheme_start, 0);
        assert_eq!(lines[0].grapheme_end, 5);
        assert!(!lines[0].is_continuation);
    }

    #[rstest::rstest]
    fn long_text_wraps_at_word_boundary() {
        // Given "hello world and more" at width 10.
        // Line 1: "hello " (6 graphemes) — wrap after space at col 6
        // Line 2: "world and " (10 graphemes) — exactly fits width 10
        // Line 3: "more" (4 graphemes)
        let text = "hello world and more";

        // When wrapping at width 10.
        let lines = wrap_text(text, 10);

        // Then there are 3 lines.
        assert_eq!(lines.len(), 3);
        // And the first line covers "hello " (graphemes 0..6).
        assert_eq!(lines[0].grapheme_start, 0);
        assert_eq!(lines[0].grapheme_end, 6);
        assert!(!lines[0].is_continuation);
        // And the second line covers "world and " (graphemes 6..16).
        assert_eq!(lines[1].grapheme_start, 6);
        assert_eq!(lines[1].grapheme_end, 16);
        assert!(lines[1].is_continuation);
        // And the third line covers "more" (graphemes 16..20).
        assert_eq!(lines[2].grapheme_start, 16);
        assert_eq!(lines[2].grapheme_end, 20);
        assert!(lines[2].is_continuation);
        // And all lines belong to the same logical line.
        for line in &lines {
            assert_eq!(line.logical_line_index, 0);
        }
    }

    #[rstest::rstest]
    fn overflow_word_breaks_long_word() {
        // Given a single long word that exceeds width.
        let text = "abcdefghijklmn";

        // When wrapping at width 5.
        let lines = wrap_text(text, 10);

        // Then the word is broken into 2 lines (10 + 4).
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].grapheme_end - lines[0].grapheme_start, 10);
        assert_eq!(lines[1].grapheme_end - lines[1].grapheme_start, 4);
        assert!(lines[1].is_continuation);
    }

    #[rstest::rstest]
    fn text_with_newlines_produces_multiple_logical_lines() {
        // Given "hello\nworld" with wide width.
        // When wrapping.
        let lines = wrap_text("hello\nworld", 80);

        // Then there are 2 visual lines (no wrapping needed).
        assert_eq!(lines.len(), 2);
        // And they have different logical_line_index.
        assert_eq!(lines[0].logical_line_index, 0);
        assert_eq!(lines[1].logical_line_index, 1);
        // And the first covers "hello" (graphemes 0..5).
        assert_eq!(lines[0].grapheme_start, 0);
        assert_eq!(lines[0].grapheme_end, 5);
        // And the second covers "world" (graphemes 6..11, skipping the \n).
        assert_eq!(lines[1].grapheme_start, 6);
        assert_eq!(lines[1].grapheme_end, 11);
    }

    #[rstest::rstest]
    fn wrapping_with_newlines() {
        // Given a long first line and a short second line.
        // "hello world and more" (20 chars) + \n + "short" (5 chars)
        let text = "hello world and more\nshort";

        // When wrapping at width 10.
        let lines = wrap_text(text, 10);

        // Then the first logical line is wrapped into 3 visual lines.
        let first_logical: Vec<_> = lines.iter().filter(|l| l.logical_line_index == 0).collect();
        assert_eq!(first_logical.len(), 3);
        // And the second logical line is a single visual line.
        let second_logical: Vec<_> = lines.iter().filter(|l| l.logical_line_index == 1).collect();
        assert_eq!(second_logical.len(), 1);
        // And the second logical line starts at the correct offset.
        // "hello world and more\n" = 21 graphemes, so "short" starts at 21.
        assert_eq!(second_logical[0].grapheme_start, 21);
        assert_eq!(second_logical[0].grapheme_end, 26);
        // And all ranges are contiguous (within each logical line).
        for window in first_logical.windows(2) {
            assert_eq!(window[0].grapheme_end, window[1].grapheme_start);
        }
    }

    #[rstest::rstest]
    fn unicode_text_wraps_correctly() {
        // Given text with emoji (each emoji is 1 grapheme, 2 display columns
        // but we count graphemes for wrapping).
        let text = "🎉🎊🎈🎁🎁🎊🎈🎉";

        // When wrapping at width 5 (graphemes, not columns).
        let lines = wrap_text(text, 5);

        // Then there are 2 lines (5 + 3 graphemes).
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].grapheme_end - lines[0].grapheme_start, 5);
    }

    #[rstest::rstest]
    fn width_zero_returns_single_line_per_logical() {
        // Given "hello\nworld" with width 0.
        // When wrapping.
        let lines = wrap_text("hello\nworld", 0);

        // Then there are 2 lines (one per logical line, no wrapping).
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].grapheme_start, 0);
        assert_eq!(lines[0].grapheme_end, 5);
        assert_eq!(lines[1].grapheme_start, 6);
        assert_eq!(lines[1].grapheme_end, 11);
    }

    #[rstest::rstest]
    fn trailing_newline_produces_empty_last_line() {
        // Given "hello\n" with wide width.
        // When wrapping.
        let lines = wrap_text("hello\n", 80);

        // Then there are 2 lines.
        assert_eq!(lines.len(), 2);
        // And the first covers "hello".
        assert_eq!(lines[0].grapheme_start, 0);
        assert_eq!(lines[0].grapheme_end, 5);
        // And the second is empty (from the trailing \n).
        assert_eq!(lines[1].grapheme_start, 6);
        assert_eq!(lines[1].grapheme_end, 6);
    }

    #[rstest::rstest]
    fn exact_width_no_wrap() {
        // Given text exactly matching the width.
        let text = "hello";

        // When wrapping at width 5.
        let lines = wrap_text(text, 5);

        // Then there is 1 line.
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].grapheme_start, 0);
        assert_eq!(lines[0].grapheme_end, 5);
    }

    #[rstest::rstest]
    fn double_newline_produces_empty_middle_line() {
        // Given "a\n\nb" with wide width.
        // When wrapping.
        let lines = wrap_text("a\n\nb", 80);

        // Then there are 3 logical lines.
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].grapheme_start, 0);
        assert_eq!(lines[0].grapheme_end, 1);
        assert_eq!(lines[0].logical_line_index, 0);
        assert_eq!(lines[1].grapheme_start, 2);
        assert_eq!(lines[1].grapheme_end, 2);
        assert_eq!(lines[1].logical_line_index, 1);
        assert_eq!(lines[2].grapheme_start, 3);
        assert_eq!(lines[2].grapheme_end, 4);
        assert_eq!(lines[2].logical_line_index, 2);
    }
}
