//! Session picker entry type and rendering.

use std::ops::Range;

use crate::feat::picker::style::{dim_style, selected_style};
use crate::protocol::SessionId;
use nullslop_selection_widget::PickerItem;
use nullslop_selection_widget::highlight_text;
use ratatui::text::{Line, Span};

/// A saved session entry ready for display in the picker.
#[derive(Debug, Clone)]
pub struct SessionEntry {
    /// The session's unique identifier.
    pub session_id: SessionId,
    /// Human-readable title (derived from first user message).
    pub title: String,
    /// When this session was last modified.
    pub updated_at: jiff::Timestamp,
    /// Byte offset in the JSONL file for direct seek.
    pub byte_offset: u64,
}

impl PickerItem for SessionEntry {
    fn display_label(&self) -> &str {
        &self.title
    }

    fn render_row(&self, is_selected: bool) -> Line<'static> {
        render_session_row(&self.title, &self.updated_at, is_selected, &[])
    }

    fn render_row_with_highlight(
        &self,
        is_selected: bool,
        match_indices: &[Range<usize>],
    ) -> Line<'static> {
        render_session_row(&self.title, &self.updated_at, is_selected, match_indices)
    }
}

/// Renders a session picker row, optionally highlighting matched characters.
///
/// Match indices are byte offsets into `title` (the `display_label`).
/// Format: `"{title}          {YYYY-MM-DD HH:MM}"` with right-aligned date.
fn render_session_row(
    title: &str,
    updated_at: &jiff::Timestamp,
    is_selected: bool,
    match_indices: &[Range<usize>],
) -> Line<'static> {
    let base_style = selected_style(is_selected);

    let date_style = dim_style(is_selected);

    let datetime = updated_at.to_zoned(jiff::tz::TimeZone::UTC).datetime();
    let date_str = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        datetime.year(),
        datetime.month() as u8,
        datetime.day(),
        datetime.hour(),
        datetime.minute(),
    );

    let title_spans = if match_indices.is_empty() {
        vec![Span::styled(title.to_owned(), base_style)]
    } else {
        highlight_text(title, base_style, match_indices)
    };

    let mut spans = title_spans;
    spans.push(Span::styled("          ".to_owned(), base_style));
    spans.push(Span::styled(date_str, date_style));
    Line::from(spans)
}
