//! Main application state and per-frame rendering.

use std::mem;

use crossterm::event::{MouseButton, MouseEventKind};
use derive_more::Debug;
use nullslop_component::AppUiRegistry;
use nullslop_core::{AppCore, AppMsg};
use nullslop_protocol::{ActiveTab, Command, Mode};
use nullslop_protocol::context::PinChatEntry;
use nullslop_protocol::chat_input::ChatEntrySelectCancel;
use nullslop_protocol::PinPosition;
use ratatui::Frame;
use ratatui_spatial_splits::{AreaId, SplitManager};
use ratatui_tabs::TabManager;
use ratatui_which_key::{CrosstermKeymapExt as _, WhichKeyState};

use crate::config::TuiConfig;
use crate::keymap;
use crate::msg::Msg;
use crate::render;
use crate::scope::Scope;
use crate::selection::{SelectableRects, SelectionState};
use crate::suspend::{Suspend, SuspendAction};
use crate::{AppStatus, MsgHandler};

/// Well-known area ID for the chat pane in the split layout.
pub(crate) const CHAT_PANE: AreaId = AreaId(1);

/// Which pane currently has keyboard focus in the Chat tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneFocus {
    /// The chat log pane (left side).
    Chat,
    /// The workflow sidebar pane (right side).
    Workflow,
    /// The pinned context sidebar pane (right side).
    Pinned,
}

/// Type alias for the which-key state parameterized for nullslop.
pub type WhichKeyInstance =
    WhichKeyState<nullslop_protocol::KeyEvent, Scope, Command, crate::keymap::KeyCategory>;

/// Top-level application state and event loop.
#[expect(
    clippy::partial_pub_fields,
    reason = "only public fields exposed externally; pub(crate) fields are internal"
)]
#[expect(
    clippy::field_scoped_visibility_modifiers,
    reason = "internal fields use pub(crate) for cross-module access within the crate"
)]
#[derive(Debug)]
pub struct TuiApp {
    /// Application core (bus, state, message channel).
    pub core: AppCore,
    /// UI element registry.
    pub ui_registry: AppUiRegistry,
    /// Message channel for the event loop.
    pub events: MsgHandler,
    /// Which-key keybinding system state.
    #[debug(skip)]
    pub which_key: WhichKeyInstance,
    /// Deferred suspend action queue (e.g., for external editor).
    pub suspend: Suspend,
    /// Background event stream. Set by [`run`](crate::run::run).
    #[debug(skip)]
    pub event_task: Option<tokio::task::JoinHandle<()>>,
    /// Runtime services.
    pub services: nullslop_services::Services,
    /// Current application lifecycle status.
    pub status: AppStatus,
    /// Tab manager for rendering the tab bar.
    pub tab_manager: TabManager,
    /// Mouse text selection state.
    pub(crate) selection: SelectionState,
    /// Selectable screen regions, rebuilt each frame during rendering.
    pub(crate) selectable_rects: SelectableRects,
    /// Set to `true` when a selection is finalized and the selected text
    /// should be copied to the system clipboard during the next render.
    pub(crate) pending_clipboard: bool,
    /// TUI configuration (mouse capture, etc.).
    pub(crate) config: TuiConfig,
    /// Split manager for the chat tab's pane layout.
    pub(crate) split_manager: SplitManager,
    /// Which pane has focus when a workflow is active in the Chat tab.
    pub(crate) pane_focus: PaneFocus,
    /// Whether the workflow sidebar pane is visible.
    pub(crate) workflow_pane_visible: bool,
    /// Whether the pinned context sidebar pane is visible.
    pub(crate) pinned_pane_visible: bool,
    /// Tracked AreaId for the workflow sidebar pane (set when opened, cleared when closed).
    pub(crate) workflow_pane_id: Option<AreaId>,
    /// Tracked AreaId for the pinned context sidebar pane (set when opened, cleared when closed).
    pub(crate) pinned_pane_id: Option<AreaId>,
}

impl TuiApp {
    /// Creates a new application with the given services and default config.
    #[must_use]
    pub fn new(services: nullslop_services::Services) -> Self {
        Self::new_with_config(services, TuiConfig::default())
    }

    /// Creates a new application with the given services and config.
    #[must_use]
    pub fn new_with_config(services: nullslop_services::Services, config: TuiConfig) -> Self {
        let mut core = AppCore::new(services.clone());
        let mut ui_registry = AppUiRegistry::new();
        nullslop_component::register_all(&mut core.bus, &mut ui_registry);
        let keymap = keymap::init();
        let which_key = WhichKeyInstance::new(keymap, Scope::Normal);

        Self {
            core,
            ui_registry,
            events: MsgHandler::new(),
            which_key,
            suspend: Suspend::new(),
            event_task: None,
            services,
            status: AppStatus::Starting,
            tab_manager: crate::render::init_tab_manager(),
            selection: SelectionState::Idle,
            selectable_rects: SelectableRects::default(),
            pending_clipboard: false,
            config,
            split_manager: SplitManager::new(),
            pane_focus: PaneFocus::Chat,
            workflow_pane_visible: false,
            pinned_pane_visible: false,
            workflow_pane_id: None,
            pinned_pane_id: None,
        }
    }

    /// Creates a new application with pre-built core, services, and default config.
    ///
    /// Use this when the caller has already registered components
    /// and set up the actor host on the core.
    #[must_use]
    pub fn new_with_core(
        services: nullslop_services::Services,
        core: nullslop_core::AppCore,
    ) -> Self {
        Self::new_with_core_and_config(services, core, TuiConfig::default())
    }

    /// Creates a new application with pre-built core, services, and config.
    ///
    /// Use this when the caller has already registered components
    /// and set up the actor host on the core.
    #[must_use]
    pub fn new_with_core_and_config(
        services: nullslop_services::Services,
        core: nullslop_core::AppCore,
        config: TuiConfig,
    ) -> Self {
        let mut ui_registry = AppUiRegistry::new();
        nullslop_component::register_tui_elements(&mut ui_registry);
        let keymap = keymap::init();
        let which_key = WhichKeyInstance::new(keymap, Scope::Normal);

        Self {
            core,
            ui_registry,
            events: MsgHandler::new(),
            which_key,
            suspend: Suspend::new(),
            event_task: None,
            services,
            status: AppStatus::Starting,
            tab_manager: crate::render::init_tab_manager(),
            selection: SelectionState::Idle,
            selectable_rects: SelectableRects::default(),
            pending_clipboard: false,
            config,
            split_manager: SplitManager::new(),
            pane_focus: PaneFocus::Chat,
            workflow_pane_visible: false,
            pinned_pane_visible: false,
            workflow_pane_id: None,
            pinned_pane_id: None,
        }
    }

    /// Processes a single message.
    pub fn handle_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Tick => {}
            Msg::Input(event) => {
                match event {
                    crossterm::event::Event::Key(key) => {
                        if key.kind != crossterm::event::KeyEventKind::Press {
                            return;
                        }
                        let Some(protocol_key) = crate::convert::from_crossterm(key) else {
                            tracing::info!(
                                crossterm_code = ?key.code,
                                crossterm_mods = ?key.modifiers,
                                "key converted to None"
                            );
                            return;
                        };
                        tracing::info!(
                            key = ?protocol_key.key,
                            mods = ?protocol_key.modifiers,
                            scope = ?self.which_key.scope(),
                            "key event received"
                        );
                        let Some(cmd) = self.which_key.handle_key(protocol_key) else {
                            return;
                        };
                        self.route_command(cmd);
                    }
                    crossterm::event::Event::Mouse(mouse) => {
                        // Selection handling — intercept before keymap
                        // (only when mouse capture is enabled).
                        if self.config.mouse_selection && self.handle_selection_mouse(mouse) {
                            return; // consumed by selection
                        }
                        // Fall through to keymap for scroll, etc.
                        let scope = *self.which_key.scope();
                        let Some(cmd) = self
                            .which_key
                            .keymap()
                            .mouse_handler()
                            .and_then(|h| h(mouse, &scope))
                        else {
                            return;
                        };
                        self.route_command(cmd);
                    }
                    _ => {}
                }
            }
            Msg::Command(cmd) => {
                self.route_command(cmd);
            }
        }
    }

    /// Handles mouse events for text selection. Returns `true` if the event was consumed.
    fn handle_selection_mouse(&mut self, mouse: crossterm::event::MouseEvent) -> bool {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(bounds) = self
                    .selectable_rects
                    .find_for_position(mouse.column, mouse.row)
                {
                    self.selection = SelectionState::start_drag(mouse.column, mouse.row, bounds);
                    return true;
                }
                false
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.selection.is_active() {
                    self.selection =
                        mem::take(&mut self.selection).update_focus(mouse.column, mouse.row);
                    return true;
                }
                false
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if self.selection.is_active() {
                    self.selection = mem::take(&mut self.selection).finalize();
                    self.pending_clipboard = true;
                    return true;
                }
                false
            }
            MouseEventKind::Down(MouseButton::Right) => {
                if self.selection.is_active() {
                    self.selection = mem::take(&mut self.selection).cancel();
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// Routes a command to the appropriate handler.
    ///
    /// Commands that need `TuiApp`-level state (which-key toggle, editor suspend)
    /// are handled directly. All other commands go through the core channel.
    fn route_command(&mut self, cmd: Command) {
        match cmd {
            Command::ToggleWhichKey => {
                self.which_key.toggle();
            }
            Command::EditInput => {
                let initial_content = self.core.state.read().active_chat_input().text().to_owned();
                self.suspend.request(SuspendAction::Edit {
                    initial_content,
                    on_result: Box::new(|result| result),
                });
            }
            Command::SetMode { payload } => {
                // Cancel any active selection when mode changes.
                // The selectable rects are rebuilt next frame, but the selection's
                // `bounds` may reference a now-invalid rect (e.g. a closed picker popup).
                self.selection = mem::take(&mut self.selection).cancel();
                // Clear keymap picker origin scope when leaving picker mode.
                if payload.mode != Mode::Picker {
                    self.core.state.write().keymap_picker_origin_scope = None;
                }
                let _ = self.core.sender().send(AppMsg::Command {
                    command: Command::SetMode { payload },
                    source: None,
                });
            }
            Command::OpenPicker { ref payload }
                if payload.kind == nullslop_protocol::PickerKind::Keymap =>
            {
                let scope = *self.which_key.scope();
                {
                    let mut state = self.core.state.write();
                    state.keymap_picker_origin_scope = Some(scope.to_string());
                    let entries = if state.keymap_picker_show_all {
                        keymap::collect_all_bindings(self.which_key.keymap())
                    } else {
                        keymap::collect_bindings_for_scope(self.which_key.keymap(), &scope)
                    };
                    state.keymap_picker.set_items(entries);
                    state.keymap_picker.reset();
                }
                let _ = self.core.sender().send(AppMsg::Command {
                    command: cmd,
                    source: None,
                });
            }
            Command::ToggleKeymapScopeFilter => {
                {
                    let mut state = self.core.state.write();
                    state.keymap_picker_show_all = !state.keymap_picker_show_all;
                    let scope: Scope = state
                        .keymap_picker_origin_scope
                        .as_deref()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(*self.which_key.scope());
                    let entries = if state.keymap_picker_show_all {
                        keymap::collect_all_bindings(self.which_key.keymap())
                    } else {
                        keymap::collect_bindings_for_scope(self.which_key.keymap(), &scope)
                    };
                    state.keymap_picker.set_items(entries);
                }
            }
            Command::WorkflowTogglePane => {
                let state = self.core.state.read();
                let incomplete = state.active_session().workflow().is_some_and(|w| {
                    w.active_step.is_some()
                        && w.steps.values().any(|s| {
                            !matches!(
                                s.status,
                                nullslop_workflow::StepStatus::Completed
                                    | nullslop_workflow::StepStatus::AwaitingInput
                            )
                        })
                });
                drop(state);
                self.toggle_workflow_pane(incomplete);
            }
            Command::WorkflowFocusChat => self.focus_chat_pane(),
            Command::WorkflowFocusWorkflow => self.focus_workflow_pane(),
            Command::PinnedPanelToggle => self.toggle_pinned_pane(),
            Command::PinnedPanelOpen => self.open_pinned_pane(),
            Command::PinnedPanelClose => self.close_pinned_pane(),
            Command::ChatEntryPinSelected => {
                let (session_id, entry_id) = {
                    let state = self.core.state.read();
                    match state.active_session().selected_entry_id() {
                        Some(id) => (state.active_session.clone(), id.clone()),
                        None => return,
                    }
                };
                let _ = self.core.sender().send(AppMsg::Command {
                    command: Command::PinChatEntry {
                        payload: PinChatEntry {
                            session_id,
                            entry_id,
                            position: PinPosition::Relative,
                        },
                    },
                    source: None,
                });
            }
            Command::NormalEscape => {
                let session_id = self.core.state.read().active_session.clone();
                let has_selection = self
                    .core
                    .state
                    .read()
                    .active_session()
                    .selected_entry_index()
                    .is_some();
                if has_selection {
                    let _ = self.core.sender().send(AppMsg::Command {
                        command: Command::ChatEntrySelectCancel {
                            payload: ChatEntrySelectCancel { session_id },
                        },
                        source: None,
                    });
                }
                if self.pinned_pane_visible {
                    self.close_pinned_pane();
                }
            }
            _ => {
                let _ = self.core.sender().send(AppMsg::Command {
                    command: cmd,
                    source: None,
                });
            }
        }
    }

    /// Renders the application for a single frame.
    pub fn render(&mut self, frame: &mut Frame<'_>) {
        render::render(self, frame);
    }

    /// Opens the workflow sidebar pane by splitting the chat area vertically.
    pub fn open_workflow_pane(&mut self) {
        if self.workflow_pane_visible {
            return;
        }
        // Pinned and workflow panels are mutually exclusive.
        if self.pinned_pane_visible {
            self.close_pinned_pane();
        }
        // Defensive: if we have a stale tracked ID in the tree, reuse it.
        if let Some(id) = self.workflow_pane_id {
            if self.split_manager.contains(id) {
                self.workflow_pane_visible = true;
                self.pane_focus = PaneFocus::Workflow;
                return;
            }
        }
        let result = self
            .split_manager
            .split_vertical_with_ratio(CHAT_PANE, 0.7)
            .expect("CHAT_PANE should always be a valid leaf");
        self.workflow_pane_id = Some(result.new);
        self.workflow_pane_visible = true;
        self.pane_focus = PaneFocus::Workflow;
    }

    /// Closes the workflow sidebar pane.
    pub fn close_workflow_pane(&mut self) {
        if !self.workflow_pane_visible {
            return;
        }
        if let Some(id) = self.workflow_pane_id {
            self.split_manager.close(id);
            self.workflow_pane_id = None;
        }
        self.workflow_pane_visible = false;
        self.pane_focus = PaneFocus::Chat;
    }

    /// Toggles the workflow sidebar pane. Returns `false` if toggle is blocked
    /// (workflow active and incomplete).
    pub fn toggle_workflow_pane(&mut self, has_active_workflow: bool) -> bool {
        if self.workflow_pane_visible {
            if has_active_workflow {
                return false;
            }
            self.close_workflow_pane();
        } else {
            self.open_workflow_pane();
        }
        true
    }

    /// Focuses the chat pane (left).
    pub fn focus_chat_pane(&mut self) {
        self.pane_focus = PaneFocus::Chat;
    }

    /// Focuses the workflow pane (right). No-op if pane not visible.
    pub fn focus_workflow_pane(&mut self) {
        if self.workflow_pane_visible {
            self.pane_focus = PaneFocus::Workflow;
        }
    }

    /// Opens the pinned context sidebar pane by splitting the chat area vertically.
    pub fn open_pinned_pane(&mut self) {
        // Close workflow pane first if visible (mutually exclusive).
        if self.workflow_pane_visible {
            self.close_workflow_pane();
        }
        if self.pinned_pane_visible {
            // Already visible — just ensure focus is set.
            self.pane_focus = PaneFocus::Pinned;
            return;
        }
        // Defensive: if we have a stale tracked ID in the tree, reuse it.
        if let Some(id) = self.pinned_pane_id {
            if self.split_manager.contains(id) {
                self.pinned_pane_visible = true;
                self.pane_focus = PaneFocus::Pinned;
                return;
            }
        }
        let result = self
            .split_manager
            .split_vertical_with_ratio(CHAT_PANE, 0.7)
            .expect("CHAT_PANE should always be a valid leaf");
        self.pinned_pane_id = Some(result.new);
        self.pinned_pane_visible = true;
        self.pane_focus = PaneFocus::Pinned;
    }

    /// Closes the pinned context sidebar pane.
    pub fn close_pinned_pane(&mut self) {
        if !self.pinned_pane_visible {
            return;
        }
        if let Some(id) = self.pinned_pane_id {
            self.split_manager.close(id);
            self.pinned_pane_id = None;
        }
        self.pinned_pane_visible = false;
        self.pane_focus = PaneFocus::Chat;
    }

    /// Toggles the pinned context sidebar pane.
    pub fn toggle_pinned_pane(&mut self) {
        if self.pinned_pane_visible {
            self.close_pinned_pane();
        } else {
            self.open_pinned_pane();
        }
    }
}

/// Returns the scope corresponding to the given mode, active tab, pane focus, and workflow visibility.
pub fn scope_for_mode(
    mode: Mode,
    active_tab: ActiveTab,
    pane_focus: PaneFocus,
    workflow_visible: bool,
) -> Scope {
    match mode {
        Mode::Normal => match active_tab {
            ActiveTab::Dashboard => Scope::Dashboard,
            ActiveTab::Chat => {
                if workflow_visible && pane_focus == PaneFocus::Workflow {
                    Scope::Workflow
                } else if pane_focus == PaneFocus::Pinned {
                    Scope::Pinned
                } else {
                    Scope::Normal
                }
            }
        },
        Mode::Input => Scope::Input,
        Mode::Picker => Scope::Picker,
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use ratatui::layout::Rect;

    use super::*;

    /// Creates a minimal `TuiApp` for testing.
    fn test_app() -> TuiApp {
        let services = nullslop_services::Services::new();
        TuiApp::new(services)
    }

    #[test]
    fn scope_for_mode_maps_correctly() {
        // Given all Mode variants.
        // When mapping each mode to a scope.
        // Then each mode maps to its corresponding scope.
        assert_eq!(
            scope_for_mode(Mode::Normal, ActiveTab::Chat, PaneFocus::Chat, false),
            Scope::Normal
        );
        assert_eq!(
            scope_for_mode(Mode::Normal, ActiveTab::Dashboard, PaneFocus::Chat, false),
            Scope::Dashboard
        );
        // Workflow scope when pane is visible and focused.
        assert_eq!(
            scope_for_mode(Mode::Normal, ActiveTab::Chat, PaneFocus::Workflow, true),
            Scope::Workflow
        );
        // Workflow focus but pane not visible → Normal scope.
        assert_eq!(
            scope_for_mode(Mode::Normal, ActiveTab::Chat, PaneFocus::Workflow, false),
            Scope::Normal
        );
        // Pane visible but chat focused → Normal scope.
        assert_eq!(
            scope_for_mode(Mode::Normal, ActiveTab::Chat, PaneFocus::Chat, true),
            Scope::Normal
        );
        // Pinned scope when pinned pane is focused.
        assert_eq!(
            scope_for_mode(Mode::Normal, ActiveTab::Chat, PaneFocus::Pinned, false),
            Scope::Pinned
        );
        assert_eq!(
            scope_for_mode(Mode::Input, ActiveTab::Chat, PaneFocus::Chat, false),
            Scope::Input
        );
        assert_eq!(
            scope_for_mode(Mode::Picker, ActiveTab::Chat, PaneFocus::Chat, false),
            Scope::Picker
        );
    }

    #[test]
    fn mouse_down_left_in_selectable_rect_starts_dragging() {
        // Given an app with a registered selectable rect.
        let mut app = test_app();
        let rect = Rect::new(5, 5, 20, 10);
        app.selectable_rects.rebuild(vec![rect]);

        // When sending a left-click inside the rect.
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 8,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_msg(Msg::Input(crossterm::event::Event::Mouse(mouse)));

        // Then the selection is Dragging with anchor at (10, 8).
        assert_eq!(
            app.selection,
            SelectionState::Dragging {
                anchor: (10, 8),
                focus: (10, 8),
                bounds: rect,
            }
        );
    }

    #[test]
    fn mouse_down_left_outside_selectable_rect_does_not_start_dragging() {
        // Given an app with a registered selectable rect.
        let mut app = test_app();
        app.selectable_rects.rebuild(vec![Rect::new(5, 5, 10, 10)]);

        // When sending a left-click outside the rect.
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 30,
            row: 30,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_msg(Msg::Input(crossterm::event::Event::Mouse(mouse)));

        // Then the selection remains Idle.
        assert_eq!(app.selection, SelectionState::Idle);
    }

    #[test]
    fn mouse_drag_updates_focus_while_dragging() {
        // Given an app with an active drag.
        let mut app = test_app();
        let rect = Rect::new(0, 0, 40, 24);
        app.selectable_rects.rebuild(vec![rect]);
        app.selection = SelectionState::start_drag(5, 5, rect);

        // When sending a drag event.
        let mouse = MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 15,
            row: 10,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_msg(Msg::Input(crossterm::event::Event::Mouse(mouse)));

        // Then the focus is updated to (15, 10).
        assert_eq!(
            app.selection,
            SelectionState::Dragging {
                anchor: (5, 5),
                focus: (15, 10),
                bounds: rect,
            }
        );
    }

    #[test]
    fn mouse_up_left_finalizes_selection() {
        // Given an app with an active drag.
        let mut app = test_app();
        let rect = Rect::new(0, 0, 40, 24);
        app.selection = SelectionState::start_drag(2, 3, rect).update_focus(10, 12);

        // When sending a mouse-up event.
        let mouse = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 10,
            row: 12,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_msg(Msg::Input(crossterm::event::Event::Mouse(mouse)));

        // Then the selection is Active with the same anchor and focus.
        assert_eq!(
            app.selection,
            SelectionState::Active {
                anchor: (2, 3),
                focus: (10, 12),
                bounds: rect,
            }
        );
    }

    #[test]
    fn mouse_down_right_cancels_selection() {
        // Given an app with an active selection.
        let mut app = test_app();
        let rect = Rect::new(0, 0, 40, 24);
        app.selection = SelectionState::start_drag(5, 5, rect);

        // When sending a right-click.
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: 5,
            row: 5,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_msg(Msg::Input(crossterm::event::Event::Mouse(mouse)));

        // Then the selection is cancelled to Idle.
        assert_eq!(app.selection, SelectionState::Idle);
    }

    #[test]
    fn scroll_events_still_route_to_keymap() {
        // Given an app in Normal scope.
        let mut app = test_app();
        let initial_selection = app.selection.clone();

        // When sending a scroll-up mouse event.
        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 10,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_msg(Msg::Input(crossterm::event::Event::Mouse(mouse)));

        // Then the selection is unchanged (event fell through to keymap).
        assert_eq!(app.selection, initial_selection);
    }

    #[test]
    fn mouse_events_not_handled_when_mouse_selection_disabled() {
        // Given an app with mouse selection disabled and a registered selectable rect.
        let services = nullslop_services::Services::new();
        let mut app = TuiApp::new_with_config(services, crate::config::TuiConfig::new(false));
        let rect = Rect::new(5, 5, 20, 10);
        app.selectable_rects.rebuild(vec![rect]);

        // When sending a left-click inside the rect.
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 8,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_msg(Msg::Input(crossterm::event::Event::Mouse(mouse)));

        // Then the selection remains Idle (event was not handled).
        assert_eq!(app.selection, SelectionState::Idle);
    }

    // --- Keymap scope toggle tests ---

    #[test]
    fn toggle_scope_filter_to_show_all_includes_multiple_scopes() {
        // Given an app in Normal scope with keymap picker entries.
        let mut app = test_app();
        app.which_key.set_scope(Scope::Normal);

        app.route_command(Command::OpenPicker {
            payload: nullslop_protocol::system::OpenPicker {
                kind: nullslop_protocol::PickerKind::Keymap,
            },
        });

        // When toggling the scope filter (false -> true).
        app.route_command(Command::ToggleKeymapScopeFilter);

        // Then show_all is true and entries include multiple scopes.
        {
            let state = app.core.state.read();
            assert!(state.keymap_picker_show_all, "should be true after toggle");
            let all_entries = state.keymap_picker.items();
            assert!(!all_entries.is_empty(), "should have entries");
            let scopes: std::collections::HashSet<&str> =
                all_entries.iter().map(|e| e.scope.as_str()).collect();
            assert!(
                scopes.len() > 1,
                "all scopes should include multiple scopes, got: {scopes:?}"
            );
        }
    }

    #[test]
    fn toggle_scope_filter_back_to_false_limits_to_normal_scope() {
        // Given an app in Normal scope with keymap picker entries.
        let mut app = test_app();
        app.which_key.set_scope(Scope::Normal);

        app.route_command(Command::OpenPicker {
            payload: nullslop_protocol::system::OpenPicker {
                kind: nullslop_protocol::PickerKind::Keymap,
            },
        });

        // When toggling twice (false -> true -> false).
        app.route_command(Command::ToggleKeymapScopeFilter);
        app.route_command(Command::ToggleKeymapScopeFilter);

        // Then show_all is false and entries are Normal-scope only (the origin scope).
        {
            let state = app.core.state.read();
            assert!(!state.keymap_picker_show_all, "should be false after second toggle");
            let scope_entries = state.keymap_picker.items();
            assert!(!scope_entries.is_empty(), "should have Normal entries");
            for entry in scope_entries {
                assert_eq!(
                    entry.scope, "Normal",
                    "all entries should be Normal scope (origin), got: {}",
                    entry.scope
                );
            }
        }
    }

    #[test]
    fn toggle_keymap_scope_filter_preserves_filter_text() {
        // Given an app with keymap picker open and filter text entered.
        let mut app = test_app();
        app.which_key.set_scope(Scope::Normal);

        // Populate initial entries (stores origin scope).
        app.route_command(Command::OpenPicker {
            payload: nullslop_protocol::system::OpenPicker {
                kind: nullslop_protocol::PickerKind::Keymap,
            },
        });

        // Insert filter text.
        {
            let mut state = app.core.state.write();
            state.keymap_picker.insert_char('q');
        }

        // When toggling the scope filter.
        app.route_command(Command::ToggleKeymapScopeFilter);

        // Then the filter text is preserved.
        let state = app.core.state.read();
        assert_eq!(
            state.keymap_picker.filter(),
            "q",
            "filter text should be preserved after toggle"
        );
    }

    // --- Pinned pane tracking tests ---

    #[test]
    fn open_pinned_pane_creates_split_and_tracks_id() {
        // Given a fresh app.
        let mut app = test_app();
        assert!(app.pinned_pane_id.is_none());

        // When opening the pinned pane.
        app.open_pinned_pane();

        // Then the tracked ID is set and the pane is visible.
        assert!(app.pinned_pane_id.is_some());
        assert!(app.pinned_pane_visible);
        assert_eq!(app.pane_focus, PaneFocus::Pinned);
        // And the split manager contains the tracked ID.
        let id = app.pinned_pane_id.unwrap();
        assert!(app.split_manager.contains(id));
        // And there are exactly 2 leaves (chat + pinned).
        assert_eq!(app.split_manager.leaves().len(), 2);
    }

    #[test]
    fn opening_pinned_pane_twice_is_idempotent() {
        // Given an app with the pinned pane already open.
        let mut app = test_app();
        app.open_pinned_pane();
        let first_id = app.pinned_pane_id;

        // When opening it again.
        app.open_pinned_pane();

        // Then the tracked ID is unchanged and no extra split is created.
        assert_eq!(app.pinned_pane_id, first_id);
        assert_eq!(app.split_manager.leaves().len(), 2);
    }

    #[test]
    fn close_pinned_pane_removes_split() {
        // Given an app with the pinned pane open.
        let mut app = test_app();
        app.open_pinned_pane();
        let id = app.pinned_pane_id.unwrap();

        // When closing the pinned pane.
        app.close_pinned_pane();

        // Then the tracked ID is cleared and the split is removed.
        assert!(app.pinned_pane_id.is_none());
        assert!(!app.pinned_pane_visible);
        assert!(!app.split_manager.contains(id));
        assert_eq!(app.split_manager.leaves().len(), 1);
        assert_eq!(app.pane_focus, PaneFocus::Chat);
    }

    #[test]
    fn close_and_reopen_pinned_pane_works_cleanly() {
        // Given an app where the pinned pane is opened, closed, then reopened.
        let mut app = test_app();
        app.open_pinned_pane();
        let first_id = app.pinned_pane_id.unwrap();
        app.close_pinned_pane();

        // When reopening.
        app.open_pinned_pane();

        // Then a new tracked ID is assigned.
        let second_id = app.pinned_pane_id.unwrap();
        assert_ne!(second_id, first_id);
        assert!(app.split_manager.contains(second_id));
        // And there are exactly 2 leaves (no orphans).
        assert_eq!(app.split_manager.leaves().len(), 2);
    }

    #[test]
    fn open_close_reopen_pinned_pane_many_times_no_orphans() {
        // Given an app.
        let mut app = test_app();

        // When opening and closing the pinned pane 5 times.
        for _ in 0..5 {
            app.open_pinned_pane();
            assert_eq!(app.split_manager.leaves().len(), 2);
            app.close_pinned_pane();
            assert_eq!(app.split_manager.leaves().len(), 1);
        }

        // Then there is still exactly 1 leaf (the chat pane).
        assert_eq!(app.split_manager.leaves().len(), 1);
    }

    // --- Workflow pane tracking tests ---

    #[test]
    fn open_workflow_pane_creates_split_and_tracks_id() {
        // Given a fresh app.
        let mut app = test_app();
        assert!(app.workflow_pane_id.is_none());

        // When opening the workflow pane.
        app.open_workflow_pane();

        // Then the tracked ID is set and the pane is visible.
        assert!(app.workflow_pane_id.is_some());
        assert!(app.workflow_pane_visible);
        assert_eq!(app.pane_focus, PaneFocus::Workflow);
        assert_eq!(app.split_manager.leaves().len(), 2);
    }

    #[test]
    fn opening_workflow_pane_twice_is_idempotent() {
        // Given an app with the workflow pane already open.
        let mut app = test_app();
        app.open_workflow_pane();
        let first_id = app.workflow_pane_id;

        // When opening it again.
        app.open_workflow_pane();

        // Then the tracked ID is unchanged.
        assert_eq!(app.workflow_pane_id, first_id);
        assert_eq!(app.split_manager.leaves().len(), 2);
    }

    #[test]
    fn close_and_reopen_workflow_pane_works_cleanly() {
        // Given an app where the workflow pane is opened, closed, then reopened.
        let mut app = test_app();
        app.open_workflow_pane();
        let first_id = app.workflow_pane_id.unwrap();
        app.close_workflow_pane();

        // When reopening.
        app.open_workflow_pane();

        // Then a new tracked ID is assigned and there are exactly 2 leaves.
        let second_id = app.workflow_pane_id.unwrap();
        assert_ne!(second_id, first_id);
        assert_eq!(app.split_manager.leaves().len(), 2);
    }

    #[test]
    fn pinned_and_workflow_panes_are_mutually_exclusive() {
        // Given an app with the workflow pane open.
        let mut app = test_app();
        app.open_workflow_pane();
        assert!(app.workflow_pane_visible);

        // When opening the pinned pane.
        app.open_pinned_pane();

        // Then the workflow pane is closed and the pinned pane is open.
        assert!(!app.workflow_pane_visible);
        assert!(app.workflow_pane_id.is_none());
        assert!(app.pinned_pane_visible);
        assert!(app.pinned_pane_id.is_some());
        // And there are exactly 2 leaves (chat + pinned, no orphan workflow).
        assert_eq!(app.split_manager.leaves().len(), 2);
    }
}
