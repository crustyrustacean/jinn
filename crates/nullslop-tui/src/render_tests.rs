#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test file, panics are acceptable"
)]

use super::render::*;
use nullslop_domain::feat::ui::chat_log::GUTTER_WIDTH;
use nullslop_selection_widget::compute_popup_rect;
use nullslop_testutil::setup_term;
use ratatui::layout::Rect;

/// Creates a minimal `TuiApp` for render testing.
fn render_test_app() -> crate::TuiApp {
    crate::TuiApp::test_builder().build()
}

// --- Element-driven selectable rect tests ---

#[rstest::rstest]
fn render_registers_content_rect_for_selectable_chat_log() {
    // Given a TuiApp rendered in Chat tab with a 80x24 terminal.

    let mut app = render_test_app();
    // Default tab is Chat.

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering.
    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .unwrap();

    // Then the chat area rect is registered as selectable, excluding the gutter.
    // Chat log is selectable — the selectable area starts after the gutter column.
    let layout = AppLayout::new(frame_area(80, 24), 1, 12, 30);
    let content = layout.content;
    let expected = Rect {
        x: content.x + GUTTER_WIDTH,
        y: content.y,
        width: content.width.saturating_sub(GUTTER_WIDTH),
        height: content.height,
    };
    let found = app
        .selectable_rects
        .find_for_position(expected.x + 1, expected.y + 1);
    assert!(
        found.is_some(),
        "chat log content rect should be selectable"
    );
    assert_eq!(found.unwrap(), expected);
}

#[rstest::rstest]
fn picker_popup_rect_is_selectable() {
    // Given a TuiApp rendered with Mode::Picker.

    let mut app = render_test_app();
    // Switch to Picker mode with an active provider picker.
    app.core
        .state
        .write()
        .frontend
        .scope_stack
        .push(nullslop_domain::FocusScope::Picker {
            kind: nullslop_domain::PickerKind::Provider,
        });

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering.
    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .unwrap();

    // Then the picker popup rect is registered as selectable.
    let popup_rect = compute_popup_rect(Rect::new(0, 0, 80, 24));
    // Query position inside popup but outside the content area (popup extends
    // further right than the content column which ends at the border).
    let outside_content_x = popup_rect.x + popup_rect.width.saturating_sub(5);
    let found = app.selectable_rects.find_for_position(outside_content_x, 0);
    assert!(found.is_some(), "picker popup rect should be selectable");
    assert_eq!(found.unwrap(), popup_rect);
}

#[rstest::rstest]
fn content_area_rect_is_selectable() {
    // Given a TuiApp rendered with Mode::Picker.

    let mut app = render_test_app();
    // Switch to Picker mode with an active provider picker.
    app.core
        .state
        .write()
        .frontend
        .scope_stack
        .push(nullslop_domain::FocusScope::Picker {
            kind: nullslop_domain::PickerKind::Provider,
        });

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering.
    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .unwrap();

    // Then the content area rect is also still selectable (chat-log is selectable).
    // Query a position inside the gutter-excluded selectable rect.
    let layout = AppLayout::new(frame_area(80, 24), 1, 12, 30);
    let content = layout.content;
    let select_x = content.x + GUTTER_WIDTH + 1;
    let content_found = app
        .selectable_rects
        .find_for_position(select_x, content.y + 1);
    assert!(
        content_found.is_some(),
        "content rect should also be selectable alongside picker"
    );
}

/// Helper to create a Rect matching the terminal dimensions.
fn frame_area(w: u16, h: u16) -> Rect {
    Rect::new(0, 0, w, h)
}

#[rstest::rstest]
fn gutter_area_is_not_selectable() {
    // Given a TuiApp rendered in Chat tab with a 80x24 terminal.
    let mut app = render_test_app();
    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering.
    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .unwrap();

    // Then clicking in the gutter (first column of content area) is not selectable.
    let layout = AppLayout::new(frame_area(80, 24), 1, 12, 30);
    let content = layout.content;
    let found = app
        .selectable_rects
        .find_for_position(content.x, content.y + 1);
    assert!(found.is_none(), "gutter area should not be selectable");
}
