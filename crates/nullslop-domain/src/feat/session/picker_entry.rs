//! Session picker entry type and rendering.

use std::ops::Range;

use crate::feat::picker::style::{dim_style, selected_style};
use crate::feat::session::chat_session::SessionState;
use crate::feat::theme::Theme;
use crate::protocol::SessionId;
use nullslop_selection_widget::PickerItem;
use nullslop_selection_widget::highlight_text_with_bg;
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
    /// Theme for rendering.
    pub theme: Theme,
    /// Whether this session is loaded in memory or archived.
    pub session_state: SessionState,
}

impl PickerItem for SessionEntry {
    fn display_label(&self) -> &str {
        &self.title
    }

    fn render_row(&self, is_selected: bool) -> Line<'static> {
        render_session_row(
            &self.title,
            &self.updated_at,
            is_selected,
            &[],
            &self.theme,
            self.session_state,
        )
    }

    fn render_row_with_highlight(
        &self,
        is_selected: bool,
        match_indices: &[Range<usize>],
    ) -> Line<'static> {
        render_session_row(
            &self.title,
            &self.updated_at,
            is_selected,
            match_indices,
            &self.theme,
            self.session_state,
        )
    }
}

/// Renders a session picker row, optionally highlighting matched characters.
///
/// Match indices are byte offsets into `title` (the `display_label`).
/// Format: `"{title}          {YYYY-MM-DD HH:MM}"` with right-aligned date.
/// Archived sessions use dimmed styling to visually distinguish them from loaded sessions.
fn render_session_row(
    title: &str,
    updated_at: &jiff::Timestamp,
    is_selected: bool,
    match_indices: &[Range<usize>],
    theme: &Theme,
    session_state: SessionState,
) -> Line<'static> {
    let base_style = if session_state == SessionState::Archived {
        dim_style(is_selected, theme)
    } else {
        selected_style(is_selected, theme)
    };

    let date_style = dim_style(is_selected, theme);

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
        highlight_text_with_bg(title, base_style, match_indices, theme.picker_highlight_bg)
    };

    let mut spans = title_spans;
    spans.push(Span::styled("          ".to_owned(), base_style));
    spans.push(Span::styled(date_str, date_style));
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::feat::theme::default_theme;

    #[rstest::rstest]
    fn archived_entry_renders_with_dimmed_foreground() {
        // Given an archived session entry.
        let entry = SessionEntry {
            session_id: SessionId::new(),
            title: "Archived Chat".to_owned(),
            updated_at: jiff::Timestamp::now(),
            theme: default_theme(),
            session_state: SessionState::Archived,
        };

        // When rendering (not selected).
        let row = entry.render_row(false);

        // Then the title span has muted_text foreground.
        let title_span = &row.spans[0];
        assert_eq!(title_span.style.fg, Some(default_theme().muted_text));
    }

    #[rstest::rstest]
    fn loaded_entry_renders_with_normal_foreground() {
        // Given a loaded session entry.
        let entry = SessionEntry {
            session_id: SessionId::new(),
            title: "Active Chat".to_owned(),
            updated_at: jiff::Timestamp::now(),
            theme: default_theme(),
            session_state: SessionState::Loaded,
        };

        // When rendering (not selected).
        let row = entry.render_row(false);

        // Then the title span has default foreground (no explicit fg).
        let title_span = &row.spans[0];
        assert_eq!(title_span.style.fg, None);
    }

    #[rstest::rstest]
    fn archived_selected_entry_has_contrast_foreground() {
        // Given an archived session entry.
        let theme = default_theme();
        let entry = SessionEntry {
            session_id: SessionId::new(),
            title: "Archived Chat".to_owned(),
            updated_at: jiff::Timestamp::now(),
            theme: theme.clone(),
            session_state: SessionState::Archived,
        };

        // When rendering (selected).
        let row = entry.render_row(true);

        // Then the title span has contrast-adjusted foreground on selected bg.
        let title_span = &row.spans[0];
        let expected_fg = crate::feat::theme::contrast::ensure_contrast(
            theme.muted_text,
            theme.picker_selected_bg,
        );
        assert_eq!(title_span.style.fg, Some(expected_fg));
        assert_eq!(title_span.style.bg, Some(theme.picker_selected_bg));
    }
}
