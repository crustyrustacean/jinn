//! Session tree entry type and rendering for the tree-structured session picker.

use std::ops::Range;

use crate::feat::picker::style::{dim_style, selected_style};
use crate::feat::session::chat_session::SessionState;
use crate::feat::theme::Theme;
use crate::protocol::SessionId;
use jinn_selection_widget::TreeItem;
use jinn_selection_widget::highlight_text_with_bg;
use ratatui::text::{Line, Span};

/// A saved session entry ready for display in the tree-structured picker.
///
/// Implements [`TreeItem`] for tree-aware fuzzy filtering and rendering.
/// ID fields are pre-computed as strings to satisfy the `&str` return types
/// on [`TreeItem::id`] and [`TreeItem::parent_id`].
#[derive(Debug, Clone)]
pub struct SessionTreeEntry {
    /// The session's unique identifier.
    pub session_id: SessionId,
    /// Pre-computed string representation of `session_id` for `TreeItem::id`.
    id_str: String,
    /// Human-readable title (derived from first user message).
    pub title: String,
    /// When this session was last modified.
    pub updated_at: jiff::Timestamp,
    /// Theme for rendering.
    pub theme: Theme,
    /// Whether this session is loaded in memory or archived.
    pub session_state: SessionState,
    /// Parent session ID — `None` for root sessions.
    pub parent_id: Option<SessionId>,
    /// Pre-computed string representation of `parent_id` for `TreeItem::parent_id`.
    parent_id_str: Option<String>,
}

impl SessionTreeEntry {
    /// Creates a new session tree entry.
    pub fn new(
        session_id: SessionId,
        title: String,
        updated_at: jiff::Timestamp,
        theme: Theme,
        session_state: SessionState,
        parent_id: Option<SessionId>,
    ) -> Self {
        let id_str = session_id.to_string();
        let parent_id_str = parent_id.as_ref().map(std::string::ToString::to_string);
        Self {
            session_id,
            id_str,
            title,
            updated_at,
            theme,
            session_state,
            parent_id,
            parent_id_str,
        }
    }
}

impl TreeItem for SessionTreeEntry {
    fn id(&self) -> &str {
        &self.id_str
    }

    fn parent_id(&self) -> Option<&str> {
        self.parent_id_str.as_deref()
    }

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
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use crate::feat::theme::default_theme;

    #[rstest::rstest]
    fn archived_entry_renders_with_dimmed_foreground() {
        // Given an archived session tree entry.
        let entry = SessionTreeEntry::new(
            SessionId::new(),
            "Archived Chat".to_owned(),
            jiff::Timestamp::now(),
            default_theme(),
            SessionState::Archived,
            None,
        );

        // When rendering (not selected).
        let row = entry.render_row(false);

        // Then the title span has muted_text foreground.
        let title_span = &row.spans[0];
        assert_eq!(title_span.style.fg, Some(default_theme().muted_text));
    }

    #[rstest::rstest]
    fn loaded_entry_renders_with_normal_foreground() {
        // Given a loaded session tree entry.
        let entry = SessionTreeEntry::new(
            SessionId::new(),
            "Active Chat".to_owned(),
            jiff::Timestamp::now(),
            default_theme(),
            SessionState::Loaded,
            None,
        );

        // When rendering (not selected).
        let row = entry.render_row(false);

        // Then the title span has default foreground (no explicit fg).
        let title_span = &row.spans[0];
        assert_eq!(title_span.style.fg, None);
    }

    #[rstest::rstest]
    fn archived_selected_entry_has_contrast_foreground() {
        // Given an archived session tree entry.
        let theme = default_theme();
        let entry = SessionTreeEntry::new(
            SessionId::new(),
            "Archived Chat".to_owned(),
            jiff::Timestamp::now(),
            theme.clone(),
            SessionState::Archived,
            None,
        );

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

    #[rstest::rstest]
    fn tree_item_id_returns_session_id_string() {
        // Given a session tree entry.
        let id = SessionId::new();
        let entry = SessionTreeEntry::new(
            id.clone(),
            "Test".to_owned(),
            jiff::Timestamp::now(),
            default_theme(),
            SessionState::Loaded,
            None,
        );

        // When calling id().
        // Then it returns the string representation of the session ID.
        assert_eq!(entry.id(), id.to_string());
        assert!(entry.parent_id().is_none());
    }

    #[rstest::rstest]
    fn tree_item_parent_id_returns_string() {
        // Given a session tree entry with a parent.
        let parent_id = SessionId::new();
        let entry = SessionTreeEntry::new(
            SessionId::new(),
            "Child".to_owned(),
            jiff::Timestamp::now(),
            default_theme(),
            SessionState::Loaded,
            Some(parent_id.clone()),
        );

        // When calling parent_id().
        // Then it returns the string representation of the parent ID.
        assert_eq!(entry.parent_id(), Some(parent_id.to_string().as_str()));
    }
}
