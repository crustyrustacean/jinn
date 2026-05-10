//! Keymap entry types for the keymap search picker.
//!
//! [`KeymapEntry`] represents a single fully-resolved leaf binding from the
//! keymap — a key sequence, its description, scope, category, and the command
//! it triggers. It implements [`PickerItem`] so `SelectionState` can
//! fuzzy-filter and render keymap entries in the picker overlay.
//!
//! The tree-walking collection functions that build `KeymapEntry` lists live
//! in `nullslop-tui` (they need the concrete `KeyEvent`/`Scope`/`KeyCategory` types).

use std::ops::Range;

use crate::PICKER_HIGHLIGHT_STYLE;
use nullslop_selection_widget::PickerItem;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// A single fully-resolved keymap binding, ready for display in the picker.
#[derive(Debug, Clone)]
pub struct KeymapEntry {
    /// Full key sequence string (e.g., `"gg"`, `"gmp"`, `"<c-p>"`).
    pub key_sequence: String,
    /// Command description from the keymap (e.g., `"scroll to top"`).
    pub description: String,
    /// Scope name (e.g., `"Normal"`, `"Input"`, `"Dashboard"`, `"Picker"`).
    pub scope: String,
    /// Category name (e.g., `"General"`, `"Navigation"`, `"Input"`).
    pub category: String,
    /// The action to execute when this entry is confirmed.
    pub command: nullslop_protocol::Intent,
    /// Pre-computed searchable text combining key sequence and description.
    /// Used by fuzzy matching so users can search by either keys or description.
    pub search_text: String,
}

impl PickerItem for KeymapEntry {
    fn display_label(&self) -> &str {
        // Returns pre-computed search text for fuzzy matching.
        // Combines key_sequence + description so users can search by either.
        &self.search_text
    }

    fn render_row(&self, is_selected: bool) -> Line<'static> {
        render_keymap_row(
            &self.key_sequence,
            &self.description,
            &self.scope,
            &self.category,
            is_selected,
            &[],
        )
    }

    fn render_row_with_highlight(
        &self,
        is_selected: bool,
        match_indices: &[Range<usize>],
    ) -> Line<'static> {
        render_keymap_row(
            &self.key_sequence,
            &self.description,
            &self.scope,
            &self.category,
            is_selected,
            match_indices,
        )
    }
}

/// Renders a keymap picker row, optionally highlighting matched characters.
///
/// When `match_indices` is non-empty, the byte offsets (which index into
/// `search_text = "{key_sequence} {description}"`) are mapped onto the rendered
/// text. Characters whose source byte offset falls within a match range get the
/// highlight style; all other characters keep their base style.
fn render_keymap_row(
    key_sequence: &str,
    description: &str,
    scope: &str,
    category: &str,
    is_selected: bool,
    match_indices: &[Range<usize>],
) -> Line<'static> {
    let sel_bg = if is_selected {
        Color::DarkGray
    } else {
        Color::Reset
    };

    let key_style = Style::default()
        .fg(Color::Yellow)
        .bg(sel_bg)
        .add_modifier(Modifier::BOLD);

    let desc_style = Style::default().fg(Color::White).bg(sel_bg);

    let meta_style = Style::default().fg(Color::DarkGray).bg(sel_bg);

    // Pad key_sequence to a minimum width for alignment.
    let key_display = if key_sequence.len() < 8 {
        format!("{key_sequence:<8}")
    } else {
        key_sequence.to_owned()
    };

    // Build the rendered text segments with their base styles and the
    // corresponding byte ranges in search_text that each character maps to.
    //
    // search_text = "{key_sequence} {description}"
    // key_len = byte length of key_sequence
    // The separator space is at byte offset key_len.
    // description starts at byte offset key_len + 1.
    // Padding spaces and separator/trailing spaces in rendered text don't
    // correspond to any search_text byte, so they map to None.
    let key_len = key_sequence.len();

    // Rendered key segment: key_display + "  "
    // Only the first key_sequence.len() chars come from search_text[0..key_len].
    // Padding and trailing spaces don't map to search_text.
    let key_spans = highlight_text_segment(
        &key_display,
        key_style,
        match_indices,
        0,                  // rendered offset where search_text mapping begins
        0..key_len,         // search_text range for the key part
        key_sequence.len(), // only this many chars are from search_text
    );
    // Trailing spaces after key (not searchable).
    let key_trailing = Span::styled("  ", key_style);

    // Rendered description segment: description + "  "
    let desc_offset = key_len + 1; // search_text byte offset where description starts
    let desc_len = description.len();
    let desc_spans = highlight_text_segment(
        description,
        desc_style,
        match_indices,
        0,
        desc_offset..desc_offset + desc_len,
        description.len(),
    );
    let desc_trailing = Span::styled("  ", desc_style);

    // Meta segments: scope and category are not part of search_text.
    let scope_span = Span::styled(format!("[{scope}] "), meta_style);
    let category_span = Span::styled(category.to_owned(), meta_style);

    let mut spans = Vec::new();
    spans.extend(key_spans);
    spans.push(key_trailing);
    spans.extend(desc_spans);
    spans.push(desc_trailing);
    spans.push(scope_span);
    spans.push(category_span);

    Line::from(spans)
}

/// Splits a text segment into spans, applying the highlight style to characters
/// whose `search_text` byte offset falls within `match_indices`.
///
/// Matched characters get [`PICKER_HIGHLIGHT_STYLE`] patched onto the base style
/// (preserving the base foreground color).
///
/// `text` is the rendered string. `base_style` is applied to non-highlighted chars.
/// `match_indices` are the fuzzy match ranges (byte offsets into `display_label`).
/// `rendered_offset` is where we start consuming characters from `text` (usually 0).
/// `search_range` is the byte range in `search_text` that this `text` maps to.
/// `searchable_len` is how many chars at the start of `text` correspond to searchable text.
#[expect(
    clippy::string_slice,
    reason = "byte_off comes from char_indices(), always a valid UTF-8 boundary"
)]
fn highlight_text_segment(
    text: &str,
    base_style: Style,
    match_indices: &[Range<usize>],
    _rendered_offset: usize,
    search_range: Range<usize>,
    searchable_len: usize,
) -> Vec<Span<'static>> {
    if match_indices.is_empty() || searchable_len == 0 {
        return vec![Span::styled(text.to_owned(), base_style)];
    }

    let highlight_style = base_style.patch(PICKER_HIGHLIGHT_STYLE);

    let mut spans = Vec::new();
    let mut current_start = 0;
    let mut in_highlight = false;

    for (char_idx, (byte_off, _ch)) in text.char_indices().enumerate() {
        if char_idx >= searchable_len {
            // Past the searchable portion — flush remaining and break.
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
            return spans;
        }

        // Map this character's byte position within text to its search_text byte offset.
        let search_byte = search_range.start + byte_off;
        let is_matched = match_indices.iter().any(|r| r.contains(&search_byte));

        if is_matched != in_highlight {
            // Transition — emit accumulated text.
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

    // Flush remaining.
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

    // If no spans were produced (empty text), return the full text with base style.
    if spans.is_empty() {
        spans.push(Span::styled(text.to_owned(), base_style));
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn make_entry(
        key_sequence: &str,
        description: &str,
        scope: &str,
        category: &str,
    ) -> KeymapEntry {
        let search_text = format!("{key_sequence} {description}");
        KeymapEntry {
            key_sequence: key_sequence.to_owned(),
            description: description.to_owned(),
            scope: scope.to_owned(),
            category: category.to_owned(),
            command: nullslop_protocol::Intent::Quit,
            search_text,
        }
    }

    #[rstest::rstest]
    fn display_label_returns_search_text() {
        // Given a keymap entry with key_sequence "gg".
        let entry = make_entry("gg", "scroll to top", "Normal", "Navigation");

        // When getting the display label.
        let label = entry.display_label();

        // Then it returns the combined search text containing both key and description.
        assert_eq!(label, "gg scroll to top");
    }

    #[rstest::rstest]
    fn render_row_contains_key() {
        // Given a keymap entry.
        let entry = make_entry("gg", "scroll to top", "Normal", "Navigation");

        // When rendering the row.
        let line = entry.render_row(false);

        // Then the rendered text contains the key sequence.
        let text: String = line.spans.iter().map(|s| &*s.content).collect();
        assert!(text.contains("gg"), "should contain key sequence");
    }

    #[rstest::rstest]
    fn render_row_contains_description() {
        // Given a keymap entry.
        let entry = make_entry("gg", "scroll to top", "Normal", "Navigation");

        // When rendering the row.
        let line = entry.render_row(false);

        // Then the rendered text contains the description.
        let text: String = line.spans.iter().map(|s| &*s.content).collect();
        assert!(text.contains("scroll to top"), "should contain description");
    }

    #[rstest::rstest]
    fn render_row_contains_scope() {
        // Given a keymap entry.
        let entry = make_entry("gg", "scroll to top", "Normal", "Navigation");

        // When rendering the row.
        let line = entry.render_row(false);

        // Then the rendered text contains the scope.
        let text: String = line.spans.iter().map(|s| &*s.content).collect();
        assert!(text.contains("[Normal]"), "should contain scope");
    }

    #[rstest::rstest]
    fn render_row_contains_category() {
        // Given a keymap entry.
        let entry = make_entry("gg", "scroll to top", "Normal", "Navigation");

        // When rendering the row.
        let line = entry.render_row(false);

        // Then the rendered text contains the category.
        let text: String = line.spans.iter().map(|s| &*s.content).collect();
        assert!(text.contains("Navigation"), "should contain category");
    }

    #[rstest::rstest]
    fn render_row_selected_has_dark_gray_background() {
        // Given a keymap entry.
        let entry = make_entry("q", "quit", "Normal", "General");

        // When rendering the row selected.
        let line = entry.render_row(true);

        // Then the key span has DarkGray background.
        let key_span = &line.spans[0];
        assert_eq!(key_span.style.bg, Some(Color::DarkGray));
    }

    #[rstest::rstest]
    fn render_row_unselected_has_reset_background() {
        // Given a keymap entry.
        let entry = make_entry("q", "quit", "Normal", "General");

        // When rendering the row not selected.
        let line = entry.render_row(false);

        // Then the key span has Reset background.
        let key_span = &line.spans[0];
        assert_eq!(key_span.style.bg, Some(Color::Reset));
    }

    #[rstest::rstest]
    fn render_row_key_is_yellow_bold() {
        // Given a keymap entry.
        let entry = make_entry("q", "quit", "Normal", "General");

        // When rendering the row.
        let line = entry.render_row(false);

        // Then the key span is yellow and bold.
        let key_span = &line.spans[0];
        assert_eq!(key_span.style.fg, Some(Color::Yellow));
        assert!(key_span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[rstest::rstest]
    fn render_row_pads_short_key_sequences() {
        // Given a keymap entry with a single-char key "q".
        let entry = make_entry("q", "quit", "Normal", "General");

        // When rendering the row.
        let line = entry.render_row(false);

        // Then the first span is the padded key (8 chars) and the second span is the trailing spaces.
        let key_span = &line.spans[0];
        assert!(
            key_span.content.len() >= 8,
            "key span should be padded to at least 8 chars"
        );
    }

    // --- Fuzzy matching via search_text tests ---

    #[rstest::rstest]
    fn search_text_combines_key_sequence_and_description() {
        // Given a keymap entry.
        let entry = make_entry("<c-p>", "open picker keymap", "Normal", "General");

        // Then search_text contains both the key sequence and description.
        assert!(entry.search_text.contains("<c-p>"));
        assert!(entry.search_text.contains("open picker keymap"));
    }

    // --- Highlight tests ---

    #[rstest::rstest]
    fn render_row_with_empty_match_indices_same_as_render_row() {
        // Given a keymap entry.
        let entry = make_entry("gg", "scroll to top", "Normal", "Navigation");

        // When rendering with and without match indices.
        let normal = entry.render_row(false);
        let highlighted = entry.render_row_with_highlight(false, &[]);

        // Then the output is identical.
        assert_eq!(normal.spans.len(), highlighted.spans.len());
        for (n, h) in normal.spans.iter().zip(highlighted.spans.iter()) {
            assert_eq!(n.content, h.content);
            assert_eq!(n.style, h.style);
        }
    }

    #[rstest::rstest]
    fn render_row_with_highlight_applies_gray_bg_to_matched_chars() {
        // Given a keymap entry with search_text "q quit".
        let entry = make_entry("q", "quit", "Normal", "General");

        // When highlighting with match at byte 0 (the "q" in key_sequence).
        #[expect(
            clippy::single_range_in_vec_init,
            reason = "genuinely want a slice containing one Range<usize>"
        )]
        let highlights: &[Range<usize>] = &[0..1];
        let line = entry.render_row_with_highlight(false, highlights);

        // Then at least one span has gray background (the matched "q").
        let has_highlight = line
            .spans
            .iter()
            .any(|s| s.style.bg == Some(Color::DarkGray));
        assert!(
            has_highlight,
            "expected at least one span with gray background"
        );
    }

    #[rstest::rstest]
    fn render_row_with_highlight_preserves_unmatched_chars() {
        // Given a keymap entry with search_text "gg scroll to top".
        let entry = make_entry("gg", "scroll to top", "Normal", "Navigation");

        // When highlighting with match at bytes 0..1 (the first "g").
        #[expect(
            clippy::single_range_in_vec_init,
            reason = "genuinely want a slice containing one Range<usize>"
        )]
        let highlights: &[Range<usize>] = &[0..1];
        let line = entry.render_row_with_highlight(false, highlights);

        // Then the full text is preserved.
        let text: String = line.spans.iter().map(|s| &*s.content).collect();
        assert!(text.contains("gg"), "should still contain 'gg'");
        assert!(
            text.contains("scroll to top"),
            "should still contain description"
        );
    }

}
