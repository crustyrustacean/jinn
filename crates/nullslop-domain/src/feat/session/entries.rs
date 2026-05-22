//! Session entries — loading and formatting for the tree-structured picker.
//!
//! Contains loader functions for session picker entries.
//! The [`SessionTreeEntry`] struct and [`TreeItem`] implementation live
//! in `picker_entry.rs`.

use crate::common::app_state::AppState;
use crate::common::services::Services;
use crate::feat::session::picker_entry::SessionTreeEntry;
use crate::feat::theme::Theme;

use super::SessionStoreService;

/// Loads session tree entries from the session store, sorted by session state then `updated_at`.
///
/// Loaded sessions appear first (sorted by `updated_at` descending),
/// followed by archived sessions (sorted by `updated_at` descending).
/// Errors are logged and result in an empty list.
pub async fn load_session_entries(services: &Services, theme: &Theme) -> Vec<SessionTreeEntry> {
    match services.session_store.load_summaries().await {
        Ok(summaries) => {
            let mut entries: Vec<SessionTreeEntry> = summaries
                .into_iter()
                .map(|summary| {
                    SessionTreeEntry::new(
                        summary.session_id,
                        summary.title,
                        summary.updated_at,
                        theme.clone(),
                        summary.session_state,
                        summary.parent_session,
                    )
                })
                .collect();
            // Loaded first, then by updated_at descending within each group.
            entries.sort_by(|a, b| {
                b.session_state
                    .cmp(&a.session_state)
                    .then_with(|| b.updated_at.cmp(&a.updated_at))
            });
            entries
        }
        Err(e) => {
            tracing::warn!(err = ?e, "failed to load session summaries");
            vec![]
        }
    }
}

/// Loads session tree entries into the picker state, ready for display.
///
/// Reads from the session store via services and stores the entries via
/// `TreePickerState::set_items`.
pub async fn load_session_picker_items(services: &Services, state: &mut AppState) {
    let entries = load_session_entries(services, &state.frontend.theme).await;
    state.frontend.session_picker.set_items(entries);
}

/// Loads session tree entries from a session store service directly.
///
/// Same as [`load_session_entries`] but accepts the store service directly
/// instead of the full `Services` container.
pub async fn load_session_entries_from_store(
    store: &SessionStoreService,
    theme: &Theme,
) -> Vec<SessionTreeEntry> {
    match store.load_summaries().await {
        Ok(summaries) => {
            let mut entries: Vec<SessionTreeEntry> = summaries
                .into_iter()
                .map(|summary| {
                    SessionTreeEntry::new(
                        summary.session_id,
                        summary.title,
                        summary.updated_at,
                        theme.clone(),
                        summary.session_state,
                        summary.parent_session,
                    )
                })
                .collect();
            // Loaded first, then by updated_at descending within each group.
            entries.sort_by(|a, b| {
                b.session_state
                    .cmp(&a.session_state)
                    .then_with(|| b.updated_at.cmp(&a.updated_at))
            });
            entries
        }
        Err(e) => {
            tracing::warn!(err = ?e, "failed to load session summaries");
            vec![]
        }
    }
}

/// Loads session tree entries into the picker state from a session store service.
pub async fn load_session_picker_items_from_store(
    store: &SessionStoreService,
    state: &mut AppState,
) {
    let entries = load_session_entries_from_store(store, &state.frontend.theme).await;
    state.frontend.session_picker.set_items(entries);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use crate::feat::session::chat_session::SessionState;
    use crate::feat::session::picker_entry::SessionTreeEntry;
    use crate::feat::theme::default_theme;
    use crate::protocol::SessionId;
    use nullslop_selection_widget::TreeItem;

    use super::*;

    #[rstest::rstest]
    fn session_entry_display_label_returns_title() {
        // Given a SessionTreeEntry with a title.
        let entry = SessionTreeEntry::new(
            SessionId::new(),
            "My Chat".to_owned(),
            jiff::Timestamp::now(),
            default_theme(),
            SessionState::Loaded,
            None,
        );

        // When calling display_label.
        // Then it returns the title.
        assert_eq!(entry.display_label(), "My Chat");
    }

    #[rstest::rstest]
    fn render_row_contains_title() {
        // Given a session tree entry.
        let entry = SessionTreeEntry::new(
            SessionId::new(),
            "My Session".to_owned(),
            jiff::Timestamp::now(),
            default_theme(),
            SessionState::Loaded,
            None,
        );

        // When rendering.
        let row = entry.render_row(false);

        // Then the title appears in the rendered line.
        assert!(row.spans.iter().any(|s| s.content.contains("My Session")));
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn load_session_entries_returns_empty_on_error() {
        // Given a test Services (with fake session store that returns empty).
        let services = crate::common::services::Services::new();

        // When loading session entries.
        let entries = load_session_entries(&services, &default_theme()).await;

        // Then an empty list is returned (fake store has no sessions).
        assert!(entries.is_empty());
    }
}
