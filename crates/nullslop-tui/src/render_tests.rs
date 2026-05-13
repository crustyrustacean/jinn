use super::*;
use crate::selection::SelectionState;
use nullslop_selection_widget::compute_popup_rect;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

/// Creates a minimal `TuiApp` for render testing.
fn render_test_app() -> crate::TuiApp {
    crate::TuiApp::test_builder().build()
}

/// Creates a test terminal with the given dimensions.
fn setup_term(width: u16, height: u16) -> (Terminal<TestBackend>, Rect) {
    let backend = TestBackend::new(width, height);
    let terminal = Terminal::new(backend).unwrap();
    let area = Rect::new(0, 0, width, height);
    (terminal, area)
}

#[rstest::rstest]
fn app_layout_meets_min_size() {
    // Given a 40x14 area.
    let area = Rect::new(0, 0, 40, 14);

    // When checking meets_min_size.
    let result = AppLayout::meets_min_size(area);

    // Then it returns true.
    assert!(result);
}

#[rstest::rstest]
fn app_layout_too_small() {
    // Given a 10x5 area.
    let area = Rect::new(0, 0, 10, 5);

    // When checking meets_min_size.
    let result = AppLayout::meets_min_size(area);

    // Then it returns false.
    assert!(!result);
}

#[rstest::rstest]
fn init_tab_manager_has_two_tabs() {
    // Given a default tab manager.
    let mgr = init_tab_manager();

    // When checking tab count.
    // Then there are 2 tabs and the first is active.
    assert_eq!(mgr.tab_count(), 2);
    assert!(mgr.active_tab().is_some());
    assert_eq!(mgr.active_tab().unwrap().name, "Chat");
}

#[rstest::rstest]
fn app_layout_includes_indicator_row() {
    // Given a 40x14 area.
    let area = Rect::new(0, 0, 40, 14);
    let layout = AppLayout::new(area, 1, 0, area.height / 2);

    // Then the indicator row has height 1 and is between content and counter.
    assert_eq!(layout.indicator.height, 1);
    assert!(layout.indicator.y > layout.content.y);
    assert!(layout.indicator.y < layout.counter.y);
}

#[rstest::rstest]
fn app_layout_queue_area_has_dynamic_height() {
    // Given a 40x20 area with 3 queued messages.
    let area = Rect::new(0, 0, 40, 20);
    let layout = AppLayout::new(area, 1, 3, area.height / 2);

    // Then the queue area has height 3 and sits between indicator and counter.
    assert_eq!(layout.queue.height, 3);
    assert!(layout.queue.y > layout.indicator.y);
    assert!(layout.queue.y < layout.counter.y);
}

#[rstest::rstest]
fn app_layout_queue_area_zero_height_when_empty() {
    // Given a 40x14 area with no queued messages.
    let area = Rect::new(0, 0, 40, 14);
    let layout = AppLayout::new(area, 1, 0, area.height / 2);

    // Then the queue area has height 0.
    assert_eq!(layout.queue.height, 0);
}

#[rstest::rstest]
fn app_layout_includes_status_bar() {
    // Given a 40x14 area.
    let area = Rect::new(0, 0, 40, 14);
    let layout = AppLayout::new(area, 1, 0, area.height / 2);

    // Then the status bar has height 1 and is at the bottom.
    assert_eq!(layout.status_bar.height, 1);
    assert!(layout.status_bar.y > layout.input.y);
    assert_eq!(layout.status_bar.y + layout.status_bar.height, area.height);
}

// --- Popup sizing tests (exercise compute_popup_rect, not domain-specific) ---

#[rstest::rstest]
fn larger_terminal_gets_taller_popup() {
    // Given two terminal sizes.

    let small_area = Rect::new(0, 0, 80, 24);
    let large_area = Rect::new(0, 0, 80, 42);

    // When computing popup rects.
    let small_popup = compute_popup_rect(small_area);
    let large_popup = compute_popup_rect(large_area);

    // Then the larger terminal gets a taller popup.
    assert!(large_popup.height > small_popup.height);
}

#[rstest::rstest]
fn small_terminal_uses_75_percent_height() {
    // Given two terminal sizes.

    let small_area = Rect::new(0, 0, 80, 24);
    let large_area = Rect::new(0, 0, 80, 42);

    // When computing popup rects.
    let small_popup = compute_popup_rect(small_area);
    let _large_popup = compute_popup_rect(large_area);

    // Then the small terminal popup uses 75% of height + 4 rows of chrome.
    // floor(24 * 0.75) = 18, min(18 + 4, 24) = 22.
    assert_eq!(small_popup.height, 22);
}

// --- Selection highlight tests ---

#[rstest::rstest]
fn cell_inside_selection_is_inverted() {
    // Given a buffer with distinctively colored cells and an active selection.
    let area = Rect::new(0, 0, 20, 10);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    // Paint a cell inside the selection with known colors and non-whitespace symbol.
    buf.cell_mut((3, 3)).unwrap().set_symbol("X");
    buf.cell_mut((3, 3)).unwrap().set_fg(Color::Yellow);
    buf.cell_mut((3, 3)).unwrap().set_bg(Color::Blue);
    // Paint a cell outside the selection with known colors.
    buf.cell_mut((15, 8)).unwrap().set_fg(Color::Red);
    buf.cell_mut((15, 8)).unwrap().set_bg(Color::Green);

    // And an app with an Active selection covering (2,2) to (5,4).
    let mut app = render_test_app();
    app.selection = SelectionState::Active {
        anchor: (2, 2),
        focus: (5, 4),
        bounds: area,
    };

    // When applying selection highlight.
    apply_selection_highlight(&app, &mut buf);

    // Then cell (3, 3) inside the selection has swapped fg/bg.
    let inside = buf.cell((3, 3)).expect("cell inside selection");
    assert_eq!(inside.fg, Color::Blue); // was bg
    assert_eq!(inside.bg, Color::Yellow); // was fg
}

#[rstest::rstest]
fn cell_outside_selection_is_unchanged() {
    // Given a buffer with distinctively colored cells and an active selection.
    let area = Rect::new(0, 0, 20, 10);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    // Paint a cell inside the selection with known colors.
    buf.cell_mut((3, 3)).unwrap().set_fg(Color::Yellow);
    buf.cell_mut((3, 3)).unwrap().set_bg(Color::Blue);
    // Paint a cell outside the selection with known colors.
    buf.cell_mut((15, 8)).unwrap().set_fg(Color::Red);
    buf.cell_mut((15, 8)).unwrap().set_bg(Color::Green);

    // And an app with an Active selection covering (2,2) to (5,4).
    let mut app = render_test_app();
    app.selection = SelectionState::Active {
        anchor: (2, 2),
        focus: (5, 4),
        bounds: area,
    };

    // When applying selection highlight.
    apply_selection_highlight(&app, &mut buf);

    // Then cell (15, 8) outside the selection is unchanged.
    let outside = buf.cell((15, 8)).expect("cell outside selection");
    assert_eq!(outside.fg, Color::Red);
    assert_eq!(outside.bg, Color::Green);
}

#[rstest::rstest]
fn cell_inside_clamped_selection_is_inverted() {
    // Given a buffer covering a large area and a selection where the raw anchor
    // extends beyond the selection's constraining bounds.
    let full_area = Rect::new(0, 0, 30, 30);
    let mut buf = ratatui::buffer::Buffer::empty(full_area);
    // Paint cell inside bounds (will be in clamped selection) with non-whitespace symbol.
    buf.cell_mut((7, 7)).unwrap().set_symbol("X");
    buf.cell_mut((7, 7)).unwrap().set_fg(Color::Cyan);
    buf.cell_mut((7, 7)).unwrap().set_bg(Color::Magenta);
    // Paint cell at raw anchor position (0, 0) — outside bounds.
    buf.cell_mut((0, 0)).unwrap().set_fg(Color::White);
    buf.cell_mut((0, 0)).unwrap().set_bg(Color::Black);

    // And an Active selection with anchor outside bounds.
    // bounds=(5,5,10,10) means valid range is (5,5)-(14,14).
    // anchor=(0,0) is outside bounds, focus=(8,8) is inside.
    // selection_rect() should clamp to (5,5)-(8,8).
    let bounds = Rect::new(5, 5, 10, 10);
    let mut app = render_test_app();
    app.selection = SelectionState::Active {
        anchor: (0, 0),
        focus: (8, 8),
        bounds,
    };

    // When applying selection highlight.
    apply_selection_highlight(&app, &mut buf);

    // Then cell (7, 7) inside the clamped selection is inverted.
    let inside = buf.cell((7, 7)).expect("cell inside clamped selection");
    assert_eq!(inside.fg, Color::Magenta); // was bg
    assert_eq!(inside.bg, Color::Cyan); // was fg
}

#[rstest::rstest]
fn cell_at_raw_anchor_not_inverted() {
    // Given a buffer covering a large area and a selection where the raw anchor
    // extends beyond the selection's constraining bounds.
    let full_area = Rect::new(0, 0, 30, 30);
    let mut buf = ratatui::buffer::Buffer::empty(full_area);
    // Paint cell inside bounds (will be in clamped selection).
    buf.cell_mut((7, 7)).unwrap().set_fg(Color::Cyan);
    buf.cell_mut((7, 7)).unwrap().set_bg(Color::Magenta);
    // Paint cell at raw anchor position (0, 0) — outside bounds.
    buf.cell_mut((0, 0)).unwrap().set_fg(Color::White);
    buf.cell_mut((0, 0)).unwrap().set_bg(Color::Black);

    // And an Active selection with anchor outside bounds.
    // bounds=(5,5,10,10) means valid range is (5,5)-(14,14).
    // anchor=(0,0) is outside bounds, focus=(8,8) is inside.
    // selection_rect() should clamp to (5,5)-(8,8).
    let bounds = Rect::new(5, 5, 10, 10);
    let mut app = render_test_app();
    app.selection = SelectionState::Active {
        anchor: (0, 0),
        focus: (8, 8),
        bounds,
    };

    // When applying selection highlight.
    apply_selection_highlight(&app, &mut buf);

    // Then cell (0, 0) at the raw anchor position is NOT inverted.
    let outside = buf.cell((0, 0)).expect("cell at raw anchor");
    assert_eq!(outside.fg, Color::White); // unchanged
    assert_eq!(outside.bg, Color::Black); // unchanged
}

#[rstest::rstest]
fn selection_highlight_does_nothing_when_idle() {
    // Given a buffer with distinctly colored cells and an Idle selection.
    let area = Rect::new(0, 0, 20, 10);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    buf.cell_mut((5, 5)).unwrap().set_fg(Color::Yellow);
    buf.cell_mut((5, 5)).unwrap().set_bg(Color::Blue);

    // And an app with an Idle selection.
    let mut app = render_test_app();
    app.selection = SelectionState::Idle;

    // When applying selection highlight.
    apply_selection_highlight(&app, &mut buf);

    // Then no cells are inverted — colors remain unchanged.
    let cell = buf.cell((5, 5)).expect("colored cell");
    assert_eq!(cell.fg, Color::Yellow); // unchanged
    assert_eq!(cell.bg, Color::Blue); // unchanged
}

#[rstest::rstest]
fn reset_bg_cell_gets_explicit_colors() {
    // Given a buffer where cells have matching fg and bg (e.g. both Reset,
    // as with user messages rendered with Style::default().bold()).
    let area = Rect::new(0, 0, 20, 10);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    // User-message-style cell: fg = Reset, bg = Reset (bold modifier).
    buf.cell_mut((3, 3))
        .unwrap()
        .set_style(Style::default().add_modifier(Modifier::BOLD));
    buf.cell_mut((3, 3)).unwrap().set_symbol("X");
    // Adjacent cell with distinct colors (assistant-style).
    buf.cell_mut((4, 3)).unwrap().set_symbol("Y");
    buf.cell_mut((4, 3)).unwrap().set_fg(Color::Cyan);

    // And an Active selection covering both cells.
    let mut app = render_test_app();
    app.selection = SelectionState::Active {
        anchor: (2, 2),
        focus: (5, 4),
        bounds: area,
    };

    // When applying selection highlight.
    apply_selection_highlight(&app, &mut buf);

    // Then the Reset/Reset cell gets explicit highlight colors.
    let reset_cell = buf.cell((3, 3)).expect("reset cell");
    assert_eq!(reset_cell.fg, Color::Black);
    assert_eq!(reset_cell.bg, Color::White);
}

#[rstest::rstest]
fn distinct_color_cell_gets_swapped() {
    // Given a buffer where cells have matching fg and bg (e.g. both Reset,
    // as with user messages rendered with Style::default().bold()).
    let area = Rect::new(0, 0, 20, 10);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    // User-message-style cell: fg = Reset, bg = Reset (bold modifier).
    buf.cell_mut((3, 3))
        .unwrap()
        .set_style(Style::default().add_modifier(Modifier::BOLD));
    buf.cell_mut((3, 3)).unwrap().set_symbol("X");
    // Adjacent cell with distinct colors (assistant-style).
    buf.cell_mut((4, 3)).unwrap().set_symbol("Y");
    buf.cell_mut((4, 3)).unwrap().set_fg(Color::Cyan);

    // And an Active selection covering both cells.
    let mut app = render_test_app();
    app.selection = SelectionState::Active {
        anchor: (2, 2),
        focus: (5, 4),
        bounds: area,
    };

    // When applying selection highlight.
    apply_selection_highlight(&app, &mut buf);

    // Then the distinct-colors cell gets swapped fg/bg.
    let cyan_cell = buf.cell((4, 3)).expect("cyan cell");
    assert_eq!(cyan_cell.fg, Color::Reset); // was bg
    assert_eq!(cyan_cell.bg, Color::Cyan); // was fg
}

// --- Clipboard flush tests ---

#[rstest::rstest]
fn clipboard_copy_clears_pending_flag_on_idle_selection() {
    // Given an app with pending_clipboard set but Idle selection.
    let mut app = render_test_app();
    app.selection = SelectionState::Idle;
    app.pending_clipboard = true;

    let area = Rect::new(0, 0, 20, 5);
    let buf = ratatui::buffer::Buffer::empty(area);

    // When flushing the pending clipboard.
    flush_pending_clipboard(&mut app, &buf);

    // Then the pending flag is cleared (even though there was nothing to copy).
    assert!(!app.pending_clipboard);
}

#[rstest::rstest]
fn clipboard_copy_skips_empty_selection() {
    // Given an app with pending_clipboard and an Active selection over empty cells.
    let area = Rect::new(0, 0, 20, 5);
    let buf = ratatui::buffer::Buffer::empty(area);

    let mut app = render_test_app();
    app.selection = SelectionState::Active {
        anchor: (0, 0),
        focus: (3, 0),
        bounds: area,
    };
    app.pending_clipboard = true;

    // When flushing the pending clipboard.
    flush_pending_clipboard(&mut app, &buf);

    // Then the pending flag is cleared.
    assert!(!app.pending_clipboard);
    // And the selection is cleared to Idle (no highlight persists).
    assert_eq!(app.selection, SelectionState::Idle);
    // And no notification is set.
    assert!(
        app.core
            .state
            .read()
            .frontend
            .active_status_notification()
            .is_none()
    );
}

#[rstest::rstest]
fn clipboard_clears_pending_flag_immediately() {
    // Given a buffer with known text and an active selection.
    let area = Rect::new(0, 0, 20, 5);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    // Write "Hello" on row 2.
    for (i, ch) in "Hello".chars().enumerate() {
        buf.cell_mut((2 + i as u16, 2))
            .unwrap()
            .set_symbol(&ch.to_string());
    }

    let mut app = render_test_app();
    app.selection = SelectionState::Active {
        anchor: (2, 2),
        focus: (6, 2),
        bounds: area,
    };
    app.pending_clipboard = true;

    // When flushing the pending clipboard.
    flush_pending_clipboard(&mut app, &buf);

    // Then the pending flag is cleared immediately.
    assert!(!app.pending_clipboard);
    // And the selection is cleared to Idle.
    assert_eq!(app.selection, SelectionState::Idle);
    // And a notification is set.
    assert_eq!(
        app.core.state.read().frontend.active_status_notification(),
        Some("Copied to clipboard")
    );
}

#[rstest::rstest]
#[ignore = "requires clipboard access (run with --ignored)"]
fn clipboard_contains_selected_text() {
    // Given a buffer with known text and an active selection.
    let area = Rect::new(0, 0, 20, 5);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    // Write "Hello" on row 2.
    for (i, ch) in "Hello".chars().enumerate() {
        buf.cell_mut((2 + i as u16, 2))
            .unwrap()
            .set_symbol(&ch.to_string());
    }

    let mut app = render_test_app();
    app.selection = SelectionState::Active {
        anchor: (2, 2),
        focus: (6, 2),
        bounds: area,
    };
    app.pending_clipboard = true;

    // When flushing the pending clipboard.
    flush_pending_clipboard(&mut app, &buf);

    // Then after the clipboard thread completes, the clipboard contains
    // the selected text.
    std::thread::sleep(std::time::Duration::from_millis(500));
    let mut clipboard = arboard::Clipboard::new().expect("clipboard access");
    let content = clipboard.get_text().expect("read clipboard");
    assert_eq!(content, "Hello");
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
    let layout = AppLayout::new(frame_area(80, 24), 1, 0, 12);
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
    app.core.state.write().frontend.mode = nullslop_domain::Mode::Picker;
    app.core.state.write().frontend.active_picker_kind =
        Some(nullslop_domain::PickerKind::Provider);

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
    app.core.state.write().frontend.mode = nullslop_domain::Mode::Picker;
    app.core.state.write().frontend.active_picker_kind =
        Some(nullslop_domain::PickerKind::Provider);

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering.
    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .unwrap();

    // Then the content area rect is also still selectable (chat-log is selectable).
    let layout = AppLayout::new(frame_area(80, 24), 1, 0, 12);
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
