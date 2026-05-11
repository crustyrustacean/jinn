//! Session entries — loading and formatting.
//!
//! Contains loader functions for session picker entries.
//! The [`SessionEntry`] struct and [`PickerItem`] implementation live
//! in `nullslop-protocol`.

use crate::common::app_state::AppState;
use crate::common::services::Services;
use crate::protocol::SessionEntry;

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
pub fn load_session_picker_items(services: &Services, state: &mut AppState) {
    let entries = load_session_entries(services);
    state.frontend.session_picker.set_items(entries);
}

#[cfg(test)]
mod tests {
    use crate::protocol::SessionId;
    use nullslop_selection_widget::PickerItem;

    use super::*;

    #[rstest::rstest]
    fn session_entry_display_label_returns_title() {
        // Given a SessionEntry with a title.
        let entry = SessionEntry {
            session_id: SessionId::new(),
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
            session_id: SessionId::new(),
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
        let services = crate::common::services::Services::new();

        // When loading session entries.
        let entries = load_session_entries(&services);

        // Then an empty list is returned (fake store has no sessions).
        assert!(entries.is_empty());
    }
}
