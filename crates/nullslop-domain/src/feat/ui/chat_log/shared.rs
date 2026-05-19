//! Shared rendering helpers for chat log entries.

use crate::feat::theme::Theme;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// Width of the left gutter column (2 cells for emoji support).
pub const GUTTER_WIDTH: u16 = 2;

/// Context passed to each entry module's `to_lines` function.
pub struct RenderContext {
    /// Available content width (area width minus gutter).
    pub content_width: u16,
    /// Whether this entry is currently selected by the cursor.
    pub _is_selected: bool,
    /// Whether this tool result entry is expanded (show all lines).
    pub is_expanded: bool,
    /// Maximum lines before truncating tool entries (tool calls and tool results).
    pub tool_entry_max_lines: u16,
    /// The current theme colors.
    pub theme: Theme,
}

/// Split text on `\n` and produce styled lines with the given prefix.
///
/// Continuation lines (after the first `\n`) have no prefix — the `indent`
/// parameter is accepted for API compatibility but currently unused.
pub fn multiline_styled<T, P, I>(text: T, prefix: P, indent: I, style: Style) -> Vec<Line<'static>>
where
    T: AsRef<str>,
    P: AsRef<str>,
    I: AsRef<str>,
{
    let text = text.as_ref();
    let text = text.trim_start_matches('\n');
    let text = text.trim_end_matches('\n');
    let prefix = prefix.as_ref();
    let _ = indent.as_ref();
    let segments = text.split('\n');
    let mut lines = Vec::new();
    for (i, segment) in segments.enumerate() {
        let content = if i == 0 {
            format!("{prefix}{segment}")
        } else {
            segment.to_owned()
        };
        lines.push(Line::from(Span::styled(content, style)));
    }
    lines
}

/// Pad a line to the given width by appending a trailing-space span with the
/// given background style. This ensures BLOCK-style entries fill the full row.
///
/// **Important:** Must be called *after* the line is fully constructed.
/// The padding span is appended to the line's spans.
pub fn pad_line_to_width(line: &mut Line<'static>, width: u16, bg_style: Style) {
    let current_width = line.width() as u16;
    let padding = width.saturating_sub(current_width);
    if padding > 0 {
        line.spans
            .push(Span::styled(" ".repeat(padding as usize), bg_style));
    }
}

/// Which sides of an entry to pad with a blank line.
///
/// Controls where visual spacing is added around a chat entry.
/// Most entries use [`Both`](Pad::Both), but entries like thinking
/// that always precede another padded entry use [`Top`](Pad::Top) only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pad {
    /// Add a blank line above and below the entry.
    Both,
    /// Add a blank line above the entry only.
    Top,
    /// Add a blank line below the entry only.
    Bottom,
}

/// Add blank padding lines around an entry for visual spacing.
///
/// Uses plain blank `Line::from("")` for the pad lines.
/// For styled padding (e.g. block backgrounds), use [`pad_entry_with`].
pub fn pad_entry(lines: &mut Vec<Line<'static>>, pad: Pad) {
    pad_entry_with(lines, pad, Line::from(""));
}

/// Add styled padding lines around an entry for visual spacing.
///
/// The `pad_line` is used for both top and bottom (cloned as needed).
pub fn pad_entry_with(lines: &mut Vec<Line<'static>>, pad: Pad, pad_line: Line<'static>) {
    if pad == Pad::Both || pad == Pad::Top {
        lines.insert(0, pad_line.clone());
    }
    if pad == Pad::Both || pad == Pad::Bottom {
        lines.push(pad_line);
    }
}

/// Replace literal `\n` (backslash + n) sequences with actual newline characters.
///
/// Tool call arguments and tool result content may contain JSON-encoded
/// newline escapes that arrive as two-character sequences in the Rust string.
/// This helper converts them so the renderer can split on real `\n`.
pub fn unescape_newlines(s: &str) -> String {
    s.replace("\\n", "\n")
}

/// Compute the display width of a string using Unicode grapheme clusters.
pub fn unicode_segementation_display_width(s: &str) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    s.graphemes(true)
        .map(|g| {
            // Emoji and wide characters take 2 columns; everything else takes 1.
            // This is a simplified heuristic — full-width detection would need
            // unicode-width, but for our use case (provider names, counts, status)
            // this is sufficient.
            if g.chars().any(|c| c as u32 > 0x2000) {
                2
            } else {
                1
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use ratatui::style::{Color, Style};
    use ratatui::text::{Line, Span};

    use super::{Pad, multiline_styled, pad_entry, pad_entry_with};

    #[rstest::rstest]
    fn pad_entry_both_adds_blank_line_above_and_below() {
        // Given a single content line.
        let mut lines = vec![Line::from("hello")];

        // When padding the entry on both sides.
        pad_entry(&mut lines, Pad::Both);

        // Then there are 3 lines total (pad + content + pad).
        assert_eq!(lines.len(), 3);
        // And the first line is blank.
        assert!(lines[0].spans.is_empty());
        // And the middle line has content.
        assert_eq!(lines[1].spans[0].content, "hello");
        // And the last line is blank.
        assert!(lines[2].spans.is_empty());
    }

    #[rstest::rstest]
    fn pad_entry_both_on_empty_produces_two_pad_lines() {
        // Given an empty lines vec.
        let mut lines: Vec<Line<'static>> = vec![];

        // When padding on both sides.
        pad_entry(&mut lines, Pad::Both);

        // Then there are 2 lines (top pad + bottom pad).
        assert_eq!(lines.len(), 2);
        assert!(lines[0].spans.is_empty());
        assert!(lines[1].spans.is_empty());
    }

    #[rstest::rstest]
    fn pad_entry_top_adds_blank_line_above_only() {
        // Given a single content line.
        let mut lines = vec![Line::from("hello")];

        // When padding top only.
        pad_entry(&mut lines, Pad::Top);

        // Then there are 2 lines (pad + content).
        assert_eq!(lines.len(), 2);
        assert!(lines[0].spans.is_empty());
        assert_eq!(lines[1].spans[0].content, "hello");
    }

    #[rstest::rstest]
    fn pad_entry_bottom_adds_blank_line_below_only() {
        // Given a single content line.
        let mut lines = vec![Line::from("hello")];

        // When padding bottom only.
        pad_entry(&mut lines, Pad::Bottom);

        // Then there are 2 lines (content + pad).
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content, "hello");
        assert!(lines[1].spans.is_empty());
    }

    #[rstest::rstest]
    fn pad_entry_with_styled_both_adds_styled_above_and_below() {
        // Given a single content line.
        let mut lines = vec![Line::from("hello")];

        // When padding with a styled line on both sides.
        let pad_line = Line::from(Span::styled(
            " ".repeat(80),
            ratatui::style::Style::default().bg(Color::DarkGray),
        ));
        pad_entry_with(&mut lines, Pad::Both, pad_line);

        // Then there are 3 lines total (pad + content + pad).
        assert_eq!(lines.len(), 3);
        // And the first line has the styled pad content.
        assert_eq!(lines[0].spans.len(), 1);
        assert_eq!(lines[0].spans[0].content, " ".repeat(80));
        // And the middle line has content.
        assert_eq!(lines[1].spans[0].content, "hello");
        // And the last line has the same styled pad content.
        assert_eq!(lines[2].spans.len(), 1);
        assert_eq!(lines[2].spans[0].content, " ".repeat(80));
    }

    #[rstest::rstest]
    fn multiline_styled_trims_leading_newline() {
        // Given text with leading newlines.
        // When converting to lines.
        let lines = multiline_styled("\n\nhello", "", "", Style::default());

        // Then there are no leading blank lines.
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "hello");
    }

    #[rstest::rstest]
    fn multiline_styled_trims_trailing_newline() {
        // Given text with trailing newlines.
        // When converting to lines.
        let lines = multiline_styled("hello\n\n", "", "", Style::default());

        // Then there are no trailing blank lines.
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "hello");
    }

    #[rstest::rstest]
    fn multiline_styled_trims_both_newlines() {
        // Given text with both leading and trailing newlines.
        // When converting to lines.
        let lines = multiline_styled("\n\nhello\n\n", "", "", Style::default());

        // Then there is exactly one content line.
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "hello");
    }
}
