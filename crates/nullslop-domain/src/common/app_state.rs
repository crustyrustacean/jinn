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
    ActiveTab, ChatEntryId, Mode, PickerKind, PinPosition, SessionId, ToolDefinition,
};

use crate::common::tui_signals::TuiSignals;
pub use crate::feat::chat_input::ChatInputBoxState;
use crate::feat::context::prompt_template::PromptTemplateStore;
pub use crate::feat::dashboard::DashboardState;
use crate::feat::persona::Persona;
use crate::feat::persona::PersonaEntry;
use crate::feat::preferences_actor::UserPreferences;
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::skills::Skill;
pub use crate::feat::ui::sidebar::pins::state::PinsState;
use crate::feat::ui::sidebar::state::SidebarState;
use crate::feat::ui::status_bar::PluginSlotRegistry;
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

    /// The default CWD for new sessions, set once at startup from the process CWD.
    /// Used by `session_mut_or_create` to ensure every session has a valid CWD.
    pub default_cwd: std::path::PathBuf,
}

impl Default for SessionState {
    fn default() -> Self {
        let session = ChatSessionState::new();
        let active_session = session.session_id().clone();
        let mut sessions = HashMap::new();
        sessions.insert(active_session.clone(), session);
        Self {
            sessions,
            active_session,
            session_loading: false,
            session_load_started_at: None,
            default_cwd: std::path::PathBuf::from("/"),
        }
    }
}

/// Context assembly state — owned by the context-actor.
///
/// Written to exclusively by `PromptAssemblyActor` and `IntentHandler`.
/// No other actor should mutate these fields.
#[derive(Debug)]
pub struct ContextAssemblyState {
    /// Loaded prompt templates from `~/.config/nullslop/prompts/`.
    /// OWNER: context-actor (replaces on PromptTemplatesLoaded event).
    pub prompt_templates: PromptTemplateStore,

    /// Discovered agent skills from `~/.agents/skills/`.
    /// OWNER: skills-scan-actor (replaces on ScanSkills command).
    pub skills: Vec<Skill>,
    /// Discovered personas from `~/.config/nullslop/personas/`.
    /// OWNER: context-actor (replaces on PersonasLoaded event).
    pub personas: Vec<Persona>,
    /// The currently active persona (injected into system prompt).
    /// OWNER: context-actor (updated on PersonasLoaded, set on picker confirm).
    pub active_persona: Option<Persona>,
    /// Registered tool definitions, keyed by tool name.
    /// OWNER: tools-actor (populated on ToolsRegistered event), read by context-actor and llm-actor.
    pub tool_definitions: HashMap<String, ToolDefinition>,
}

impl Default for ContextAssemblyState {
    fn default() -> Self {
        Self {
            prompt_templates: PromptTemplateStore::new(),
            skills: Vec::new(),
            personas: Vec::new(),
            active_persona: None,
            tool_definitions: HashMap::new(),
        }
    }
}

/// Provider selection state — imported from `nsslice-provider-protocol`.
pub use crate::feat::provider::ProviderState;

/// A single focus context on the scope stack.
///
/// Each layer of the [`ScopeStack`] is a `FocusScope`. The top of the stack
/// determines the active mode, keymap scope, and which overlays are visible.
#[derive(Debug, Clone, PartialEq)]
pub enum FocusScope {
    /// Browsing chat entries (base scope).
    Normal,
    /// Typing into the input buffer.
    Input,
    /// Sidebar panel focused.
    Sidebar,
    /// Picker overlay active — kind distinguishes Provider/Session/Keymap/etc.
    Picker { kind: PickerKind },
}

impl FocusScope {
    /// Returns the [`Mode`] corresponding to this scope.
    #[must_use]
    pub fn mode(&self) -> Mode {
        match self {
            Self::Normal | Self::Sidebar => Mode::Normal,
            Self::Input => Mode::Input,
            Self::Picker { .. } => Mode::Picker,
        }
    }
}

impl std::fmt::Display for FocusScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normal => write!(f, "Normal"),
            Self::Input => write!(f, "Input"),
            Self::Sidebar => write!(f, "Sidebar"),
            Self::Picker { kind } => write!(f, "Picker({kind})"),
        }
    }
}

/// A LIFO stack of [`FocusScope`] layers.
///
/// Always has at least one entry (the base scope). Entering an overlay
/// pushes a new scope; escaping pops one level, restoring the previous scope.
#[derive(Debug, Clone)]
pub struct ScopeStack {
    stack: Vec<FocusScope>,
}

impl Default for ScopeStack {
    fn default() -> Self {
        Self {
            stack: vec![FocusScope::Normal],
        }
    }
}

impl ScopeStack {
    /// Pushes a new scope onto the stack (entering an overlay).
    pub fn push(&mut self, scope: FocusScope) {
        self.stack.push(scope);
    }

    /// Pops the top scope, returning it. Returns `None` if only the base remains.
    pub fn pop(&mut self) -> Option<FocusScope> {
        if self.stack.len() <= 1 {
            None
        } else {
            self.stack.pop()
        }
    }

    /// Returns the current (top) scope.
    ///
    /// # Panics
    ///
    /// Panics if the stack is empty (should never happen as the base is always present).
    #[must_use]
    pub fn current(&self) -> &FocusScope {
        self.stack.last().expect("stack always has base")
    }

    /// Returns the scope one level below the top (the "return target").
    ///
    /// Returns `None` if only the base scope is on the stack.
    #[must_use]
    pub fn parent(&self) -> Option<&FocusScope> {
        if self.stack.len() < 2 {
            None
        } else {
            self.stack.get(self.stack.len() - 2)
        }
    }

    /// Pops all overlay scopes, returning to the base scope.
    pub fn clear_overlays(&mut self) {
        self.stack.truncate(1);
    }

    /// Returns `true` if the current scope is a Picker.
    #[must_use]
    pub fn is_picker(&self) -> bool {
        matches!(self.current(), FocusScope::Picker { .. })
    }

    /// Returns the `PickerKind` if the current scope is a Picker.
    #[must_use]
    pub fn picker_kind(&self) -> Option<&PickerKind> {
        match self.current() {
            FocusScope::Picker { kind } => Some(kind),
            _ => None,
        }
    }

    /// Returns `true` if the current scope is Sidebar.
    #[must_use]
    pub fn is_sidebar(&self) -> bool {
        matches!(self.current(), FocusScope::Sidebar)
    }

    /// Returns `true` if the stack has no scopes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// Returns the number of scopes on the stack.
    #[must_use]
    pub fn len(&self) -> usize {
        self.stack.len()
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
    /// The currently active tab.
    /// OWNER: IntentHandler (tab switching).
    pub active_tab: ActiveTab,

    /// Set to `true` when the user has requested to quit.
    /// OWNER: IntentHandler (Quit intent),
    ///        shutdown-tracker (ProceedWithShutdown command).
    pub should_quit: bool,

    /// Pins sidebar section state — selection index within the pinned entries list.
    /// OWNER: IntentHandler (pins navigation).
    pub pins: PinsState,

    /// Sidebar state — focus tracking.
    /// OWNER: IntentHandler (sidebar focus/leave).
    pub sidebar: SidebarState,

    /// Actor dashboard — tracks registered actors and their status.
    /// OWNER: IntentHandler (dashboard navigation).
    pub dashboard: DashboardState,

    /// Signals from the IntentHandler for the outer platform layer.
    /// OWNER: IntentHandler (cleared and set each handle() call).
    pub tui_signals: TuiSignals,

    /// Cached copy of user preferences from `nullslop.toml`.
    /// Updated exclusively by `PreferencesStateSyncActor` on `PreferencesUpdated` events.
    /// This is a cache — the file is the authoritative source.
    pub preferences: UserPreferences,

    /// All keymap entries, populated once at startup.
    /// OWNER: IntentHandler (populated when keymap picker opens).
    pub all_keymap_entries: Vec<KeymapEntry>,

    /// Keymap picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (keymap picker navigation).
    pub keymap_picker: nullslop_selection_widget::SelectionState<KeymapEntry>,

    /// Whether the keymap picker shows all scopes or current scope only.
    /// OWNER: IntentHandler (toggle filter).
    pub keymap_picker_show_all: bool,

    /// Session picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (session picker navigation).
    pub session_picker: nullslop_selection_widget::SelectionState<SessionEntry>,

    /// Context strategy picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (strategy picker navigation).
    pub context_strategy_picker: nullslop_selection_widget::SelectionState<StrategyEntry>,
    /// Persona picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (persona picker navigation).
    pub persona_picker: nullslop_selection_widget::SelectionState<PersonaEntry>,

    /// Transient status bar notification (auto-dismisses after 3 seconds).
    /// OWNER: TUI render loop (sets on clipboard copy), tick handler (clears expired).
    pub status_notification: Option<StatusNotification>,

    /// Focus scope stack — single source of truth for what the user is focused on.
    /// OWNER: IntentHandler (push/pop on scope transitions).
    pub scope_stack: ScopeStack,

    /// Whether the "Press ESC again to cancel" prompt is showing.
    /// OWNER: IntentHandler (set on first ESC in Normal/Sidebar with active stream,
    ///         consumed on second ESC or dismissed on any other key).
    pub cancel_stream_prompt: bool,
}

impl Default for FrontendState {
    fn default() -> Self {
        Self {
            active_tab: ActiveTab::Chat,
            should_quit: false,
            pins: PinsState::default(),
            sidebar: SidebarState::default(),
            dashboard: DashboardState::new(),
            tui_signals: TuiSignals::new(),
            preferences: UserPreferences::default(),
            all_keymap_entries: vec![],
            keymap_picker: nullslop_selection_widget::SelectionState::new(),
            keymap_picker_show_all: false,
            session_picker: nullslop_selection_widget::SelectionState::new(),
            context_strategy_picker: nullslop_selection_widget::SelectionState::new(),
            persona_picker: nullslop_selection_widget::SelectionState::new(),
            status_notification: None,
            scope_stack: ScopeStack::default(),
            cancel_stream_prompt: false,
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
    /// Frontend / UI state — owned by IntentHandler.
    pub frontend: FrontendState,
    /// Plugin status bar slots — owned by plugin-actor.
    pub plugin_slots: PluginSlotRegistry,
}

impl AppState {
    /// Returns a mutable reference to the active picker's navigation interface.
    ///
    /// Returns `None` if no picker is currently active.
    /// Use for operations that work the same way on all picker types
    /// (insert char, backspace, move up/down, cursor left/right).
    pub fn active_picker_ops(&mut self) -> Option<&mut dyn nullslop_selection_widget::PickerOps> {
        let kind = self.frontend.scope_stack.picker_kind().copied()?;
        match kind {
            PickerKind::Provider => Some(&mut self.provider.provider_picker),
            PickerKind::ContextAssembly => Some(&mut self.frontend.context_strategy_picker),
            PickerKind::Keymap => Some(&mut self.frontend.keymap_picker),
            PickerKind::Session => Some(&mut self.frontend.session_picker),
            PickerKind::Persona => Some(&mut self.frontend.persona_picker),
        }
    }

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
        let default_cwd = self.session.default_cwd.clone();
        self.session.sessions.entry(id.clone()).or_insert_with(|| {
            let mut s = ChatSessionState::new();
            s.set_session_id(id.clone());
            s.set_cwd(default_cwd.clone());
            s
        })
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
        let mut state = FrontendState {
            status_notification: Some(StatusNotification {
                message: "old".to_owned(),
                // Created 10 seconds ago — expired.
                created_at: std::time::Instant::now()
                    .checked_sub(std::time::Duration::from_secs(10))
                    .unwrap(),
            }),
            ..Default::default()
        };

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

    // --- ScopeStack tests ---

    #[rstest::rstest]
    fn default_creates_normal_base() {
        // Given a default ScopeStack.
        let stack = ScopeStack::default();

        // Then the current scope is Normal.
        assert_eq!(stack.current(), &FocusScope::Normal);
    }

    #[rstest::rstest]
    fn push_and_pop_round_trip() {
        // Given a default ScopeStack.
        let mut stack = ScopeStack::default();

        // When pushing Input.
        stack.push(FocusScope::Input);

        // Then current is Input.
        assert_eq!(stack.current(), &FocusScope::Input);

        // When popping.
        let popped = stack.pop();

        // Then we get Input back and current is Normal.
        assert_eq!(popped, Some(FocusScope::Input));
        assert_eq!(stack.current(), &FocusScope::Normal);
    }

    #[rstest::rstest]
    fn pop_on_base_returns_none() {
        // Given a default ScopeStack (only base).
        let mut stack = ScopeStack::default();

        // When popping the base.
        let popped = stack.pop();

        // Then nothing is returned.
        assert!(popped.is_none());
        // And the base scope remains.
        assert_eq!(stack.current(), &FocusScope::Normal);
    }

    #[rstest::rstest]
    fn parent_returns_none_on_base() {
        // Given a default ScopeStack (only base).
        let stack = ScopeStack::default();

        // Then parent is None.
        assert!(stack.parent().is_none());
    }

    #[rstest::rstest]
    fn parent_returns_previous_after_push() {
        // Given a ScopeStack with Input pushed.
        let mut stack = ScopeStack::default();
        stack.push(FocusScope::Input);

        // Then parent is Normal.
        assert_eq!(stack.parent(), Some(&FocusScope::Normal));
    }

    #[rstest::rstest]
    fn clear_overlays_returns_to_base() {
        // Given a ScopeStack with multiple overlays.
        let mut stack = ScopeStack::default();
        stack.push(FocusScope::Input);
        stack.push(FocusScope::Picker {
            kind: PickerKind::Provider,
        });

        // When clearing overlays.
        stack.clear_overlays();

        // Then current is Normal.
        assert_eq!(stack.current(), &FocusScope::Normal);
        assert_eq!(stack.len(), 1);
    }

    #[rstest::rstest]
    fn is_picker_returns_true_when_picker_active() {
        // Given a ScopeStack with Picker on top.
        let mut stack = ScopeStack::default();
        stack.push(FocusScope::Picker {
            kind: PickerKind::Session,
        });

        // Then is_picker is true.
        assert!(stack.is_picker());
    }

    #[rstest::rstest]
    fn is_picker_returns_false_when_input_active() {
        // Given a ScopeStack with Input on top.
        let mut stack = ScopeStack::default();
        stack.push(FocusScope::Input);

        // Then is_picker is false.
        assert!(!stack.is_picker());
    }

    #[rstest::rstest]
    fn picker_kind_returns_kind_when_picker_active() {
        // Given a ScopeStack with Picker(Provider) on top.
        let mut stack = ScopeStack::default();
        stack.push(FocusScope::Picker {
            kind: PickerKind::Provider,
        });

        // Then picker_kind returns Provider.
        assert_eq!(stack.picker_kind(), Some(&PickerKind::Provider));
    }

    #[rstest::rstest]
    fn picker_kind_returns_none_when_not_picker() {
        // Given a default ScopeStack.
        let stack = ScopeStack::default();

        // Then picker_kind is None.
        assert!(stack.picker_kind().is_none());
    }

    #[rstest::rstest]
    fn is_sidebar_returns_true_when_sidebar_active() {
        // Given a ScopeStack with Sidebar on top.
        let mut stack = ScopeStack::default();
        stack.push(FocusScope::Sidebar);

        // Then is_sidebar is true.
        assert!(stack.is_sidebar());
    }

    #[rstest::rstest]
    fn is_sidebar_returns_false_when_normal() {
        // Given a default ScopeStack.
        let stack = ScopeStack::default();

        // Then is_sidebar is false.
        assert!(!stack.is_sidebar());
    }

    // --- FocusScope::mode() parameterized ---

    #[rstest::rstest]
    #[case(FocusScope::Normal, Mode::Normal)]
    #[case(FocusScope::Input, Mode::Input)]
    #[case(FocusScope::Sidebar, Mode::Normal)]
    #[case(FocusScope::Picker { kind: PickerKind::Provider }, Mode::Picker)]
    fn focus_scope_mode_mapping(#[case] scope: FocusScope, #[case] expected: Mode) {
        // Given a FocusScope variant.
        // When calling mode().
        // Then it returns the expected Mode.
        assert_eq!(scope.mode(), expected);
    }

    // --- FocusScope::Display ---

    #[rstest::rstest]
    #[case(FocusScope::Normal, "Normal")]
    #[case(FocusScope::Input, "Input")]
    #[case(FocusScope::Sidebar, "Sidebar")]
    #[case(FocusScope::Picker { kind: PickerKind::Provider }, "Picker(models)")]
    fn focus_scope_display(#[case] scope: FocusScope, #[case] expected: &str) {
        // Given a FocusScope variant.
        // When formatting as Display.
        // Then it produces the expected string.
        assert_eq!(scope.to_string(), expected);
    }

    // --- session_mut_or_create CWD ---

    #[rstest::rstest]
    fn session_mut_or_create_sets_cwd_from_default_cwd() {
        // Given an AppState with a custom default CWD.
        let mut state = AppState::default();
        state.session.default_cwd = std::path::PathBuf::from("/custom/cwd");

        let session_id = SessionId::new();

        // When creating a session via session_mut_or_create.
        let session = state.session_mut_or_create(&session_id);

        // Then the session's CWD is the default CWD.
        assert_eq!(session.cwd(), std::path::Path::new("/custom/cwd"));
    }

    #[rstest::rstest]
    fn session_mut_or_create_does_not_overwrite_existing_session_cwd() {
        // Given an AppState with a session that has a specific CWD.
        let mut state = AppState::default();
        state.session.default_cwd = std::path::PathBuf::from("/new/default");

        let session_id = SessionId::new();
        {
            let session = state.session_mut_or_create(&session_id);
            session.set_cwd(std::path::PathBuf::from("/existing/cwd"));
        }

        // When accessing the same session via session_mut_or_create.
        let session = state.session_mut_or_create(&session_id);

        // Then the CWD is unchanged.
        assert_eq!(session.cwd(), std::path::Path::new("/existing/cwd"));
    }
}
