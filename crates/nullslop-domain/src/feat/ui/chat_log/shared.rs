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
    /// Whether this entry is pinned.
    pub is_pinned: bool,
    /// Whether this tool result entry is expanded (show all lines).
    pub is_expanded: bool,
    /// Maximum lines before truncating tool result entries.
    pub tool_result_max_lines: u16,
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
