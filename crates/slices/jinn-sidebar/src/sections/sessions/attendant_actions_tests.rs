//! Tests for the attendant key actions — `N` (new attendant) and `R` (re-run).
//!
//! `handle_new_attendant` is the only production caller of
//! `ChatSessionState::new_attendant`, so the creation path's persistability
//! is proven here against the way production actually builds the session.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code"
)]

use std::sync::{Arc, Mutex};

use error_stack::Report;
use jinn_kernel::common::app_state::AppState;
use jinn_session_state::{SessionSnapshot, SessionStore, SessionStoreError};
use jinn_slices::ConfigLayer;

use super::attendant_actions::handle_new_attendant;

/// A recording store that honors the same `persist` gate the real SQLite
/// store applies — a non-persistable snapshot is dropped without a write.
///
/// Mirroring the gate is the whole point: a stub that recorded every snapshot
/// would report success with the production bug fully intact.
#[derive(Debug, Default)]
struct RecordingStore {
    saved: Mutex<Vec<SessionSnapshot>>,
}

impl RecordingStore {
    fn saved_for(&self, session_id: &jinn_core_types::SessionId) -> Option<SessionSnapshot> {
        self.saved
            .lock()
            .unwrap()
            .iter()
            .find(|snapshot| &snapshot.metadata.session_id == session_id)
            .cloned()
    }
}

#[async_trait::async_trait]
impl SessionStore for RecordingStore {
    fn name(&self) -> &'static str {
        "recording"
    }

    async fn save(&self, snapshot: &SessionSnapshot) -> Result<(), Report<SessionStoreError>> {
        if !snapshot.metadata.persist {
            return Ok(());
        }
        self.saved.lock().unwrap().push(snapshot.clone());
        Ok(())
    }

    async fn load_summaries(
        &self,
    ) -> Result<Vec<jinn_session_store_msg::SessionSummary>, Report<SessionStoreError>> {
        Ok(Vec::new())
    }

    async fn load_session(
        &self,
        _session_id: &jinn_core_types::SessionId,
    ) -> Result<Option<SessionSnapshot>, Report<SessionStoreError>> {
        Ok(None)
    }

    async fn delete(
        &self,
        _session_id: &jinn_core_types::SessionId,
    ) -> Result<(), Report<SessionStoreError>> {
        Ok(())
    }

    async fn fork(
        &self,
        _source_session_id: &jinn_core_types::SessionId,
        _at_ordinal: usize,
    ) -> Result<jinn_core_types::SessionId, Report<SessionStoreError>> {
        Ok(jinn_core_types::SessionId::new())
    }

    async fn set_archived(
        &self,
        _session_id: &jinn_core_types::SessionId,
        _archived: bool,
    ) -> Result<(), Report<SessionStoreError>> {
        Ok(())
    }

    async fn set_archived_many(
        &self,
        _session_ids: &[jinn_core_types::SessionId],
        _archived: bool,
    ) -> Result<(), Report<SessionStoreError>> {
        Ok(())
    }

    async fn load_unarchived_summaries(
        &self,
    ) -> Result<Vec<jinn_session_store_msg::SessionSummary>, Report<SessionStoreError>> {
        Ok(Vec::new())
    }

    async fn dirty_session_ids(
        &self,
    ) -> Result<Vec<jinn_core_types::SessionId>, Report<SessionStoreError>> {
        Ok(Vec::new())
    }

    async fn reindex_session_chunk(
        &self,
        _session_id: &jinn_core_types::SessionId,
        _max_entries: usize,
    ) -> Result<bool, Report<SessionStoreError>> {
        Ok(true)
    }

    async fn pending_dirty_count(&self) -> Result<usize, Report<SessionStoreError>> {
        Ok(0)
    }

    async fn search(
        &self,
        _params: jinn_session_store_msg::SearchParams,
    ) -> Result<jinn_session_store_msg::SearchOutcome, Report<SessionStoreError>> {
        Ok(jinn_session_store_msg::SearchOutcome {
            total_matches: 0,
            per_session: Vec::new(),
            hits: Vec::new(),
        })
    }

    async fn fetch_window(
        &self,
        _session_id: &jinn_core_types::SessionId,
        _anchor: &jinn_core_types::ChatEntryId,
        _context: usize,
    ) -> Result<Option<jinn_session_store_msg::TranscriptWindow>, Report<SessionStoreError>> {
        Ok(None)
    }

    async fn fetch_tail(
        &self,
        _session_id: &jinn_core_types::SessionId,
        _limit: usize,
    ) -> Result<Option<jinn_session_store_msg::TranscriptWindow>, Report<SessionStoreError>> {
        Ok(None)
    }
}

/// An empty config layer — the creation path reads per-session defaults
/// from it, and none are needed to exercise persistability.
fn empty_config() -> ConfigLayer {
    jinn_config::testutil::config_layer("")
}

/// State with the sessions section focused and its first row highlighted,
/// which is what both `N` and `R` require of the cursor.
fn state_with_selected_session() -> AppState {
    let state = AppState::default_with_scope_focus();
    state
        .frontend
        .scope_push(jinn_sidebar_msg::SidebarSectionId::Sessions.focus_scope());
    state
        .frontend
        .update_sections(|s| s.sessions.selected_index = Some(0));
    state
}

#[rstest::rstest]
#[tokio::test]
async fn created_attendant_reaches_the_store() {
    // Given a store that drops any snapshot the production store would also drop.
    let store = Arc::new(RecordingStore::default());
    let service = jinn_session_state::SessionStoreService::new(store.clone());

    // Given the sessions section focused with a session highlighted.
    let mut state = state_with_selected_session();
    let parent_id = state.session.active_session_id().clone();

    // When creating an attendant with `N` and saving it as the handler asks.
    let result = handle_new_attendant(&mut state, &empty_config());
    let attendant_id = state.session.active_session_id().clone();
    let snapshot = state
        .session
        .get(&attendant_id)
        .expect("attendant exists")
        .capture_snapshot();
    service.save(&snapshot).await.expect("snapshot saves");

    // Then the store holds the attendant, parented to the session it came from.
    let saved = store
        .saved_for(&attendant_id)
        .expect("the attendant was written to the store");
    assert_eq!(saved.metadata.parent_session, Some(parent_id));
    // And the handler still asks for the save.
    assert!(
        result.message_names.contains(&"PersistSession"),
        "the creation path must still request a save, published: {:?}",
        result.message_names
    );
}

#[rstest::rstest]
fn created_attendant_is_persistable() {
    // Given the sessions section focused with a session highlighted.
    let mut state = state_with_selected_session();

    // When creating an attendant with `N`.
    let _result = handle_new_attendant(&mut state, &empty_config());

    // Then the new attendant is persistable.
    let attendant_id = state.session.active_session_id().clone();
    assert!(
        state
            .session
            .get(&attendant_id)
            .expect("attendant exists")
            .is_persistable(),
        "an attendant created by `N` must be persistable"
    );
}
