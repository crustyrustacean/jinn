//! Shared highlight utility for picker entry rows.
//!
//! Provides [`highlight_text`], which splits a string into styled [`Span`]s
//! based on fuzzy match byte offsets. Used by the context strategy picker
//! and provider picker entry types.

use std::ops::Range;

use crate::PICKER_HIGHLIGHT_STYLE;
use ratatui::style::Style;
use ratatui::text::Span;

/// Splits `text` into spans, applying the highlight style to characters whose
/// byte offset falls within one of `match_indices`.
///
/// Matched characters get [`PICKER_HIGHLIGHT_STYLE`] patched onto the base style
/// (preserving the base foreground color).
///
/// # Panics
///
/// Does not panic; string slicing is safe because `byte_off` comes from
/// `char_indices()`, which always yields valid UTF-8 boundaries.
#[expect(
    clippy::string_slice,
    reason = "byte_off comes from char_indices(), always a valid UTF-8 boundary"
)]
pub fn highlight_text<'a>(
    text: &str,
    base_style: Style,
    match_indices: &[Range<usize>],
) -> Vec<Span<'a>> {
    if match_indices.is_empty() || text.is_empty() {
        return vec![Span::styled(text.to_owned(), base_style)];
    }

    let highlight_style = base_style.patch(PICKER_HIGHLIGHT_STYLE);

    let mut spans = Vec::new();
    let mut current_start = 0;
    let mut in_highlight = false;

    for (byte_off, _ch) in text.char_indices() {
        let is_matched = match_indices.iter().any(|r| r.contains(&byte_off));

        if is_matched != in_highlight {
            let segment = text[current_start..byte_off].to_owned();
            if !segment.is_empty() {
                spans.push(Span::styled(
                    segment,
                    if in_highlight {
                        highlight_style
                    } else {
                        base_style
                    },
                ));
            }
            current_start = byte_off;
            in_highlight = is_matched;
        }
    }

    if current_start < text.len() {
        let rest = text[current_start..].to_owned();
        spans.push(Span::styled(
            rest,
            if in_highlight {
                highlight_style
            } else {
                base_style
            },
        ));
    }

    if spans.is_empty() {
        spans.push(Span::styled(text.to_owned(), base_style));
    }

    spans
}
