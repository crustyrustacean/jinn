//! Shared rendering helpers for chat log entries.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// Split text on `\n` and produce styled lines with the given prefix/indent.
///
/// When `is_selected` is true, the first line gets a `▶ ` prefix and
/// `Modifier::REVERSED` added to its style.
pub fn multiline_styled<T, P, I>(
    text: T,
    prefix: P,
    indent: I,
    style: Style,
    is_selected: bool,
) -> Vec<Line<'static>>
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
        let (content, line_style) = if i == 0 && is_selected {
            (
                format!("▶ {prefix}{segment}"),
                style.add_modifier(Modifier::REVERSED),
            )
        } else if i == 0 {
            (format!("{prefix}{segment}"), style)
        } else {
            (segment.to_owned(), style)
        };
        lines.push(Line::from(Span::styled(content, line_style)));
    }
    lines
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
