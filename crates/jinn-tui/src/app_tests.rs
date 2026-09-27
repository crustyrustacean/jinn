#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::needless_lifetimes,
    reason = "test file, panics are acceptable"
)]
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::TuiApp;
use crate::app::{WhichKeyInstance, scope_for_focus};
use crate::config::TuiConfig;
use crate::keymap;
use crate::msg::Msg;
use crate::scope::Scope;
use crate::selection::SelectionState;

/// Creates a minimal `TuiApp` for testing.
async fn test_app() -> TuiApp {
    TuiApp::test_builder().build().await
}

#[rstest::rstest]
#[case::normal_chat(jinn_slices::FocusScope::Normal, Scope::Normal)]
#[case::sidebar(jinn_sidebar_msg::SidebarSectionId::Persona.focus_scope(), Scope::Dynamic(jinn_slices::SliceScopeId::navigation("sidebar", "persona")))]
#[case::input(jinn_slices::FocusScope::Input, Scope::Input)]
fn scope_for_focus_maps_correctly(#[case] focus: jinn_slices::FocusScope, #[case] expected: Scope) {
    // Given a focus scope.
    // When mapping to a keymap scope.
    // Then the expected scope is returned.
    assert_eq!(scope_for_focus(&focus), expected);
}

#[rstest::rstest]
#[tokio::test]
async fn mouse_down_left_in_selectable_rect_starts_dragging() {
    // Given an app with a registered selectable rect.
    let mut app = test_app().await;
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
#[tokio::test]
async fn mouse_down_left_outside_selectable_rect_does_not_start_dragging() {
    // Given an app with a registered selectable rect.
    let mut app = test_app().await;
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
#[tokio::test]
async fn mouse_drag_updates_focus_while_dragging() {
    // Given an app with an active drag.
    let mut app = test_app().await;
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
#[tokio::test]
async fn mouse_up_left_finalizes_selection() {
    // Given an app with an active drag.
    let mut app = test_app().await;
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
#[tokio::test]
async fn mouse_down_right_cancels_selection() {
    // Given an app with an active selection.
    let mut app = test_app().await;
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
#[tokio::test]
async fn scroll_events_still_route_to_keymap() {
    // Given an app in Normal scope.
    let mut app = test_app().await;
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
#[tokio::test]
async fn mouse_events_not_handled_when_mouse_selection_disabled() {
    // Given an app with mouse selection disabled and a registered selectable rect.
    let mut app = test_app().await;
    app.config = TuiConfig::new(false);
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

/// A keymap with the chat input box's route rows bound (as launch.rs does).
fn keymap_with_chat_input_rows_at(scope: Scope) -> WhichKeyInstance {
    let mut km = keymap::init();
    let routes = jinn_slices::route::KeyRoutes::new();
    jinn_chat_input::routes::attach_chat_input_rows(&routes);
    crate::keymap_gen::bind_route_rows(&routes, &mut km);
    WhichKeyInstance::new(km, scope)
}

/// A keymap with the chat input box's rows AND key hook bound, as the real
/// composition does — rows for the explicit keys, the hook for characters
/// and cursor motion.
fn keymap_with_chat_input_at(scope: Scope) -> WhichKeyInstance {
    let mut km = keymap::init();
    let routes = jinn_slices::route::KeyRoutes::new();
    jinn_chat_input::routes::attach_all(&routes);
    crate::keymap_gen::bind_route_rows(&routes, &mut km);
    WhichKeyInstance::new(km, scope)
}

/// A keymap with the chat log's route rows bound (as launch.rs does).
fn keymap_with_chat_log_at(scope: Scope) -> WhichKeyInstance {
    let mut km = keymap::init();
    let routes = jinn_slices::route::KeyRoutes::new();
    jinn_chat_log_view::routes::attach_all(&routes);
    crate::keymap_gen::bind_route_rows(&routes, &mut km);
    WhichKeyInstance::new(km, scope)
}

/// A keymap with the sidebar's route rows bound (as launch.rs does).
fn keymap_with_routes_at(scope: Scope) -> WhichKeyInstance {
    let mut km = keymap::init();
    let routes = jinn_slices::route::KeyRoutes::new();
    jinn_sidebar::key_routes::attach_sidebar_rows(&routes);
    crate::keymap_gen::bind_route_rows(&routes, &mut km);
    WhichKeyInstance::new(km, scope)
}

fn key<'a>(notation: &'a str) -> jinn_kernel::KeyEvent {
    jinn_kernel::KeyEvent::parse_notation(notation).expect("notation should parse")
}

#[rstest::rstest]
#[case::normal(Scope::Normal)]
#[case::input(Scope::Input)]
#[case::sidebar_sessions(Scope::Dynamic(jinn_slices::SliceScopeId::navigation(
    "sidebar", "sessions"
)))]
fn s_outside_sidebar_task_list_does_not_open_task_list_picker(#[case] scope: Scope) {
    // Given the keymap rooted at a non-sidebar-task-list scope.
    let mut wk = keymap_at(scope);

    // When pressing `s`.
    let intent = wk.handle_key(key("s"));

    // Then it does NOT resolve to the TaskList open intent. It may resolve to
    // some other intent (e.g. Input's catch-all `InsertChar('s')`) or None,
    // but never to "search task list".
    // The browser is opened only from the sidebar's task-list section. Since
    // the picker is slice-owned it has no kernel intent name, so the check is
    // that `s` did not become a dynamic route into the task-list scope.
    let opened_the_task_list = matches!(
        intent,
        Some(jinn_kernel::KernelIntent::Dynamic(dynamic))
            if dynamic.slice == jinn_tools_msg::task_list_picker_scope()
    );
    assert!(
        !opened_the_task_list,
        "`s` outside the sidebar's task-list section must not open the browser"
    );
}

#[rstest::rstest]
#[test]
fn alt_q_in_input_scope_toggles_input_mode() {
    // Given the keymap (with the box's route rows) rooted at Input scope.
    let mut wk = keymap_with_chat_input_rows_at(Scope::Input);

    // When pressing Alt+q (notation: `m-q`).
    let intent = wk.handle_key(key("m-q"));

    // Then it resolves to ToggleInputMode (Queue ↔ Steer).
    assert_eq!(
        intent.map(|i| i.to_string()).as_deref(),
        Some("toggle input mode")
    );
}

#[rstest::rstest]
#[test]
fn alt_s_in_input_scope_focuses_sidebar_sessions() {
    // Given the keymap (with sidebar route rows) rooted at Input scope.
    let mut wk = keymap_with_routes_at(Scope::Input);

    // When pressing Alt+s (notation: `m-s`).
    let intent = wk.handle_key(key("m-s"));

    // Then it resolves to SidebarFocusSessions (now bound in Input scope too).
    assert_eq!(
        intent.map(|i| i.to_string()).as_deref(),
        Some("focus session list")
    );
}

#[rstest::rstest]
#[test]
fn alt_s_in_normal_scope_focuses_sidebar_sessions() {
    // Given the keymap (with sidebar route rows) rooted at Normal scope.
    let mut wk = keymap_with_routes_at(Scope::Normal);

    // When pressing Alt+s (notation: `m-s`).
    let intent = wk.handle_key(key("m-s"));

    // Then it resolves to SidebarFocusSessions (scope-aware binding).
    assert_eq!(
        intent.map(|i| i.to_string()).as_deref(),
        Some("focus session list")
    );
}

#[rstest::rstest]
#[test]
fn r_in_normal_scope_resets_entry_to_default_context() {
    // Given a keymap with the chat log's route rows bound, at Normal scope.
    let mut wk = keymap_with_chat_log_at(Scope::Normal);

    // When pressing `r`.
    let intent = wk.handle_key(key("r"));

    // Then it resolves to the log's reset action.
    assert_eq!(
        intent.map(|i| i.to_string()).as_deref(),
        Some("reset entry to default context")
    );
}

#[rstest::rstest]
#[test]
fn enter_in_input_scope_submits_through_the_slice_row() {
    // Given the keymap with the box's rows and hook bound, at Input scope.
    let mut wk = keymap_with_chat_input_at(Scope::Input);

    // When pressing Enter.
    let intent = wk.handle_key(key("enter"));

    // Then it resolves to the box's submit action, not a kernel intent.
    assert_eq!(
        intent.map(|i| i.to_string()).as_deref(),
        Some("send this message")
    );
}

#[rstest::rstest]
#[test]
fn printable_char_in_input_scope_reaches_the_slice_insert_action() {
    // Given the keymap with the box's rows and hook bound, at Input scope.
    let mut wk = keymap_with_chat_input_at(Scope::Input);

    // When pressing `a`.
    let intent = wk.handle_key(key("a"));

    // Then it resolves to the box's character insertion, not a kernel
    // `insert-char` intent.
    assert_eq!(
        intent.map(|i| i.to_string()).as_deref(),
        Some("type a character")
    );
}

#[rstest::rstest]
#[test]
fn ctrl_c_in_input_scope_still_reaches_its_kernel_bind() {
    // Given the keymap with the box's hook bound, at Input scope.
    let mut wk = keymap_with_chat_input_at(Scope::Input);

    // When pressing Ctrl+c.
    let intent = wk.handle_key(key("c-c"));

    // Then it resolves to a kernel intent, not one of the box's actions —
    // the box's key hook declines it rather than swallowing it.
    let resolved = intent.map(|i| i.to_string()).expect("ctrl-c is bound");
    assert!(
        !resolved.contains("type a character"),
        "ctrl-c must not be captured by the box, got {resolved}"
    );
}

/// `i` in Normal scope enters the chat input box.
///
/// This is the box's only door from chat history into insert mode, so its
/// absence is invisible until someone tries to type and nothing happens.
/// Asserting the key directly — not "whatever rows got attached resolve" —
/// is deliberate: a test that walks the attached rows cannot see a row that
/// was never attached.
#[rstest::rstest]
#[test]
fn i_in_normal_scope_enters_insert_mode() {
    // Given the keymap with the box's rows and hook bound, at Normal scope.
    let mut wk = keymap_with_chat_input_at(Scope::Normal);

    // When pressing `i`.
    let intent = wk.handle_key(key("i"));

    // Then it resolves to the box's insert-mode action.
    assert_eq!(
        intent.map(|i| i.to_string()).as_deref(),
        Some("type a message")
    );
}

/// `<c-j>` in Normal scope enters the box, the alternate door.
#[rstest::rstest]
#[test]
fn ctrl_j_in_normal_scope_enters_insert_mode() {
    // Given the keymap with the box's rows and hook bound, at Normal scope.
    let mut wk = keymap_with_chat_input_at(Scope::Normal);

    // When pressing Ctrl+j.
    let intent = wk.handle_key(key("c-j"));

    // Then it resolves to the box's insert-mode action.
    assert_eq!(
        intent.map(|i| i.to_string()).as_deref(),
        Some("type a message")
    );
}

// ── The cancel-stream prompt must not outlive a keystroke ──
//
// `IntentHandler` dismisses the prompt for every intent it receives, but a
// key that feeds a which-key sequence never becomes one: `handle_key`
// returns `None` for a branch and for a key matching nothing pending, and
// the event loop returns on that `None`, so the handler never runs. These
// assert the `is_pending` signal the dismissal is keyed on, over the
// keymap the app actually composes.

/// A `TuiApp` with the prompt armed over a turn in flight.
async fn app_with_prompt_armed() -> TuiApp {
    let mut app = test_app().await;
    {
        let mut state = app.core.state.write();
        state.active_session_mut().begin_streaming();
    }
    jinn_kernel::feat::intent::IntentHandler::handle(
        &jinn_kernel::KernelIntent::NormalEscape,
        &mut app.core.state.write(),
        &app.services.slices,
        &app.services.key_routes,
        &app.services.config,
    );
    assert!(
        app.core.state.read().frontend.cancel_stream_prompt,
        "the prompt must be armed before the dismissing key"
    );
    app
}

/// The which-key instance the app composes, at the given scope.
fn app_keymap_at(app: &TuiApp, scope: Scope) -> WhichKeyInstance {
    let mut km = keymap::init();
    crate::keymap_gen::bind_route_rows(&app.services.key_routes, &mut km);
    WhichKeyInstance::new(km, scope)
}

/// Opening a which-key sequence with a bound group prefix dismisses the prompt.
///
/// `g` is a group prefix with real leaves (`gm`, `gc`), so it resolves to a
/// branch: `handle_key` returns `None` and the key never becomes an intent.
#[rstest::rstest]
#[tokio::test]
async fn opening_a_which_key_sequence_dismisses_the_cancel_stream_prompt() {
    // Given the armed prompt and the app's keymap.
    let app = app_with_prompt_armed().await;
    let mut wk = app_keymap_at(&app, Scope::Normal);

    // When `g` is pressed, opening the which-key popup.
    let intent = wk.handle_key(key("g"));

    // Then no intent was minted — the handler never saw this key.
    assert!(
        intent.is_none(),
        "a group prefix must mint no intent, or this test proves nothing"
    );
    // And a sequence is pending, so the dismissal keys on it.
    assert!(
        wk.is_pending(),
        "a group prefix must leave a sequence pending"
    );
    // And the prompt clears for that keystroke.
    app.dismiss_prompt_for_unresolved_key();
    assert!(
        !app.core.state.read().frontend.cancel_stream_prompt,
        "opening a which-key sequence must dismiss the prompt"
    );
}

/// Continuing a which-key sequence dismisses the prompt too.
///
/// A chord is several keystrokes. The user stays in the popup for all of
/// them, so the prompt must clear on the second key as well, not only on
/// the one that opened the popup.
#[rstest::rstest]
#[tokio::test]
async fn continuing_a_which_key_sequence_dismisses_the_cancel_stream_prompt() {
    // Given the armed prompt with a `g` sequence open.
    let app = app_with_prompt_armed().await;
    let mut wk = app_keymap_at(&app, Scope::Normal);
    assert!(wk.handle_key(key("g")).is_none());

    // When the next key deepens the sequence.
    let intent = wk.handle_key(key("c"));

    // Then it still minted no intent, and the sequence is still pending.
    assert!(
        intent.is_none(),
        "a nested group prefix must mint no intent: {intent:?}"
    );
    assert!(wk.is_pending(), "the deeper sequence must stay pending");
    // And the prompt clears for that keystroke as well.
    app.dismiss_prompt_for_unresolved_key();
    assert!(
        !app.core.state.read().frontend.cancel_stream_prompt,
        "every key in a which-key sequence must dismiss the prompt"
    );
}

/// Escape inside a pending sequence dismisses instead of confirming.
///
/// Mid-chord, escape matches nothing pending, so it closes the popup and
/// mints no intent — it can never reach the confirming arm, and must not
/// leave the prompt armed.
#[rstest::rstest]
#[tokio::test]
async fn escape_inside_a_pending_sequence_dismisses_without_confirming() {
    // Given the armed prompt with a `g` sequence open.
    let app = app_with_prompt_armed().await;
    let mut wk = app_keymap_at(&app, Scope::Normal);
    assert!(wk.handle_key(key("g")).is_none());

    // When escape is pressed mid-sequence.
    let intent = wk.handle_key(key("esc"));

    // Then it minted no intent, so it cannot confirm the cancel.
    assert!(
        intent.is_none(),
        "escape mid-sequence must not mint an intent: {intent:?}"
    );
    // And the popup closed, leaving nothing pending.
    assert!(!wk.is_pending(), "escape mid-sequence must close the popup");
    // And the prompt is dismissed rather than left armed by a no-op.
    app.dismiss_prompt_for_unresolved_key();
    assert!(
        !app.core.state.read().frontend.cancel_stream_prompt,
        "a mid-sequence escape must dismiss, not leave the prompt armed"
    );
}

/// Backspace walks a pending sequence back instead of resolving an intent.
#[rstest::rstest]
#[tokio::test]
async fn backspace_inside_a_pending_sequence_dismisses_the_prompt() {
    // Given the armed prompt with a `g c` sequence open.
    let app = app_with_prompt_armed().await;
    let mut wk = app_keymap_at(&app, Scope::Normal);
    assert!(wk.handle_key(key("g")).is_none());
    assert!(wk.handle_key(key("c")).is_none());
    assert!(wk.is_pending());

    // When backspace pops the last key of the sequence.
    let intent = wk.handle_key(key("backspace"));

    // Then no intent is minted, but a sequence is still pending.
    assert!(intent.is_none(), "backspace must mint no intent");
    assert!(
        wk.is_pending(),
        "one pop must leave the rest of the sequence pending"
    );
    // So the dismissal applies to it as well.
    app.dismiss_prompt_for_unresolved_key();
    assert!(
        !app.core.state.read().frontend.cancel_stream_prompt,
        "backspace inside a sequence must dismiss the prompt"
    );
}

/// The leader key resolves through the catch-all, not a pending sequence.
///
/// The app's Normal scope describes `<leader>` as a group but binds no leaf
/// under it, so `navigate` finds no node and the catch-all claims the key.
/// It therefore mints `NoOp` — an intent, which the handler dismisses on its
/// own. Pinned here because the dismissal reads `is_pending`: if the leader
/// ever gains children, this key silently changes which path it takes.
#[rstest::rstest]
#[tokio::test]
async fn the_leader_key_resolves_to_an_intent_rather_than_a_sequence() {
    // Given the app's keymap in Normal scope.
    let app = test_app().await;
    let mut wk = app_keymap_at(&app, Scope::Normal);

    // When the leader is pressed.
    let intent = wk.handle_key(key("space"));

    // Then it mints the catch-all no-op instead of opening a sequence.
    assert_eq!(
        intent.map(|i| i.to_string()).as_deref(),
        Some("no-op"),
        "the leader must mint an intent; the Normal catch-all claims unbound chars"
    );
    // And nothing is left pending.
    assert!(
        !wk.is_pending(),
        "the leader must not leave a sequence pending"
    );
}
