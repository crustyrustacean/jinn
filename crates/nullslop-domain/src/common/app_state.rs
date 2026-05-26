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

use parking_lot::RwLock;

use crate::protocol::{ChatEntryId, Mode, PickerKind, PinPosition, SessionId, ToolDefinition};

use crate::common::session_map::SessionMap;
use crate::common::tui_signals::TuiSignals;
pub use crate::feat::chat_input::ChatInputBoxState;
use crate::feat::context::env_context::ContextFile;
use crate::feat::context::prompt_template::PromptTemplateStore;
use crate::feat::persona::Persona;
use crate::feat::persona::PersonaEntry;
use crate::feat::preferences_actor::UserPreferences;
use crate::feat::session::chat_session::ChatSessionState;
use crate::feat::session_lifecycle::picker_entry::SessionLifecycleEntry;

/// State for the arg input popup — collecting positional args for a lifecycle command.
#[derive(Debug, Clone, Default)]
pub struct ArgInputState {
    /// Which lifecycle we're collecting args for.
    pub lifecycle_name: String,
    /// The command template with `<param>` tokens for display.
    pub template_display: String,
    /// User's raw input text.
    pub input: String,
    /// Byte offset for cursor position in the input.
    pub cursor_pos: usize,
}

/// State for the token budget input popup — typing a numeric budget value.
#[derive(Debug, Clone, Default)]
pub struct TokenBudgetInputState {
    /// User's raw input text (digits only).
    pub input: String,
    /// Byte offset for cursor position in the input.
    pub cursor_pos: usize,
    /// In-popup error message (e.g., "Paste rejected: digits only").
    /// Set when paste is rejected, cleared on any subsequent input.
    pub error_message: Option<String>,
}

/// State for the sliding window input popup — typing a numeric window size.
#[derive(Debug, Clone, Default)]
pub struct SlidingWindowInputState {
    /// User's raw input text (digits only).
    pub input: String,
    /// Byte offset for cursor position in the input.
    pub cursor_pos: usize,
    /// In-popup error message (e.g., "Paste rejected: digits only").
    /// Set when paste is rejected, cleared on any subsequent input.
    pub error_message: Option<String>,
}

/// State for the rename session input popup — editing a session title.
#[derive(Debug, Clone, Default)]
pub struct RenameSessionInputState {
    /// User's raw input text.
    pub input: String,
    /// Byte offset for cursor position in the input.
    pub cursor_pos: usize,
}
use crate::feat::session::picker_entry::SessionTreeEntry;
use crate::feat::skills::Skill;
use crate::feat::theme::Theme;
pub use crate::feat::ui::sidebar::persona_section::PersonaSectionState;
pub use crate::feat::ui::sidebar::pins::state::PinsState;
use crate::feat::ui::sidebar::section_trait::SidebarSectionId;
pub use crate::feat::ui::sidebar::sessions::SessionsSectionState;
use crate::feat::ui::sidebar::state::SidebarState;

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
///
/// See [`SessionMap`] for the full API.
pub type SessionState = SessionMap;

/// Context assembly state — owned by the context-actor.
///
/// Written to exclusively by `SessionActor` and `IntentHandler`.
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
    /// Cached project context files (AGENTS.md, CLAUDE.md).
    /// OWNER: populated on startup, refreshed on session/CWD change.
    pub context_files: Vec<ContextFile>,
    /// Discovered judges from `~/.config/nullslop/judges/`.
    /// OWNER: session-actor (replaces on JudgesLoaded event).
    pub judges: Vec<crate::feat::judge::Judge>,
    /// Loaded compaction system prompt from `~/.config/nullslop/prompts/_compaction.md`.
    /// OWNER: populated once at startup by the app init code.
    pub compaction_prompt: String,
}

impl Default for ContextAssemblyState {
    fn default() -> Self {
        Self {
            prompt_templates: PromptTemplateStore::new(),
            skills: Vec::new(),
            personas: Vec::new(),
            active_persona: None,
            tool_definitions: HashMap::new(),
            context_files: Vec::new(),
            judges: Vec::new(),
            compaction_prompt: String::new(),
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
    /// Sidebar — Persona section focused.
    SidebarPersona,
    /// Sidebar — Pins section focused.
    SidebarPins,
    /// Sidebar — Sessions section focused.
    SidebarSessions,
    /// Picker overlay active — kind distinguishes Provider/Session/Keymap/etc.
    Picker { kind: PickerKind },
    /// Arg input popup — collecting positional args for a lifecycle command.
    ArgInput,
    /// Token budget input popup — typing a numeric budget value.
    /// Sliding window input popup — typing a numeric window size.
    /// Rename session input popup — editing a session title.
    RenameSessionInput,
    /// Sidebar resize mode — adjusting sidebar width with h/l keys.
    SidebarResize,
    /// Workflow tab — browsing workflow node status.
    Workflow,
    /// Workflow input editing — typing into the source node output buffer.
    WorkflowInput,
}

impl FocusScope {
    /// Returns the [`Mode`] corresponding to this scope.
    #[must_use]
    pub fn mode(&self) -> Mode {
        match self {
            Self::Normal
            | Self::SidebarPersona
            | Self::SidebarPins
            | Self::SidebarSessions
            | Self::SidebarResize
            | Self::Workflow => Mode::Normal,
            Self::Input | Self::ArgInput | Self::RenameSessionInput | Self::WorkflowInput => {
                Mode::Input
            }
            Self::Picker { .. } => Mode::Picker,
        }
    }
}

impl std::fmt::Display for FocusScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normal => write!(f, "Normal"),
            Self::Input => write!(f, "Input"),
            Self::SidebarPersona => write!(f, "SidebarPersona"),
            Self::SidebarPins => write!(f, "SidebarPins"),
            Self::SidebarSessions => write!(f, "SidebarSessions"),
            Self::Picker { kind } => write!(f, "Picker({kind})"),
            Self::ArgInput => write!(f, "ArgInput"),
            Self::RenameSessionInput => write!(f, "RenameSessionInput"),
            Self::SidebarResize => write!(f, "SidebarResize"),
            Self::Workflow => write!(f, "Workflow"),
            Self::WorkflowInput => write!(f, "WorkflowInput"),
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
        #[expect(clippy::expect_used, reason = "ScopeStack invariant: always has base")]
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

    /// Replaces the base scope with `new_base` and clears all overlays.
    ///
    /// Use when transitioning between top-level contexts (e.g., Chat → Workflow)
    /// where the entire scope stack should be replaced, not just pushed onto.
    pub fn swap_base(&mut self, new_base: FocusScope) {
        self.stack.clear();
        self.stack.push(new_base);
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

    /// Returns `true` if the current scope is a sidebar section.
    #[must_use]
    pub fn is_sidebar(&self) -> bool {
        matches!(
            self.current(),
            FocusScope::SidebarPersona | FocusScope::SidebarPins | FocusScope::SidebarSessions
        )
    }

    /// Returns the focused sidebar section, if a sidebar scope is active.
    #[must_use]
    pub fn sidebar_section(&self) -> Option<SidebarSectionId> {
        match self.current() {
            FocusScope::SidebarPersona => Some(SidebarSectionId::Persona),
            FocusScope::SidebarPins => Some(SidebarSectionId::Pins),
            FocusScope::SidebarSessions => Some(SidebarSectionId::Sessions),
            _ => None,
        }
    }

    /// Swaps the top of the scope stack to a different sidebar section.
    ///
    /// No-op if the current scope is not a sidebar section.
    pub fn set_sidebar_section(&mut self, section: SidebarSectionId) {
        if self.is_sidebar() {
            let scope = match section {
                SidebarSectionId::Persona => FocusScope::SidebarPersona,
                SidebarSectionId::Pins => FocusScope::SidebarPins,
                SidebarSectionId::Sessions => FocusScope::SidebarSessions,
            };
            self.stack.pop();
            self.stack.push(scope);
        }
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

use nullslop_workflow::spatial_layout::SpatialRect;
/// Workflow tab UI state — persisted across frames in `FrontendState`.
///
use std::sync::atomic::{AtomicU16, Ordering};

/// OWNER: IntentHandler (selection, inspector toggle, cancel prompt).
#[derive(Debug, Default)]
pub struct WorkflowUiState {
    /// Currently selected node name, if any.
    pub selected_node: Option<String>,
    /// Viewport horizontal offset (cells).
    pub viewport_offset_x: i32,
    /// Viewport vertical offset (cells).
    pub viewport_offset_y: i32,
    /// Whether the sticky inspector popup is showing.
    pub inspector_open: bool,
    /// Scroll position within the inspector popup (lines from top).
    pub inspector_scroll: u16,
    /// The actual clamped scroll position after rendering.
    ///
    /// Written by the renderer each frame, read by intent handlers
    /// so repeated "scroll down" inputs don't accumulate past the limit.
    pub inspector_scroll_rendered: AtomicU16,
    /// Whether the "Press ESC again to cancel" prompt is showing.
    pub cancel_prompt: bool,
    /// Cached spatial index: node name → bounding rect in content coordinates.
    ///
    /// Recomputed lazily when empty and a spatial navigation intent fires.
    /// Cleared when the active workflow changes.
    pub node_rects: HashMap<String, SpatialRect>,
    /// The text editing buffer for the workflow node being edited.
    /// Reuses `ChatInputBoxState` for cursor, wrapping, and scroll management.
    pub input_buffer: ChatInputBoxState,
    /// The name of the source node currently being edited, if any.
    pub editing_node: Option<String>,
}

impl Clone for WorkflowUiState {
    fn clone(&self) -> Self {
        Self {
            selected_node: self.selected_node.clone(),
            viewport_offset_x: self.viewport_offset_x,
            viewport_offset_y: self.viewport_offset_y,
            inspector_open: self.inspector_open,
            inspector_scroll: self.inspector_scroll,
            inspector_scroll_rendered: AtomicU16::new(
                self.inspector_scroll_rendered.load(Ordering::Relaxed),
            ),
            cancel_prompt: self.cancel_prompt,
            node_rects: self.node_rects.clone(),
            input_buffer: self.input_buffer.clone(),
            editing_node: self.editing_node.clone(),
        }
    }
}

/// Theme-sensitive caches owned by the frontend.
///
/// All caches that store pre-rendered styled data (which embeds theme colors)
/// live here so they can be invalidated in one call when the theme changes.
///
/// Each cache is wrapped in a `RwLock` so render code can borrow mutably
/// while holding shared references to the rest of `AppState`.
#[derive(Debug, Default)]
pub struct FrontendCaches {
    /// Cached wrapped line counts and rendered lines per chat entry.
    pub entry_line_cache: RwLock<crate::feat::ui::chat_log::line_count_cache::EntryLineCache>,
    /// Cached rendered lines for session preview popups.
    pub session_preview_cache: RwLock<crate::feat::ui::sidebar::sessions::preview::SessionPreviewCache>,
}

impl FrontendCaches {
    /// Invalidate all caches. Called when the active theme changes.
    pub fn invalidate_all(&self) {
        self.entry_line_cache.write().clear();
        self.session_preview_cache.write().clear();
    }
}

/// Frontend / UI state — owned by the IntentHandler (main thread).
///
/// Written to by `IntentHandler` and various UI elements (read-only).
/// Actors should NOT write to these fields — they are for the frontend only.
#[derive(Debug)]
pub struct FrontendState {
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

    /// Signals from the IntentHandler for the outer platform layer.
    /// OWNER: IntentHandler (cleared and set each handle() call).
    pub tui_signals: TuiSignals,

    /// Cached copy of user preferences from `nullslop.toml`.
    /// Updated exclusively by `PreferencesStateSyncActor` on `PreferencesUpdated` events.
    /// This is a cache — the file is the authoritative source.
    pub preferences: UserPreferences,

    /// Session picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (session picker navigation).
    pub session_picker: nullslop_selection_widget::TreePickerState<SessionTreeEntry>,

    /// Context strategy picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (strategy picker navigation).
    /// Persona picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (persona picker navigation).
    pub persona_picker: nullslop_selection_widget::SelectionState<PersonaEntry>,

    /// Focus scope stack — single source of truth for what the user is focused on.
    /// OWNER: IntentHandler (push/pop on scope transitions).
    pub scope_stack: ScopeStack,

    /// The current resolved theme (colors for the render pipeline).
    /// OWNER: IntentHandler (theme picker preview), PreferencesStateSyncActor (on prefs change).
    pub theme: Theme,

    /// Theme-sensitive caches. Invalidated when `theme` changes.
    /// OWNER: IntentHandler (cleared on theme change).
    pub caches: FrontendCaches,

    /// Whether the "Press ESC again to cancel" prompt is showing.
    /// OWNER: IntentHandler (set on first ESC in Normal/Sidebar with active stream,
    ///         consumed on second ESC or dismissed on any other key).
    pub cancel_stream_prompt: bool,

    /// Whether the "Press x again to confirm closure" prompt is showing.
    /// OWNER: IntentHandler (set on first SidebarSessionClose, consumed on second
    ///         SidebarSessionClose or dismissed on any other key).
    pub close_session_prompt: bool,

    /// Theme picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (theme picker navigation).
    pub theme_picker: nullslop_selection_widget::SelectionState<crate::feat::theme::ThemeEntry>,

    /// Saved theme before preview — restored on ESC.
    /// OWNER: IntentHandler (set on theme picker open, consumed on confirm/cancel).
    pub theme_preview_original: Option<Theme>,

    /// Path to the themes directory (`~/.config/nullslop/themes/`).
    /// Set once during init from `AppPaths`. Used by the theme picker to discover themes.
    /// OWNER: Init code (set once at startup).
    pub themes_dir: std::path::PathBuf,

    /// Path to the system themes directory (`/usr/share/nullslop/themes/`).
    /// Set once during init from `AppPaths`. Used as fallback for theme discovery.
    /// OWNER: Init code (set once at startup).
    pub system_themes_dir: std::path::PathBuf,

    /// Session lifecycle picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (lifecycle picker navigation).
    pub session_lifecycle_picker: nullslop_selection_widget::SelectionState<SessionLifecycleEntry>,

    /// Workflow picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (workflow picker navigation) + WorkflowActor (entry population).
    pub workflow_picker: nullslop_selection_widget::SelectionState<
        crate::feat::workflow::picker_entry::WorkflowPickerEntry,
    >,

    /// Judge picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (judge picker navigation, confirm creates judge session).
    pub judge_picker:
        nullslop_selection_widget::SelectionState<crate::feat::judge::JudgePickerEntry>,

    /// Compaction model picker state (items, filter text, selection index).
    /// OWNER: IntentHandler (compaction model picker navigation).
    pub compaction_model_picker:
        nullslop_selection_widget::SelectionState<crate::protocol::PickerEntry>,

    /// Arg input popup state — active when `FocusScope::ArgInput` is on the scope stack.
    /// OWNER: IntentHandler (arg input editing, confirmation).
    pub arg_input: ArgInputState,

    /// Token budget input popup state — active when `FocusScope::TokenBudgetInput` is on the scope stack.
    /// OWNER: IntentHandler (budget input editing, confirmation).
    /// Sliding window input popup state — active when `FocusScope::SlidingWindowInput` is on the scope stack.
    /// OWNER: IntentHandler (window size input editing, confirmation).
    /// Rename session input popup state — active when `FocusScope::RenameSessionInput` is on the scope stack.
    /// OWNER: IntentHandler (rename input editing, confirmation).
    pub rename_session_input: RenameSessionInputState,

    /// Sidebar width in columns, synced from preferences.
    /// OWNER: PreferencesStateSyncActor (on PreferencesUpdated).
    pub sidebar_width: u16,

    /// Currently active tab in the main area.
    /// OWNER: IntentHandler (SwitchTab intent).
    pub active_tab: crate::protocol::tab::ActiveTab,

    /// Workflow tab UI state — selection, viewport, inspector, cancel prompt.
    /// OWNER: IntentHandler (all workflow UI interactions).
    pub workflow_ui: WorkflowUiState,
}

impl Default for FrontendState {
    fn default() -> Self {
        let mut scope_stack = ScopeStack::default();
        scope_stack.push(FocusScope::Input);
        Self {
            should_quit: false,
            pins: PinsState::default(),
            sidebar: SidebarState::default(),
            persona_section: PersonaSectionState::default(),
            sessions_section: SessionsSectionState::default(),
            tui_signals: TuiSignals::new(),
            preferences: UserPreferences::default(),
            session_picker: nullslop_selection_widget::TreePickerState::new(),
            persona_picker: nullslop_selection_widget::SelectionState::new(),
            scope_stack,
            theme: crate::feat::theme::default_theme(),
            caches: FrontendCaches::default(),
            cancel_stream_prompt: false,
            close_session_prompt: false,
            theme_picker: nullslop_selection_widget::SelectionState::new(),
            theme_preview_original: None,
            themes_dir: std::path::PathBuf::new(),
            system_themes_dir: std::path::PathBuf::new(),
            session_lifecycle_picker: nullslop_selection_widget::SelectionState::new(),
            workflow_picker: nullslop_selection_widget::SelectionState::new(),
            judge_picker: nullslop_selection_widget::SelectionState::new(),
            compaction_model_picker: nullslop_selection_widget::SelectionState::new(),
            arg_input: ArgInputState::default(),
            rename_session_input: RenameSessionInputState::default(),
            sidebar_width: 30,
            active_tab: crate::protocol::tab::ActiveTab::default(),
            workflow_ui: WorkflowUiState::default(),
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
    /// Workflow execution state — owned by workflow-actor.
    pub workflow: crate::feat::workflow::workflow_state::WorkflowMap,
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
            PickerKind::Session => Some(&mut self.frontend.session_picker),
            PickerKind::Persona => Some(&mut self.frontend.persona_picker),
            PickerKind::Theme => Some(&mut self.frontend.theme_picker),

            PickerKind::SessionLifecycle => Some(&mut self.frontend.session_lifecycle_picker),
            PickerKind::Workflow => Some(&mut self.frontend.workflow_picker),
            PickerKind::Judge => Some(&mut self.frontend.judge_picker),
            PickerKind::CompactionModel => Some(&mut self.frontend.compaction_model_picker),
        }
    }

    /// Read-only access to the active chat session.
    ///
    /// Infallible — `SessionMap` guarantees the active session exists.
    pub fn active_session(&self) -> &ChatSessionState {
        self.session.active_session()
    }

    /// Mutable access to the active chat session.
    ///
    /// Infallible — `SessionMap` guarantees the active session exists.
    pub fn active_session_mut(&mut self) -> &mut ChatSessionState {
        self.session.active_session_mut()
    }

    /// Read-only access to a session by ID.
    ///
    /// # Panics
    ///
    /// Panics if the given session ID does not exist.
    pub fn session(&self, id: &SessionId) -> &ChatSessionState {
        self.session.get_unchecked(id)
    }

    /// Mutable access to a session by ID.
    ///
    /// # Panics
    ///
    /// Panics if the given session ID does not exist.
    pub fn session_mut(&mut self, id: &SessionId) -> &mut ChatSessionState {
        self.session.get_unchecked_mut(id)
    }

    /// Returns mutable access to a session by ID, creating it if missing.
    ///
    /// Used by streaming handlers that receive tokens from actors
    /// which may create new session IDs not yet present in the
    /// sessions map.
    pub fn session_mut_or_create(&mut self, id: &SessionId) -> &mut ChatSessionState {
        self.session.get_or_create(id)
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

    /// Invalidate all theme-sensitive caches. Called when the active theme changes.
    pub fn invalidate_theme_caches(&self) {
        self.frontend.caches.invalidate_all();
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
