//! Main application state and per-frame rendering.

mod builder;
mod signals;

use std::mem;

use crossterm::event::{MouseButton, MouseEventKind};
use derive_more::Debug;
use nullslop_domain::ActorHostService;
use nullslop_domain::AppUiRegistry;
use nullslop_domain::IntentHandler;
use nullslop_domain::feat::ui::sidebar::sidebar::Sidebar;
use nullslop_domain::{ActiveTab, FocusScope, Intent, PickerKind};
use nullslop_domain::{AppCore, AppMsg};
use ratatui::Frame;
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

pub use builder::TuiAppBuilder;

/// Type alias for the which-key state parameterized for nullslop.
pub type WhichKeyInstance =
    WhichKeyState<nullslop_domain::KeyEvent, Scope, Intent, crate::keymap::KeyCategory>;

/// Top-level application state and event loop.
#[derive(Debug)]
pub struct TuiApp {
    /// Application core (state, message channel).
    pub core: AppCore,
    /// Runtime services.
    pub services: nullslop_domain::Services,
    /// Actor host for coordinated shutdown.
    pub actor_host: ActorHostService,
    /// Receiver for core lifecycle notifications (shutdown complete).
    pub core_receiver: kanal::Receiver<nullslop_domain::CoreNotification>,
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
    /// Sidebar container with registered sections.
    pub sidebar: Sidebar,
}

impl TuiApp {
    /// Create a test builder with sensible defaults.
    pub fn test_builder() -> builder::TuiAppBuilder {
        builder::TuiAppBuilder::default()
    }

    /// Processes a single message.
    pub fn handle_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Tick => {
                let load_started = {
                    let state = self.core.state.read();
                    state.session.session_load_started_at
                };
                if let Some(started) = load_started
                    && started.elapsed() >= std::time::Duration::from_secs(10)
                {
                    let mut state = self.core.state.write();
                    state.session.session_loading = false;
                    state.session.session_load_started_at = None;
                    state
                        .active_session_mut()
                        .push_entry(nullslop_domain::ChatEntry::system(
                            "Failed to load session: timed out",
                        ));
                }
                // Lazy cleanup of expired status notifications.
                self.core
                    .state
                    .write()
                    .frontend
                    .clear_expired_notification();
            }
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
    #[expect(
        clippy::needless_pass_by_value,
        reason = "Intent is consumed by intent routing, ownership is semantic"
    )]
    pub fn route_intent(&mut self, intent: Intent) {
        // Step 1–3: Handle intent, collect results, release lock.
        let (commands, signals) = {
            let mut state = self.core.state.write();
            let result = IntentHandler::handle(&intent, &mut state);

            // Populate keymap picker entries if opening the keymap picker.
            if matches!(
                intent,
                Intent::OpenPicker {
                    kind: PickerKind::Keymap
                }
            ) {
                let scope = *self.which_key.scope();
                let intent_entries = if state.frontend.keymap_picker_show_all {
                    keymap::collect_all_bindings(self.which_key.keymap())
                } else {
                    keymap::collect_bindings_for_scope(self.which_key.keymap(), &scope)
                };
                // Entries now carry Intent directly — store them in AppState.
                state.frontend.keymap_picker.set_items(intent_entries);
                // Also populate all_keymap_entries for scope toggle.
                state.frontend.all_keymap_entries =
                    keymap::collect_all_bindings(self.which_key.keymap());
            }

            // Cancel selection when mode changes away from Picker.
            if matches!(intent, Intent::EnterNormalMode | Intent::NormalEscape) {
                self.selection = mem::take(&mut self.selection).cancel();
            }

            // Collect signals before releasing lock.
            let signals = signals::TuiSignalsSnapshot::from_state(&state);
            let commands = result.commands;

            (commands, signals)
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

        // Step 6: Update scope based on new focus.
        let state_read = self.core.state.read();
        let active_tab = state_read.frontend.active_tab;
        let new_scope = scope_for_focus(state_read.frontend.scope_stack.current(), active_tab);
        drop(state_read);
        self.which_key.set_scope(new_scope);
    }

    /// Renders the application for a single frame.
    pub fn render(&mut self, frame: &mut Frame<'_>) {
        render::render(self, frame);
    }
}

/// Returns the keymap scope corresponding to the given focus scope and active tab.
pub fn scope_for_focus(focus: &nullslop_domain::FocusScope, active_tab: ActiveTab) -> Scope {
    match focus {
        FocusScope::Picker { .. } => Scope::Picker,
        FocusScope::Input => Scope::Input,
        FocusScope::Sidebar => Scope::Sidebar,
        FocusScope::Normal => match active_tab {
            ActiveTab::Dashboard => Scope::Dashboard,
            ActiveTab::Chat => Scope::Normal,
        },
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use ratatui::layout::Rect;

    use super::*;

    /// Creates a minimal `TuiApp` for testing.
    fn test_app() -> TuiApp {
        let services = nullslop_domain::Services::new();
        let (sender, _receiver) = kanal::unbounded();
        let core = nullslop_domain::AppCore {
            state: nullslop_domain::State::new(nullslop_domain::AppState::default()),
            sender,
        };
        let (_, core_rx) = kanal::unbounded::<nullslop_domain::CoreNotification>();
        let fake_host = nullslop_domain::ActorHostService::new(std::sync::Arc::new(
            nullslop_domain::FakeActorHost::new(),
        ));
        let mut ui_registry = AppUiRegistry::new();
        nullslop_domain::register_all_ui_elements(&mut ui_registry);
        nullslop_domain::feat::ui::status_bar::register(&mut ui_registry);
        nullslop_domain::feat::ui::char_counter::register(&mut ui_registry);
        nullslop_domain::feat::dashboard::register(&mut ui_registry);
        nullslop_domain::feat::ui::chat_log::register(&mut ui_registry);
        nullslop_domain::feat::provider::register(&mut ui_registry);
        nullslop_domain::feat::chat_input::register(&mut ui_registry);
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
            sidebar: {
                let mut s = Sidebar::new();
                nullslop_domain::feat::ui::sidebar::register_sections(&mut s);
                s
            },
        }
    }

    #[rstest::rstest]
    #[case::normal_chat(nullslop_domain::FocusScope::Normal, ActiveTab::Chat, Scope::Normal)]
    #[case::normal_dashboard(
        nullslop_domain::FocusScope::Normal,
        ActiveTab::Dashboard,
        Scope::Dashboard
    )]
    #[case::sidebar(nullslop_domain::FocusScope::Sidebar, ActiveTab::Chat, Scope::Sidebar)]
    #[case::input(nullslop_domain::FocusScope::Input, ActiveTab::Chat, Scope::Input)]
    #[case::picker(nullslop_domain::FocusScope::Picker { kind: nullslop_domain::PickerKind::Provider }, ActiveTab::Chat, Scope::Picker)]
    fn scope_for_focus_maps_correctly(
        #[case] focus: nullslop_domain::FocusScope,
        #[case] tab: ActiveTab,
        #[case] expected: Scope,
    ) {
        // Given a focus scope and active tab.
        // When mapping to a keymap scope.
        // Then the expected scope is returned.
        assert_eq!(scope_for_focus(&focus, tab), expected);
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
        let services = nullslop_domain::Services::new();
        let (sender, _receiver) = kanal::unbounded();
        let core = nullslop_domain::AppCore {
            state: nullslop_domain::State::new(nullslop_domain::AppState::default()),
            sender,
        };
        let (_, core_rx) = kanal::unbounded::<nullslop_domain::CoreNotification>();
        let fake_host = nullslop_domain::ActorHostService::new(std::sync::Arc::new(
            nullslop_domain::FakeActorHost::new(),
        ));
        let mut ui_registry = AppUiRegistry::new();
        nullslop_domain::register_all_ui_elements(&mut ui_registry);
        nullslop_domain::feat::ui::status_bar::register(&mut ui_registry);
        nullslop_domain::feat::ui::char_counter::register(&mut ui_registry);
        nullslop_domain::feat::dashboard::register(&mut ui_registry);
        nullslop_domain::feat::ui::chat_log::register(&mut ui_registry);
        nullslop_domain::feat::provider::register(&mut ui_registry);
        nullslop_domain::feat::chat_input::register(&mut ui_registry);
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
            sidebar: {
                let mut s = Sidebar::new();
                nullslop_domain::feat::ui::sidebar::register_sections(&mut s);
                s
            },
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
            assert!(
                state.frontend.keymap_picker_show_all,
                "should be true after toggle"
            );
            let all_entries = state.frontend.keymap_picker.items();
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
                !state.frontend.keymap_picker_show_all,
                "should be false after second toggle"
            );
            let scope_entries = state.frontend.keymap_picker.items();
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
            state.frontend.keymap_picker.insert_char('q');
        }

        // When toggling the scope filter.
        app.route_intent(Intent::ToggleKeymapScopeFilter);

        // Then the filter text is preserved.
        let state = app.core.state.read();
        assert_eq!(
            state.frontend.keymap_picker.filter(),
            "q",
            "filter text should be preserved after toggle"
        );
    }
}
