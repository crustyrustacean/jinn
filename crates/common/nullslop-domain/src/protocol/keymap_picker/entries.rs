//! Keymap picker entry type and rendering.

use std::ops::Range;

use crate::protocol::Intent;
use nullslop_selection_widget::PICKER_HIGHLIGHT_STYLE;
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
    pub command: Intent,
    /// Pre-computed searchable text combining key sequence and description.
    /// Used by fuzzy matching so users can search by either keys or description.
    pub search_text: String,
}

impl PickerItem for KeymapEntry {
    fn display_label(&self) -> &str {
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

    let key_display = if key_sequence.len() < 8 {
        format!("{key_sequence:<8}")
    } else {
        key_sequence.to_owned()
    };

    let key_len = key_sequence.len();

    let key_spans = highlight_text_segment(
        &key_display,
        key_style,
        match_indices,
        0,
        0..key_len,
        key_sequence.len(),
    );
    let key_trailing = Span::styled("  ", key_style);

    let desc_offset = key_len + 1;
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

        let search_byte = search_range.start + byte_off;
        let is_matched = match_indices.iter().any(|r| r.contains(&search_byte));

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
