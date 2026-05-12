//! Shared application state.
//!
//! [`AppState`] is the single source of truth for what the user sees and how the
//! application is currently behaving. Every component reads from and writes to this
//! shared state.
//!
//! Fields are grouped into owner-named structs (`Session`, `Context`, `Provider`,
//! `Shutdown`, `Frontend`) to make cross-boundary writes visually obvious during
//! code review. Each group struct carries `/// OWNER:` documentation on the struct
//! and on each field.

use std::collections::HashMap;

use crate::protocol::{
    ActiveTab, ChatEntryId, Mode, PickerKind, PinPosition, PromptStrategyId, SessionId,
};
use serde_json::Value as JsonValue;

use crate::common::tui_signals::TuiSignals;
pub use crate::feat::chat_input::ChatInputBoxState;
use crate::feat::context::prompt_template::PromptTemplateStore;
pub use crate::feat::dashboard::DashboardState;
pub use crate::feat::pinned_panel::PinnedPanelState;
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::shutdown_actor::ShutdownTrackerState;
use crate::feat::skills::Skill;
use crate::protocol::KeymapEntry;
use crate::protocol::SessionEntry;
use crate::protocol::StrategyEntry;

/// Session lifecycle state — owned by the session-actor.
///
/// Written to exclusively by `SessionPersistenceActor` and `IntentHandler`.
/// No other actor should mutate these fields.
#[derive(Debug)]
pub struct SessionState {
    /// All chat sessions, keyed by session ID.
    /// OWNER: session-actor (creates/removes sessions, restores history),
    ///        IntentHandler (creates new sessions, reads for dispatch).
    pub sessions: HashMap<SessionId, ChatSessionState>,

    /// The currently active session ID.
    /// OWNER: session-actor (sets on SessionLoadCompleted),
    ///        IntentHandler (sets on SessionNew).
    pub active_session: SessionId,

    /// Whether a session is currently being loaded from disk.
    /// OWNER: session-actor (clears on SessionLoadCompleted),
    ///        IntentHandler (sets true on confirm_session).
    pub session_loading: bool,
    /// When the current session load started. Used for timeout detection.
    /// Set by IntentHandler (on confirm_session), cleared by session-actor (on load completed)
    /// and TUI tick (on timeout).
    pub session_load_started_at: Option<std::time::Instant>,
}

impl Default for SessionState {
    fn default() -> Self {
        let active_session = SessionId::new();
        let mut sessions = HashMap::new();
        sessions.insert(active_session.clone(), ChatSessionState::new());
        Self {
            sessions,
            active_session,
            session_loading: false,
            session_load_started_at: None,
        }
    }
}

/// Context assembly state — owned by the context-actor.
///
/// Written to exclusively by `PromptAssemblyActor` and `IntentHandler`.
/// No other actor should mutate these fields.
#[derive(Debug)]
pub struct ContextAssemblyState {
    /// Persisted strategy state blobs, keyed by (session_id, strategy_id).
    /// OWNER: context-actor (reads/writes during RestoreStrategyState, SwitchPromptStrategy).
    pub strategy_state: HashMap<(SessionId, PromptStrategyId), JsonValue>,

    /// Loaded prompt templates from `~/.config/nullslop/prompts/`.
    /// OWNER: context-actor (replaces on PromptTemplatesLoaded event).
    pub prompt_templates: PromptTemplateStore,

    /// Discovered agent skills from `~/.agents/skills/`.
    /// OWNER: skills-scan-actor (replaces on ScanSkills command).
    pub skills: Vec<Skill>,
}

impl Default for ContextAssemblyState {
    fn default() -> Self {
        Self {
            strategy_state: HashMap::new(),
            prompt_templates: PromptTemplateStore::new(),
            skills: Vec::new(),
        }
    }
}

/// Provider selection state — imported from `nsslice-provider-protocol`.
pub use crate::feat::provider::ProviderState;

/// Shutdown coordination state — owned by the shutdown-tracker actor.
///
/// Written to exclusively by `ShutdownTrackerActor` and `IntentHandler`.
/// No other actor should mutate these fields.
#[derive(Debug)]
pub struct ShutdownCoordinatorState {
    /// Bookkeeping for which actors are still running during shutdown.
    /// OWNER: shutdown-tracker (tracks start/complete lifecycle),
    ///        IntentHandler (sets should_quit),
    ///        AppCore (calls begin_shutdown).
    pub shutdown_tracker: ShutdownTrackerState,
}

impl Default for ShutdownCoordinatorState {
    fn default() -> Self {
        Self {
            shutdown_tracker: ShutdownTrackerState::new(),
        }
    }
}

/// A transient status bar notification with auto-expiry.
///
/// Created with a timestamp and lazily checked for expiry during rendering.
/// No background timer — the renderer checks elapsed time each frame.
#[derive(Debug)]
pub struct StatusNotification {
    /// The notification message text.
    pub message: String,
    /// When this notification was created.
    pub created_at: std::time::Instant,
}

/// Frontend / UI state — owned by the IntentHandler (main thread).
///
/// Written to by `IntentHandler` and various UI elements (read-only).
/// Actors should NOT write to these fields — they are for the frontend only.
#[derive(Debug)]
pub struct FrontendState {
    /// Whether the user is browsing or actively typing.
    /// OWNER: IntentHandler (all mode transitions).
    pub mode: Mode,

    /// The currently active tab.
    /// OWNER: IntentHandler (tab switching).
    pub active_tab: ActiveTab,

    /// Set to `true` when the user has requested to quit.
    /// OWNER: IntentHandler (Quit intent),
    ///        shutdown-tracker (ProceedWithShutdown command).
    pub should_quit: bool,

    /// Which picker is currently active. `None` when not in picker mode.
    /// OWNER: IntentHandler (open/close picker).
    pub active_picker_kind: Option<PickerKind>,

    /// Pinned panel state — selection index within the pinned entries list.
    /// OWNER: IntentHandler (pinned panel navigation).
    pub pinned_panel: PinnedPanelState,

    /// Actor dashboard — tracks registered actors and their status.
    /// OWNER: IntentHandler (dashboard navigation).
    pub dashboard: DashboardState,

    /// Signals from the IntentHandler for the outer platform layer.
    /// OWNER: IntentHandler (cleared and set each handle() call).
    pub tui_signals: TuiSignals,

    /// The default strategy for new sessions.
    /// OWNER: IntentHandler (updated when user confirms strategy selection).
    pub default_strategy: PromptStrategyId,

    /// All keymap entries, populated once at startup.
    /// OWNER: IntentHandler (populated when keymap picker opens).
    pub all_keymap_entries: Vec<KeymapEntry>,

    /// Keymap picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (keymap picker navigation).
    pub keymap_picker: nullslop_selection_widget::SelectionState<KeymapEntry>,

    /// Whether the keymap picker shows all scopes or current scope only.
    /// OWNER: IntentHandler (toggle filter).
    pub keymap_picker_show_all: bool,

    /// The scope the user was in when they opened the keymap picker.
    /// OWNER: IntentHandler (set on open, cleared on close).
    pub keymap_picker_origin_scope: Option<String>,

    /// Session picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (session picker navigation).
    pub session_picker: nullslop_selection_widget::SelectionState<SessionEntry>,

    /// Context strategy picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (strategy picker navigation).
    pub context_strategy_picker: nullslop_selection_widget::SelectionState<StrategyEntry>,

    /// Transient status bar notification (auto-dismisses after 3 seconds).
    /// OWNER: TUI render loop (sets on clipboard copy), tick handler (clears expired).
    pub status_notification: Option<StatusNotification>,
}

impl Default for FrontendState {
    fn default() -> Self {
        Self {
            mode: Mode::Normal,
            active_tab: ActiveTab::Chat,
            should_quit: false,
            active_picker_kind: None,
            pinned_panel: PinnedPanelState::default(),
            dashboard: DashboardState::new(),
            tui_signals: TuiSignals::new(),
            default_strategy: PromptStrategyId::passthrough(),
            all_keymap_entries: vec![],
            keymap_picker: nullslop_selection_widget::SelectionState::new(),
            keymap_picker_show_all: false,
            keymap_picker_origin_scope: None,
            session_picker: nullslop_selection_widget::SelectionState::new(),
            context_strategy_picker: nullslop_selection_widget::SelectionState::new(),
            status_notification: None,
        }
    }
}

impl FrontendState {
    /// Sets a transient status bar notification.
    pub fn set_status_notification(&mut self, message: impl Into<String>) {
        self.status_notification = Some(StatusNotification {
            message: message.into(),
            created_at: std::time::Instant::now(),
        });
    }

    /// Returns the active notification message if it hasn't expired (3 seconds).
    pub fn active_status_notification(&self) -> Option<&str> {
        self.status_notification
            .as_ref()
            .filter(|n| n.created_at.elapsed().as_secs() < 3)
            .map(|n| n.message.as_str())
    }

    /// Clears the notification if it has expired (3 seconds).
    pub fn clear_expired_notification(&mut self) {
        if let Some(ref n) = self.status_notification
            && n.created_at.elapsed().as_secs() >= 3
        {
            self.status_notification = None;
        }
    }
}

/// A snapshot of everything the application is doing right now.
#[derive(Debug, Default)]
pub struct AppState {
    /// Session lifecycle state — owned by session-actor.
    pub session: SessionState,
    /// Context assembly state — owned by context-actor.
    pub context: ContextAssemblyState,
    /// Provider selection state — owned by provider-actor.
    pub provider: ProviderState,
    /// Shutdown coordination state — owned by shutdown-tracker.
    pub shutdown: ShutdownCoordinatorState,
    /// Frontend / UI state — owned by IntentHandler.
    pub frontend: FrontendState,
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
        self.session
            .sessions
            .get(&self.session.active_session)
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
        self.session
            .sessions
            .get_mut(&self.session.active_session)
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
        self.session.sessions.get(id).expect("session must exist")
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
        self.session
            .sessions
            .get_mut(id)
            .expect("session must exist")
    }

    /// Returns mutable access to a session by ID, creating it if missing.
    ///
    /// Used by streaming handlers that receive tokens from actors
    /// (e.g. workflow executor) which may create new session IDs
    /// not yet present in the sessions map.
    pub fn session_mut_or_create(&mut self, id: &SessionId) -> &mut ChatSessionState {
        self.session.sessions.entry(id.clone()).or_default()
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
        &self.frontend.default_strategy
    }

    /// Update the sticky default strategy for future sessions.
    pub fn set_default_strategy(&mut self, strategy: PromptStrategyId) {
        self.frontend.default_strategy = strategy;
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
    use crate::protocol::ChatEntry;

    use super::*;

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

    // --- StatusNotification tests ---

    #[rstest::rstest]
    fn set_status_notification_stores_message() {
        // Given a default FrontendState.
        let mut state = FrontendState::default();

        // When setting a notification.
        state.set_status_notification("Copied to clipboard");

        // Then active_status_notification returns the message.
        assert_eq!(
            state.active_status_notification(),
            Some("Copied to clipboard")
        );
    }

    #[rstest::rstest]
    fn active_status_notification_returns_none_when_unset() {
        // Given a default FrontendState.
        let state = FrontendState::default();

        // When checking for an active notification.
        // Then it returns None.
        assert_eq!(state.active_status_notification(), None);
    }

    #[rstest::rstest]
    fn clear_expired_notification_removes_when_old() {
        // Given a FrontendState with a manually constructed expired notification.
        let mut state = FrontendState::default();
        state.status_notification = Some(StatusNotification {
            message: "old".to_owned(),
            // Created 10 seconds ago — expired.
            created_at: std::time::Instant::now() - std::time::Duration::from_secs(10),
        });

        // When clearing expired notifications.
        state.clear_expired_notification();

        // Then the notification is removed.
        assert!(state.status_notification.is_none());
    }

    #[rstest::rstest]
    fn clear_expired_notification_keeps_when_fresh() {
        // Given a FrontendState with a fresh notification.
        let mut state = FrontendState::default();
        state.set_status_notification("fresh");

        // When clearing expired notifications.
        state.clear_expired_notification();

        // Then the notification is still present.
        assert!(state.status_notification.is_some());
    }
}
