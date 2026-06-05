#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test file, panics are acceptable"
)]

use std::sync::Arc;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use jinn_domain::feat::ui::sidebar::Sidebar;
use jinn_domain::{
    ActorHostService, AppCore, AppState, AppUiRegistry, FakeActorHost, Services, State,
};
use ratatui::layout::Rect;

use crate::app::{WhichKeyInstance, scope_for_focus};
use crate::config::TuiConfig;
use crate::keymap;
use crate::msg::Msg;
use crate::scope::Scope;
use crate::selection::{SelectableRects, SelectionState};
use crate::{AppStatus, MsgHandler, TuiApp};

/// Creates a minimal `TuiApp` for testing.
fn test_app() -> TuiApp {
    let services = Services::new();
    let (sender, _receiver) = kanal::unbounded();
    let core = AppCore {
        state: State::new(AppState::default()),
        sender,
    };
    let fake_host = ActorHostService::new(Arc::new(FakeActorHost::new()));
    let mut ui_registry = AppUiRegistry::new();
    jinn_domain::register_all_ui_elements(&mut ui_registry);
    jinn_domain::feat::ui::status_bar::register(&mut ui_registry);
    TuiApp {
        core,
        services,
        actor_host: fake_host,
        ui_registry,
        events: MsgHandler::new(),
        which_key: WhichKeyInstance::new(keymap::init(), Scope::Normal),
        suspend: crate::suspend::Suspend::new(),
        event_thread: None,
        status: AppStatus::Starting,
        selection: SelectionState::Idle,
        selectable_rects: SelectableRects::default(),
        pending_clipboard: false,
        config: TuiConfig::default(),
        sidebar: {
            let mut s = Sidebar::new();
            jinn_domain::feat::ui::sidebar::register_sections(&mut s);
            s
        },
    }
}

#[rstest::rstest]
#[case::normal_chat(jinn_domain::FocusScope::Normal, Scope::Normal)]
#[case::sidebar(jinn_domain::FocusScope::SidebarPersona, Scope::SidebarPersona)]
#[case::input(jinn_domain::FocusScope::Input, Scope::Input)]
#[case::picker_provider(jinn_domain::FocusScope::Picker { kind: jinn_domain::PickerKind::Provider }, Scope::PickerProvider)]
#[case::sidebar_resize(jinn_domain::FocusScope::SidebarResize, Scope::SidebarResize)]
#[case::picker_compaction_model(jinn_domain::FocusScope::Picker { kind: jinn_domain::PickerKind::CompactionModel }, Scope::PickerCompactionModel)]
#[case::picker_task_list(jinn_domain::FocusScope::Picker { kind: jinn_domain::PickerKind::TaskList }, Scope::PickerTaskList)]
fn scope_for_focus_maps_correctly(#[case] focus: jinn_domain::FocusScope, #[case] expected: Scope) {
    // Given a focus scope.
    // When mapping to a keymap scope.
    // Then the expected scope is returned.
    assert_eq!(scope_for_focus(&focus), expected);
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
    let services = Services::new();
    let (sender, _receiver) = kanal::unbounded();
    let core = AppCore {
        state: State::new(AppState::default()),
        sender,
    };
    let fake_host = ActorHostService::new(Arc::new(FakeActorHost::new()));
    let mut ui_registry = AppUiRegistry::new();
    jinn_domain::register_all_ui_elements(&mut ui_registry);
    jinn_domain::feat::ui::status_bar::register(&mut ui_registry);
    let mut app = TuiApp {
        core,
        services,
        actor_host: fake_host,
        ui_registry,
        events: MsgHandler::new(),
        which_key: WhichKeyInstance::new(keymap::init(), Scope::Normal),
        suspend: crate::suspend::Suspend::new(),
        event_thread: None,
        status: AppStatus::Starting,
        selection: SelectionState::Idle,
        selectable_rects: SelectableRects::default(),
        pending_clipboard: false,
        config: TuiConfig::new(false),
        sidebar: {
            let mut s = Sidebar::new();
            jinn_domain::feat::ui::sidebar::register_sections(&mut s);
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

// -----------------------------------------------------------------------------
// Keymap tests for the task-list zoom picker (sidebar `s` binding)
// -----------------------------------------------------------------------------
//
// These tests use a bare `WhichKeyInstance` rather than a full `TuiApp` because
// they only verify that the keymap resolves the right `Intent`. They don't need
// the actor host, sidebar, or selection state.

fn keymap_at(scope: Scope) -> WhichKeyInstance {
    WhichKeyInstance::new(keymap::init(), scope)
}

fn key<'a>(notation: &'a str) -> jinn_domain::KeyEvent {
    jinn_domain::KeyEvent::parse_notation(notation)
        .expect("notation should parse")
}

#[rstest::rstest]
fn s_in_sidebar_task_list_opens_task_list_picker() {
    // Given the keymap rooted at SidebarTaskList.
    let mut wk = keymap_at(Scope::SidebarTaskList);

    // When pressing `s`.
    let intent = wk.handle_key(key("s"));

    // Then it resolves to OpenPicker { kind: TaskList } ("search task list").
    assert_eq!(intent.map(|i| i.to_string()).as_deref(), Some("search task list"));
}


#[rstest::rstest]
#[case::normal(Scope::Normal)]
#[case::input(Scope::Input)]
#[case::sidebar_sessions(Scope::SidebarSessions)]
#[case::picker_session(Scope::PickerSession)]
fn s_outside_sidebar_task_list_does_not_open_task_list_picker(#[case] scope: Scope) {
    // Given the keymap rooted at a non-SidebarTaskList scope.
    let mut wk = keymap_at(scope);

    // When pressing `s`.
    let intent = wk.handle_key(key("s"));

    // Then it does NOT resolve to the TaskList open intent. It may resolve to
    // some other intent (e.g. Input's catch-all `InsertChar('s')`) or None,
    // but never to "search task list".
    assert_ne!(
        intent.map(|i| i.to_string()).as_deref(),
        Some("search task list")
    );
}

#[rstest::rstest]
fn esc_in_picker_task_list_returns_to_normal_mode() {
    // Given the keymap rooted at PickerTaskList.
    let mut wk = keymap_at(Scope::PickerTaskList);

    // When pressing `<esc>`.
    let intent = wk.handle_key(key("escape"));

    // Then it resolves to EnterNormalMode (the existing handler closes the picker
    // and restores the prior SidebarTaskList scope).
    assert_eq!(
        intent.map(|i| i.to_string()).as_deref(),
        Some("enter normal mode")
    );
}


