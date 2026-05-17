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
use crate::feat::theme::Theme;
pub use crate::feat::ui::sidebar::persona_section::PersonaSectionState;
pub use crate::feat::ui::sidebar::pins::state::PinsState;
pub use crate::feat::ui::sidebar::sessions::SessionsSectionState;
use crate::feat::ui::sidebar::state::SidebarState;
use crate::protocol::KeymapEntry;
use crate::protocol::SessionEntry;
use crate::protocol::StrategyEntry;

/// Session lifecycle state — owned by the session-actor.
///
/// Tracks an in-progress session load from disk.
///
/// Only one session can be loaded at a time. The guard is set by the
/// IntentHandler when the user confirms a session load, and cleared by
/// the session-actor on completion (or the TUI tick on timeout).
#[derive(Debug)]
pub struct SessionLoadGuard {
    /// Which session is being loaded.
    pub session_id: SessionId,
    /// When the load started — used for timeout detection.
    pub started_at: std::time::Instant,
}

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

    /// In-progress session load from disk, if any.
    /// OWNER: session-actor (clears on SessionLoadCompleted),
    ///        IntentHandler (sets on confirm_session),
    ///        TUI tick (clears on timeout).
    pub session_load_guard: Option<SessionLoadGuard>,

    /// The default CWD for new sessions, set once at startup from the process CWD.
    /// Used by `session_mut_or_create` to ensure every session has a valid CWD.
    pub default_cwd: std::path::PathBuf,
}

impl SessionState {
    /// Whether a session is currently being loaded from disk.
    pub fn is_loading(&self) -> bool {
        self.session_load_guard.is_some()
    }

    /// Begin loading a session. Sets the guard with the current timestamp.
    pub fn begin_load(&mut self, session_id: SessionId) {
        self.session_load_guard = Some(SessionLoadGuard {
            session_id,
            started_at: std::time::Instant::now(),
        });
    }

    /// Clear the loading guard (called on completion or timeout).
    pub fn clear_load(&mut self) {
        self.session_load_guard = None;
    }
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
            session_load_guard: None,
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

    /// Persona sidebar section state — cursor tracking.
    /// OWNER: IntentHandler (sidebar navigation).
    pub persona_section: PersonaSectionState,

    /// Sessions sidebar section state — cursor tracking.
    /// OWNER: IntentHandler (sidebar navigation).
    pub sessions_section: SessionsSectionState,

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

    /// The current resolved theme (colors for the render pipeline).
    /// OWNER: IntentHandler (theme picker preview), PreferencesStateSyncActor (on prefs change).
    pub theme: Theme,

    /// Whether the "Press ESC again to cancel" prompt is showing.
    /// OWNER: IntentHandler (set on first ESC in Normal/Sidebar with active stream,
    ///         consumed on second ESC or dismissed on any other key).
    pub cancel_stream_prompt: bool,

    /// Theme picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (theme picker navigation).
    pub theme_picker: nullslop_selection_widget::SelectionState<crate::feat::theme::ThemeEntry>,

    /// Saved theme before preview — restored on ESC.
    /// OWNER: IntentHandler (set on theme picker open, consumed on confirm/cancel).
    pub theme_preview_original: Option<Theme>,

    /// Fork picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (fork picker navigation).
    pub fork_picker:
        nullslop_selection_widget::SelectionState<crate::feat::session::fork_entry::ForkEntry>,

    /// All fork entries for the current session (pre-filter).
    /// OWNER: IntentHandler (populated when fork picker opens).
    pub all_fork_entries: Vec<crate::feat::session::fork_entry::ForkEntry>,

    /// Whether user messages are shown in the fork picker.
    /// OWNER: IntentHandler (toggled via ToggleForkUserFilter intent).
    pub fork_show_user: bool,

    /// Whether assistant messages are shown in the fork picker.
    /// OWNER: IntentHandler (toggled via ToggleForkAssistantFilter intent).
    pub fork_show_assistant: bool,

    /// Path to the themes directory (`~/.config/nullslop/themes/`).
    /// Set once during init from `AppPaths`. Used by the theme picker to discover themes.
    /// OWNER: Init code (set once at startup).
    pub themes_dir: std::path::PathBuf,
}

impl Default for FrontendState {
    fn default() -> Self {
        Self {
            active_tab: ActiveTab::Chat,
            should_quit: false,
            pins: PinsState::default(),
            sidebar: SidebarState::default(),
            persona_section: PersonaSectionState::default(),
            sessions_section: SessionsSectionState::default(),
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
            theme: crate::feat::theme::default_theme(),
            cancel_stream_prompt: false,
            theme_picker: nullslop_selection_widget::SelectionState::new(),
            theme_preview_original: None,
            fork_picker: nullslop_selection_widget::SelectionState::new(),
            all_fork_entries: vec![],
            fork_show_user: true,
            fork_show_assistant: true,
            themes_dir: std::path::PathBuf::new(),
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
            PickerKind::Theme => Some(&mut self.frontend.theme_picker),
            PickerKind::SessionFork => Some(&mut self.frontend.fork_picker),
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
