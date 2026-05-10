//! Main application state and per-frame rendering.

use std::mem;

use crossterm::event::{MouseButton, MouseEventKind};
use derive_more::Debug;
use nullslop_actor_host::ActorHostService;
use nullslop_component::AppUiRegistry;
use nullslop_core::{AppCore, AppMsg};
use nullslop_intent::IntentHandler;
use nullslop_protocol::{ActiveTab, Intent, Mode, PickerKind};
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
    /// The pinned context sidebar pane (right side).
    Pinned,
}

/// Type alias for the which-key state parameterized for nullslop.
pub type WhichKeyInstance =
    WhichKeyState<nullslop_protocol::KeyEvent, Scope, Intent, crate::keymap::KeyCategory>;

/// Top-level application state and event loop.
#[derive(Debug)]
pub struct TuiApp {
    /// Application core (state, message channel).
    pub core: AppCore,
    /// Runtime services.
    pub services: nullslop_services::Services,
    /// Actor host for coordinated shutdown.
    pub actor_host: ActorHostService,
    /// Receiver for core lifecycle notifications (shutdown complete).
    pub core_receiver: kanal::Receiver<nullslop_protocol::CoreNotification>,
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
    /// Current application lifecycle status.
    pub status: AppStatus,
    /// Tab manager for rendering the tab bar.
    pub tab_manager: TabManager,
    /// Mouse text selection state.
    pub selection: SelectionState,
    /// Selectable screen regions, rebuilt each frame during rendering.
    pub selectable_rects: SelectableRects,
    /// Set to `true` when a selection is finalized and the selected text
    /// should be copied to the system clipboard during the next render.
    pub pending_clipboard: bool,
    /// TUI configuration (mouse capture, etc.).
    pub config: TuiConfig,
    /// Split manager for the chat tab's pane layout.
    pub split_manager: SplitManager,
    /// Which pane currently has keyboard focus in the Chat tab.
    pub pane_focus: PaneFocus,
    /// Whether the pinned context sidebar pane is visible.
    pub pinned_pane_visible: bool,
    /// Tracked [`AreaId`] for the pinned context sidebar pane (set when opened, cleared when closed).
    pub pinned_pane_id: Option<AreaId>,
}

impl TuiApp {
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
                        let Some(intent) = self.which_key.handle_key(protocol_key) else {
                            return;
                        };
                        self.route_intent(intent);
                    }
                    crossterm::event::Event::Mouse(mouse) => {
                        // Selection handling — intercept before keymap
                        // (only when mouse capture is enabled).
                        if self.config.mouse_selection && self.handle_selection_mouse(mouse) {
                            return; // consumed by selection
                        }
                        // Fall through to keymap for scroll, etc.
                        let scope = *self.which_key.scope();
                        let Some(intent) = self
                            .which_key
                            .keymap()
                            .mouse_handler()
                            .and_then(|h| h(mouse, &scope))
                        else {
                            return;
                        };
                        self.route_intent(intent);
                    }
                    _ => {}
                }
            }
            Msg::Command(cmd) => {
                let _ = self.core.sender().send(AppMsg::Command {
                    command: cmd,
                    source: None,
                });
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

    /// Routes an intent through the [`IntentHandler`] and handles TUI signals.
    ///
    /// 1. Acquires the state write lock and calls [`IntentHandler::handle`].
    /// 2. Collects TUI signals, commands, and mode from the result.
    /// 3. Drops the write lock.
    /// 4. Sends commands to the core channel.
    /// 5. Handles TUI signals (which-key toggle, editor, pinned pane, etc.).
    /// 6. Updates the keymap scope based on the new mode.
    pub fn route_intent(&mut self, intent: Intent) {
        // Step 1–3: Handle intent, collect results, release lock.
        let (commands, signals, mode) = {
            let mut state = self.core.state.write();
            let result = IntentHandler::handle(&intent, &mut state);

            // Populate keymap picker entries if opening the keymap picker.
            if matches!(intent, Intent::OpenPicker { kind: PickerKind::Keymap }) {
                let scope = *self.which_key.scope();
                state.keymap_picker_origin_scope = Some(scope.to_string());
                let intent_entries = if state.keymap_picker_show_all {
                    keymap::collect_all_bindings(self.which_key.keymap())
                } else {
                    keymap::collect_bindings_for_scope(self.which_key.keymap(), &scope)
                };
                // Entries now carry Intent directly — store them in AppState.
                state.keymap_picker.set_items(intent_entries);
                // Also populate all_keymap_entries for scope toggle.
                state.all_keymap_entries = keymap::collect_all_bindings(self.which_key.keymap());
            }

            // Cancel selection when mode changes away from Picker.
            if matches!(intent, Intent::SetMode { .. } | Intent::NormalEscape) {
                self.selection = mem::take(&mut self.selection).cancel();
                if !matches!(state.mode, Mode::Picker) {
                    state.keymap_picker_origin_scope = None;
                }
            }

            // Collect signals and mode before releasing lock.
            let signals = TuiSignalsSnapshot::from_state(&state);
            let mode = state.mode;
            let commands = result.commands;

            (commands, signals, mode)
        };

        // Step 4: Send commands to core channel.
        for cmd in commands {
            let _ = self.core.sender().send(AppMsg::Command {
                command: cmd,
                source: None,
            });
        }

        // Step 5: Handle TUI signals.
        if signals.toggle_whichkey {
            self.which_key.toggle();
        }
        if signals.edit_requested {
            let initial_content = self.core.state.read().active_chat_input().text().to_owned();
            self.suspend.request(SuspendAction::Edit {
                initial_content,
                on_result: Box::new(|result| result),
            });
        }
        if signals.pinned_pane_toggle {
            self.toggle_pinned_pane();
        }
        if signals.pinned_pane_open {
            self.open_pinned_pane();
        }
        if signals.pinned_pane_close {
            self.close_pinned_pane();
        }

        // Step 6: Update scope based on new mode.
        let active_tab = self.core.state.read().active_tab;
        let new_scope = scope_for_mode(mode, active_tab, self.pane_focus);
        self.which_key.set_scope(new_scope);
    }

    /// Renders the application for a single frame.
    pub fn render(&mut self, frame: &mut Frame<'_>) {
        render::render(self, frame);
    }

    /// Opens the pinned context sidebar pane by splitting the chat area vertically.
    ///
    /// # Panics
    ///
    /// Panics if `CHAT_PANE` is not a valid leaf in the split manager.
    #[expect(
        clippy::expect_used,
        reason = "CHAT_PANE invariant maintained by split manager"
    )]
    pub fn open_pinned_pane(&mut self) {
        if self.pinned_pane_visible {
            // Already visible — just ensure focus is set.
            self.pane_focus = PaneFocus::Pinned;
            return;
        }
        // Defensive: if we have a stale tracked ID in the tree, reuse it.
        if let Some(id) = self.pinned_pane_id
            && self.split_manager.contains(id)
        {
            self.pinned_pane_visible = true;
            self.pane_focus = PaneFocus::Pinned;
            return;
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

/// Snapshot of [`nullslop_component::tui_signals::TuiSignals`] fields, copied
/// out of AppState before releasing the write lock.
#[derive(Debug)]
struct TuiSignalsSnapshot {
    toggle_whichkey: bool,
    edit_requested: bool,
    pinned_pane_toggle: bool,
    pinned_pane_open: bool,
    pinned_pane_close: bool,
}

impl TuiSignalsSnapshot {
    fn from_state(state: &nullslop_component::AppState) -> Self {
        Self {
            toggle_whichkey: state.tui_signals.toggle_whichkey,
            edit_requested: state.tui_signals.edit_requested,
            pinned_pane_toggle: state.tui_signals.pinned_pane_toggle,
            pinned_pane_open: state.tui_signals.pinned_pane_open,
            pinned_pane_close: state.tui_signals.pinned_pane_close,
        }
    }
}

/// Returns the scope corresponding to the given mode, active tab, and pane focus.
pub fn scope_for_mode(mode: Mode, active_tab: ActiveTab, pane_focus: PaneFocus) -> Scope {
    match mode {
        Mode::Normal => match active_tab {
            ActiveTab::Dashboard => Scope::Dashboard,
            ActiveTab::Chat => {
                if pane_focus == PaneFocus::Pinned {
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
        let (sender, _receiver) = kanal::unbounded();
        let core = nullslop_core::AppCore {
            state: nullslop_component::State::new(nullslop_component::AppState::default()),
            sender,
        };
        let (_, core_rx) = kanal::unbounded::<nullslop_protocol::CoreNotification>();
        let fake_host = nullslop_actor_host::ActorHostService::new(std::sync::Arc::new(
            nullslop_actor_host::FakeActorHost::new(),
        ));
        let mut ui_registry = AppUiRegistry::new();
        nullslop_component::register_all(&mut ui_registry);
        TuiApp {
            core,
            services,
            actor_host: fake_host,
            core_receiver: core_rx,
            ui_registry,
            events: MsgHandler::new(),
            which_key: WhichKeyInstance::new(keymap::init(), Scope::Normal),
            suspend: Suspend::new(),
            event_task: None,
            status: AppStatus::Starting,
            tab_manager: crate::render::init_tab_manager(),
            selection: SelectionState::Idle,
            selectable_rects: SelectableRects::default(),
            pending_clipboard: false,
            config: TuiConfig::default(),
            split_manager: SplitManager::new(),
            pane_focus: PaneFocus::Chat,
            pinned_pane_visible: false,
            pinned_pane_id: None,
        }
    }

    #[rstest::rstest]
    #[case::normal_chat(Mode::Normal, ActiveTab::Chat, PaneFocus::Chat, Scope::Normal)]
    #[case::normal_dashboard(Mode::Normal, ActiveTab::Dashboard, PaneFocus::Chat, Scope::Dashboard)]
    #[case::pinned(Mode::Normal, ActiveTab::Chat, PaneFocus::Pinned, Scope::Pinned)]
    #[case::input(Mode::Input, ActiveTab::Chat, PaneFocus::Chat, Scope::Input)]
    #[case::picker(Mode::Picker, ActiveTab::Chat, PaneFocus::Chat, Scope::Picker)]
    fn scope_for_mode_maps_correctly(
        #[case] mode: Mode,
        #[case] tab: ActiveTab,
        #[case] focus: PaneFocus,
        #[case] expected: Scope,
    ) {
        // Given a mode, tab, and pane focus.
        // When mapping to a scope.
        // Then the expected scope is returned.
        assert_eq!(scope_for_mode(mode, tab, focus), expected);
    }

    #[rstest::rstest]
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

    #[rstest::rstest]
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

    #[rstest::rstest]
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

    #[rstest::rstest]
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

    #[rstest::rstest]
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

    #[rstest::rstest]
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

    #[rstest::rstest]
    fn mouse_events_not_handled_when_mouse_selection_disabled() {
        // Given an app with mouse selection disabled and a registered selectable rect.
        let services = nullslop_services::Services::new();
        let (sender, _receiver) = kanal::unbounded();
        let core = nullslop_core::AppCore {
            state: nullslop_component::State::new(nullslop_component::AppState::default()),
            sender,
        };
        let (_, core_rx) = kanal::unbounded::<nullslop_protocol::CoreNotification>();
        let fake_host = nullslop_actor_host::ActorHostService::new(std::sync::Arc::new(
            nullslop_actor_host::FakeActorHost::new(),
        ));
        let mut ui_registry = AppUiRegistry::new();
        nullslop_component::register_all(&mut ui_registry);
        let mut app = TuiApp {
            core,
            services,
            actor_host: fake_host,
            core_receiver: core_rx,
            ui_registry,
            events: MsgHandler::new(),
            which_key: WhichKeyInstance::new(keymap::init(), Scope::Normal),
            suspend: Suspend::new(),
            event_task: None,
            status: AppStatus::Starting,
            tab_manager: crate::render::init_tab_manager(),
            selection: SelectionState::Idle,
            selectable_rects: SelectableRects::default(),
            pending_clipboard: false,
            config: crate::config::TuiConfig::new(false),
            split_manager: SplitManager::new(),
            pane_focus: PaneFocus::Chat,
            pinned_pane_visible: false,
            pinned_pane_id: None,
        };
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

    #[rstest::rstest]
    fn toggle_scope_filter_to_show_all_includes_multiple_scopes() {
        // Given an app in Normal scope with keymap picker entries.
        let mut app = test_app();
        app.which_key.set_scope(Scope::Normal);

        app.route_intent(Intent::OpenPicker {
            kind: PickerKind::Keymap,
        });

        // When toggling the scope filter (false -> true).
        app.route_intent(Intent::ToggleKeymapScopeFilter);

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

    #[rstest::rstest]
    fn toggle_scope_filter_back_to_false_limits_to_normal_scope() {
        // Given an app in Normal scope with keymap picker entries.
        let mut app = test_app();
        app.which_key.set_scope(Scope::Normal);

        app.route_intent(Intent::OpenPicker {
            kind: PickerKind::Keymap,
        });

        // When toggling twice (false -> true -> false).
        app.route_intent(Intent::ToggleKeymapScopeFilter);
        app.route_intent(Intent::ToggleKeymapScopeFilter);

        // Then show_all is false and entries are Normal-scope only (the origin scope).
        {
            let state = app.core.state.read();
            assert!(
                !state.keymap_picker_show_all,
                "should be false after second toggle"
            );
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

    #[rstest::rstest]
    fn toggle_keymap_scope_filter_preserves_filter_text() {
        // Given an app with keymap picker open and filter text entered.
        let mut app = test_app();
        app.which_key.set_scope(Scope::Normal);

        // Populate initial entries (stores origin scope).
        app.route_intent(Intent::OpenPicker {
            kind: PickerKind::Keymap,
        });

        // Insert filter text.
        {
            let mut state = app.core.state.write();
            state.keymap_picker.insert_char('q');
        }

        // When toggling the scope filter.
        app.route_intent(Intent::ToggleKeymapScopeFilter);

        // Then the filter text is preserved.
        let state = app.core.state.read();
        assert_eq!(
            state.keymap_picker.filter(),
            "q",
            "filter text should be preserved after toggle"
        );
    }

    // --- Pinned pane tracking tests ---

    #[rstest::rstest]
    fn open_pinned_sets_tracked_id() {
        // Given a fresh app.
        let mut app = test_app();
        assert!(app.pinned_pane_id.is_none());

        // When opening the pinned pane.
        app.open_pinned_pane();

        // Then the tracked ID is set and the pane is visible.
        assert!(app.pinned_pane_id.is_some());
        assert!(app.pinned_pane_visible);
        assert_eq!(app.pane_focus, PaneFocus::Pinned);
    }

    #[rstest::rstest]
    fn open_pinned_adds_split() {
        // Given a fresh app.
        let mut app = test_app();
        assert!(app.pinned_pane_id.is_none());

        // When opening the pinned pane.
        app.open_pinned_pane();

        // Then the split manager contains the tracked ID.
        let id = app.pinned_pane_id.unwrap();
        assert!(app.split_manager.contains(id));
    }

    #[rstest::rstest]
    fn open_pinned_has_two_leaves() {
        // Given a fresh app.
        let mut app = test_app();
        assert!(app.pinned_pane_id.is_none());

        // When opening the pinned pane.
        app.open_pinned_pane();

        // Then there are exactly 2 leaves (chat + pinned).
        assert_eq!(app.split_manager.leaves().len(), 2);
    }

    #[rstest::rstest]
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

    #[rstest::rstest]
    fn close_pinned_clears_tracked_id() {
        // Given an app with the pinned pane open.
        let mut app = test_app();
        app.open_pinned_pane();

        // When closing the pinned pane.
        app.close_pinned_pane();

        // Then the tracked ID is cleared and the pane is hidden.
        assert!(app.pinned_pane_id.is_none());
        assert!(!app.pinned_pane_visible);
        assert_eq!(app.pane_focus, PaneFocus::Chat);
    }

    #[rstest::rstest]
    fn close_pinned_removes_split() {
        // Given an app with the pinned pane open.
        let mut app = test_app();
        app.open_pinned_pane();
        let id = app.pinned_pane_id.unwrap();

        // When closing the pinned pane.
        app.close_pinned_pane();

        // Then the split is removed and only one leaf remains.
        assert!(!app.split_manager.contains(id));
        assert_eq!(app.split_manager.leaves().len(), 1);
    }

    #[rstest::rstest]
    fn reopen_assigns_new_id() {
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
    }

    #[rstest::rstest]
    fn reopen_has_two_leaves() {
        // Given an app where the pinned pane is opened, closed, then reopened.
        let mut app = test_app();
        app.open_pinned_pane();
        app.close_pinned_pane();

        // When reopening.
        app.open_pinned_pane();

        // Then there are exactly 2 leaves (no orphans).
        assert_eq!(app.split_manager.leaves().len(), 2);
    }

    #[rstest::rstest]
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
}

/// Builder for constructing a [`TuiApp`] with sensible defaults for tests.
///
/// All fields default to fake/noop implementations. Override only what the test needs.
///
/// ```ignore
/// let app = TuiApp::test_builder()
///     .services(custom_services)
///     .build();
/// ```
#[derive(Default)]
pub struct TuiAppBuilder {
    services: Option<nullslop_services::Services>,
    state: Option<nullslop_component::AppState>,
}

impl TuiAppBuilder {
    /// Override the default services.
    pub fn services(mut self, services: nullslop_services::Services) -> Self {
        self.services = Some(services);
        self
    }

    /// Override the default app state.
    pub fn state(mut self, state: nullslop_component::AppState) -> Self {
        self.state = Some(state);
        self
    }

    /// Build the `TuiApp` with the configured overrides.
    pub fn build(self) -> TuiApp {
        let services = self.services.unwrap_or_else(nullslop_services::Services::new);
        let state = self.state.unwrap_or_default();

        let (sender, _receiver) = kanal::unbounded();
        let core = AppCore {
            state: nullslop_component::State::new(state),
            sender,
        };
        let (_, core_rx) = kanal::unbounded::<nullslop_protocol::CoreNotification>();
        let fake_host = ActorHostService::new(std::sync::Arc::new(
            nullslop_actor_host::FakeActorHost::new(),
        ));
        let mut ui_registry = AppUiRegistry::new();
        nullslop_component::register_all(&mut ui_registry);

        TuiApp {
            core,
            services,
            actor_host: fake_host,
            core_receiver: core_rx,
            ui_registry,
            events: MsgHandler::new(),
            which_key: WhichKeyInstance::new(crate::keymap::init(), Scope::Normal),
            suspend: Suspend::new(),
            event_task: None,
            status: AppStatus::Starting,
            tab_manager: crate::render::init_tab_manager(),
            selection: SelectionState::Idle,
            selectable_rects: SelectableRects::default(),
            pending_clipboard: false,
            config: TuiConfig::default(),
            split_manager: SplitManager::new(),
            pane_focus: PaneFocus::Chat,
            pinned_pane_visible: false,
            pinned_pane_id: None,
        }
    }
}

impl TuiApp {
    /// Create a test builder with sensible defaults.
    pub fn test_builder() -> TuiAppBuilder {
        TuiAppBuilder::default()
    }
}
