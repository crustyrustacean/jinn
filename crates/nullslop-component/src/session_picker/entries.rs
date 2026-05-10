//! Session entries for the picker.
//!
//! Builds the list of saved sessions from the session store,
//! and implements [`PickerItem`] so [`SelectionState`] can fuzzy-filter
//! and render them.
//!
//! [`PickerItem`]: nullslop_selection_widget::PickerItem
//! [`SelectionState`]: nullslop_selection_widget::SelectionState

use std::ops::Range;

use nullslop_protocol::SessionId;
use nullslop_selection_widget::PickerItem;
use nullslop_selection_widget::highlight_text;
use nullslop_services::Services;
use ratatui::style::{Color, Style};
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
    let base_style = if is_selected {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    } else {
        Style::default()
    };

    let date_style = if is_selected {
        Style::default().fg(Color::DarkGray).bg(Color::DarkGray)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Format timestamp as YYYY-MM-DD HH:MM (absolute date/time).
    let datetime = updated_at.to_zoned(jiff::tz::TimeZone::UTC).datetime();
    let date_str = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        datetime.year(),
        datetime.month() as u8,
        datetime.day(),
        datetime.hour(),
        datetime.minute(),
    );

    // Highlight within the title portion only.
    let title_spans = if match_indices.is_empty() {
        vec![Span::styled(title.to_owned(), base_style)]
    } else {
        highlight_text(title, base_style, match_indices)
    };

    // Right-pad title with spaces, then append date.
    let mut spans = title_spans;
    spans.push(Span::styled("          ".to_owned(), base_style));
    spans.push(Span::styled(date_str, date_style));
    Line::from(spans)
}

/// Loads session entries from the session store, sorted by `updated_at` descending.
///
/// Reads summaries from the store, maps them to [`SessionEntry`], and sorts
/// so the most recently updated session appears first. Errors are logged and
/// result in an empty list.
pub fn load_session_entries(services: &Services) -> Vec<SessionEntry> {
    match services.session_store.load_summaries() {
        Ok(summaries) => {
            let mut entries: Vec<SessionEntry> = summaries
                .into_iter()
                .map(|(session_id, summary, byte_offset)| SessionEntry {
                    session_id,
                    title: summary.title,
                    updated_at: summary.updated_at,
                    byte_offset,
                })
                .collect();
            // Sort by updated_at descending (most recent first).
            entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            entries
        }
        Err(e) => {
            tracing::warn!(err = ?e, "failed to load session summaries");
            vec![]
        }
    }
}

/// Loads session entries into the picker state, ready for display.
///
/// Reads from the session store via services and stores the entries via
/// `SelectionState::set_items`.
///
/// [`SelectionState`]: nullslop_selection_widget::SelectionState
pub fn load_session_picker_items(services: &Services, state: &mut crate::AppState) {
    let entries = load_session_entries(services);
    state.frontend.session_picker.set_items(entries);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn session_entry_display_label_returns_title() {
        // Given a SessionEntry with a title.
        let entry = SessionEntry {
            session_id: nullslop_protocol::SessionId::new(),
            title: "My Chat".to_owned(),
            updated_at: jiff::Timestamp::now(),
            byte_offset: 0,
        };

        // When calling display_label.
        // Then it returns the title.
        assert_eq!(entry.display_label(), "My Chat");
    }

    #[rstest::rstest]
    fn render_row_contains_title() {
        // Given a session entry.
        let entry = SessionEntry {
            session_id: nullslop_protocol::SessionId::new(),
            title: "My Session".to_owned(),
            updated_at: jiff::Timestamp::now(),
            byte_offset: 0,
        };

        // When rendering.
        let row = entry.render_row(false);

        // Then the title appears in the rendered line.
        assert!(row.spans.iter().any(|s| s.content.contains("My Session")));
    }

    #[rstest::rstest]
    fn load_session_entries_returns_empty_on_error() {
        // Given a test Services (with fake session store that returns empty).
        let services = nullslop_services::Services::new();

        // When loading session entries.
        let entries = load_session_entries(&services);

        // Then an empty list is returned (fake store has no sessions).
        assert!(entries.is_empty());
    }
}
