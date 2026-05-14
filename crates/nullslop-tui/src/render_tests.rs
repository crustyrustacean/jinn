use super::render::*;
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

    // Then the chat area rect is registered as selectable.
    // Chat log is selectable — content area is the main column's sub-area.
    let layout = AppLayout::new(frame_area(80, 24), 1, 12);
    let chat_area = layout.content;
    let found = app
        .selectable_rects
        .find_for_position(chat_area.x + 1, chat_area.y + 1);
    assert!(
        found.is_some(),
        "chat log content rect should be selectable"
    );
    assert_eq!(found.unwrap(), chat_area);
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
    // Query position (popup.x + 1, 0) — inside popup, but above the content area (y=1)
    // so the smallest matching rect is the picker popup, not the content.
    let found = app.selectable_rects.find_for_position(popup_rect.x + 1, 0);
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
    let layout = AppLayout::new(frame_area(80, 24), 1, 12);
    let content_found = app
        .selectable_rects
        .find_for_position(layout.content.x + 1, layout.content.y + 1);
    assert!(
        content_found.is_some(),
        "content rect should also be selectable alongside picker"
    );
}

/// Helper to create a Rect matching the terminal dimensions.
fn frame_area(w: u16, h: u16) -> Rect {
    Rect::new(0, 0, w, h)
}
