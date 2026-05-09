//! Shared application state.
//!
//! [`AppState`] is the single source of truth for what the user sees and how the
//! application is currently behaving. Every component reads from and writes to this
//! shared state.

use std::collections::HashMap;

use nullslop_protocol::{
    ActiveTab, ChatEntryId, Mode, PickerKind, PinPosition, PromptStrategyId, SessionId,
};
use nullslop_providers::NO_PROVIDER_ID;
use serde_json::Value as JsonValue;

use crate::chat_input_box::ChatInputBoxState;
use crate::chat_session::ChatSessionState;
use crate::context_strategy_picker::entries::StrategyEntry;
use crate::dashboard::DashboardState;
use crate::keymap_picker::entries::KeymapEntry;
use crate::pinned_panel::PinnedPanelState;
use crate::prompt_template::PromptTemplateStore;
use crate::provider_picker::entries::PickerEntry;
use crate::session_picker::entries::SessionEntry;
use crate::shutdown_tracker::ShutdownTrackerState;
use crate::tui_signals::TuiSignals;
/// A snapshot of everything the application is doing right now.
#[derive(Debug)]
pub struct AppState {
    /// All chat sessions, keyed by session ID.
    pub sessions: HashMap<SessionId, ChatSessionState>,

    /// The currently active session ID.
    pub active_session: SessionId,

    /// Whether the user is browsing or actively typing.
    pub mode: Mode,

    /// Bookkeeping for which actors are still running during shutdown.
    pub shutdown_tracker: ShutdownTrackerState,

    /// Actor dashboard — tracks registered actors and their status.
    pub dashboard: DashboardState,

    /// The currently active tab.
    pub active_tab: ActiveTab,

    /// Set to `true` when the user has requested to quit.
    pub should_quit: bool,

    /// The currently active provider. Always set — starts as [`NO_PROVIDER_ID`].
    pub active_provider: String,

    /// Which picker is currently active. `None` when not in picker mode.
    pub active_picker_kind: Option<PickerKind>,

    /// Provider picker state (items, filter text, selection index).
    pub provider_picker: nullslop_selection_widget::SelectionState<PickerEntry>,

    /// Last known model cache from discovery.
    pub model_cache: Option<nullslop_providers::ModelCache>,

    /// When the model list was last refreshed (UTC).
    /// `None` if the model list has never been refreshed.
    pub last_refreshed_at: Option<jiff::Timestamp>,

    /// Pinned panel state — selection index within the pinned entries list.
    pub pinned_panel: PinnedPanelState,

    /// Persisted strategy state blobs, keyed by (`session_id`, `strategy_id`).
    /// Stored as `serde_json::Value` — the host doesn't interpret the blobs.
    /// In-memory only; actual disk/DB persistence is a follow-up.
    pub strategy_state: HashMap<(SessionId, PromptStrategyId), JsonValue>,

    /// Context strategy picker state (items, filter text, selection index).
    pub context_strategy_picker: nullslop_selection_widget::SelectionState<StrategyEntry>,

    /// The default strategy for new sessions. Updated when the user confirms
    /// a strategy selection ("sticky" default).
    pub default_strategy: PromptStrategyId,

    /// Loaded prompt templates from `~/.config/nullslop/prompts/`.
    pub prompt_templates: PromptTemplateStore,

    /// Keymap picker state (items, filter text, selection index).
    pub keymap_picker: nullslop_selection_widget::SelectionState<KeymapEntry>,

    /// Whether the keymap picker shows all scopes (`true`) or current scope only (`false`).
    pub keymap_picker_show_all: bool,

    /// The scope the user was in when they opened the keymap picker.
    /// Used to filter back to the originating scope when toggling the filter.
    /// `None` when the keymap picker is not open.
    pub keymap_picker_origin_scope: Option<String>,

    /// Session picker state (items, filter text, selection index).
    pub session_picker: nullslop_selection_widget::SelectionState<SessionEntry>,

    /// Whether a session is currently being loaded from disk.
    /// When `true`, the chat log shows a centered "Loading session..." indicator
    /// instead of the conversation history.
    pub session_loading: bool,

    /// Signals from the [`IntentHandler`] for the outer platform layer.
    /// Cleared at the start of each `handle()` call.
    pub tui_signals: TuiSignals,

    /// All keymap entries, populated once at startup. Used by [`ToggleKeymapScopeFilter`](crate::Intent::ToggleKeymapScopeFilter)
    /// to re-filter entries by scope without needing a live keymap reference.
    pub all_keymap_entries: Vec<KeymapEntry>,
}

impl Default for AppState {
    fn default() -> Self {
        let active_session = SessionId::new();
        let mut sessions = HashMap::new();
        sessions.insert(active_session.clone(), ChatSessionState::new());
        Self {
            sessions,
            active_session,
            mode: Mode::Normal,
            shutdown_tracker: ShutdownTrackerState::new(),
            dashboard: DashboardState::new(),
            active_tab: ActiveTab::Chat,
            should_quit: false,
            active_provider: NO_PROVIDER_ID.to_owned(),
            active_picker_kind: None,
            provider_picker: nullslop_selection_widget::SelectionState::new(),
            model_cache: None,
            last_refreshed_at: None,
            pinned_panel: PinnedPanelState::default(),
            strategy_state: HashMap::new(),
            context_strategy_picker: nullslop_selection_widget::SelectionState::new(),
            default_strategy: PromptStrategyId::passthrough(),
            prompt_templates: PromptTemplateStore::new(),
            keymap_picker: nullslop_selection_widget::SelectionState::new(),
            keymap_picker_show_all: false,
            keymap_picker_origin_scope: None,
            session_picker: nullslop_selection_widget::SelectionState::new(),
            session_loading: false,
            tui_signals: TuiSignals::new(),
            all_keymap_entries: vec![],
        }
    }
}

impl AppState {
    /// Read-only access to the active chat session.
    ///
    /// # Panics
    ///
    /// Panics if the active session does not exist in the sessions map.
    /// This should never happen in normal operation.
    #[expect(
        clippy::expect_used,
        reason = "active session invariant guaranteed by construction"
    )]
    pub fn active_session(&self) -> &ChatSessionState {
        self.sessions
            .get(&self.active_session)
            .expect("active session must exist")
    }

    /// Mutable access to the active chat session.
    ///
    /// # Panics
    ///
    /// Panics if the active session does not exist in the sessions map.
    /// This should never happen in normal operation.
    #[expect(
        clippy::expect_used,
        reason = "active session invariant guaranteed by construction"
    )]
    pub fn active_session_mut(&mut self) -> &mut ChatSessionState {
        self.sessions
            .get_mut(&self.active_session)
            .expect("active session must exist")
    }

    /// Read-only access to a session by ID.
    ///
    /// # Panics
    ///
    /// Panics if the given session ID does not exist in the sessions map.
    #[expect(
        clippy::expect_used,
        reason = "session invariant guaranteed by construction"
    )]
    pub fn session(&self, id: &SessionId) -> &ChatSessionState {
        self.sessions.get(id).expect("session must exist")
    }

    /// Mutable access to a session by ID.
    ///
    /// # Panics
    ///
    /// Panics if the given session ID does not exist in the sessions map.
    #[expect(
        clippy::expect_used,
        reason = "session invariant guaranteed by construction"
    )]
    pub fn session_mut(&mut self, id: &SessionId) -> &mut ChatSessionState {
        self.sessions.get_mut(id).expect("session must exist")
    }

    /// Returns mutable access to a session by ID, creating it if missing.
    ///
    /// Used by streaming handlers that receive tokens from actors
    /// (e.g. workflow executor) which may create new session IDs
    /// not yet present in the sessions map.
    pub fn session_mut_or_create(&mut self, id: &SessionId) -> &mut ChatSessionState {
        self.sessions.entry(id.clone()).or_default()
    }

    /// Read-only access to the active session's input box.
    ///
    /// Delegates to [`ChatSessionState::chat_input`] on the active session.
    ///
    /// # Panics
    ///
    /// Panics if the active session does not exist in the sessions map.
    pub fn active_chat_input(&self) -> &ChatInputBoxState {
        self.active_session().chat_input()
    }

    /// Mutable access to the active session's input box.
    ///
    /// Delegates to [`ChatSessionState::chat_input_mut`] on the active session.
    ///
    /// # Panics
    ///
    /// Panics if the active session does not exist in the sessions map.
    pub fn active_chat_input_mut(&mut self) -> &mut ChatInputBoxState {
        self.active_session_mut().chat_input_mut()
    }

    /// Returns pinned entry IDs sorted by position for the active session.
    ///
    /// Order: TOP entries first, then RELATIVE, then BOTTOM.
    /// Within each group, entries maintain their original history order (stable sort).
    #[must_use]
    pub fn sorted_pinned_ids(&self) -> Vec<ChatEntryId> {
        let mut pinned = self.active_session().pinned_entries();
        pinned.sort_by_key(|entry| pin_sort_key(entry.pin_position));
        pinned.iter().map(|e| e.id.clone()).collect()
    }

    /// The default strategy used for new sessions.
    pub fn default_strategy(&self) -> &PromptStrategyId {
        &self.default_strategy
    }

    /// Update the sticky default strategy for future sessions.
    pub fn set_default_strategy(&mut self, strategy: PromptStrategyId) {
        self.default_strategy = strategy;
    }
}

/// Returns the sort key for a pin position.
///
/// TOP = 0, RELATIVE (or None) = 1, BOTTOM = 2.
/// Used to sort pinned entries in display order.
#[must_use]
pub fn pin_sort_key(position: Option<PinPosition>) -> u8 {
    match position {
        Some(PinPosition::Top) => 0,
        Some(PinPosition::Relative) | None => 1,
        Some(PinPosition::Bottom) => 2,
    }
}

#[cfg(test)]
mod tests {
    use nullslop_protocol::ChatEntry;

    use super::*;

    #[rstest::rstest]
    fn app_state_default_has_empty_prompt_templates() {
        // Given a default AppState.
        let state = AppState::default();

        // Then the prompt template store is empty.
        assert!(state.prompt_templates.is_empty());
    }

    #[rstest::rstest]
    fn push_entry_adds_to_history() {
        // Given a new AppState.
        let mut data = AppState::default();
        let entry = ChatEntry::user("hello");

        // When pushing an entry via the active session.
        let index = data.active_session_mut().push_entry(entry);

        // Then the index is 0 and history has one entry.
        assert_eq!(index, 0);
        assert_eq!(data.active_session().history().len(), 1);
    }
}
