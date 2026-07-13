#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test file, panics are acceptable"
)]

use super::render::*;
use jinn_domain::FocusScope;
use jinn_domain::feat::session::chat_entry::ChatEntry;
use jinn_domain::feat::ui::chat_log::GUTTER_WIDTH;
use jinn_selection_widget::compute_popup_rect;
use jinn_testutil::setup_term;
use ratatui::layout::Rect;
use ratatui::style::Color;

/// Creates a minimal `TuiApp` for render testing.
async fn render_test_app() -> crate::TuiApp {
    crate::TuiApp::test_builder().build().await
}

#[rstest::rstest]
#[tokio::test]
async fn render_registers_content_rect_for_selectable_chat_log() {
    // Given a TuiApp rendered in Chat tab with a 80x24 terminal.

    let mut app = render_test_app().await;
    // Default tab is Chat.

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering.
    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .unwrap();

    // Then the chat area rect is registered as selectable, excluding the gutter.
    // Chat log is selectable - the selectable area starts after the gutter column.
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
#[tokio::test]
async fn picker_popup_rect_is_selectable() {
    // Given a TuiApp rendered with Mode::Picker.

    let mut app = render_test_app().await;
    // Switch to Picker mode with an active provider picker.
    app.core
        .state
        .write_test()
        .frontend
        .scope_stack
        .push(jinn_domain::FocusScope::Picker {
            kind: jinn_domain::PickerKind::Provider,
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
#[tokio::test]
async fn content_area_rect_is_selectable() {
    // Given a TuiApp rendered with Mode::Picker.

    let mut app = render_test_app().await;
    // Switch to Picker mode with an active provider picker.
    app.core
        .state
        .write_test()
        .frontend
        .scope_stack
        .push(jinn_domain::FocusScope::Picker {
            kind: jinn_domain::PickerKind::Provider,
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

/// Helper to find the minimap arrow cell position.
///
/// The arrow renders at the rightmost column of the chat_log_area at the
/// midpoint row (chat_log_height / 2). The chat_log_area is the content area
/// minus 2 bottom lines.
fn arrow_cell_position(layout: &AppLayout) -> (u16, u16) {
    let bottom_lines: u16 = 2;
    let chat_log_height = layout.content.height.saturating_sub(bottom_lines);
    let midpoint = chat_log_height / 2;
    let x = layout.content.x + layout.content.width.saturating_sub(1);
    let y = layout.content.y + midpoint;
    (x, y)
}

#[rstest::rstest]
#[tokio::test]
async fn minimap_arrow_is_yellow_when_normal_scope() {
    // Given a TuiApp rendered with Normal scope and one chat entry.
    let mut app = render_test_app().await;
    app.core
        .state
        .write_test()
        .frontend
        .scope_stack
        .clear_overlays();
    app.core
        .state
        .write_test()
        .active_session_mut()
        .push_entry(ChatEntry::user("hello"));
    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering.
    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .unwrap();

    // Then the minimap arrow is Yellow (focus_accent).
    let layout = AppLayout::new(frame_area(80, 24), 1, 12, 30);
    let (x, y) = arrow_cell_position(&layout);
    let buffer = terminal.backend().buffer();
    let cell = buffer.cell((x, y)).expect("minimap arrow cell");
    assert_eq!(cell.symbol(), ">");
    assert_eq!(cell.fg, Color::Yellow);
}

#[rstest::rstest]
#[tokio::test]
async fn minimap_arrow_is_darkgray_when_input_scope() {
    // Given a TuiApp rendered with Input scope and one chat entry.
    let mut app = render_test_app().await;
    app.core
        .state
        .write_test()
        .frontend
        .scope_stack
        .push(FocusScope::Input);
    app.core
        .state
        .write_test()
        .active_session_mut()
        .push_entry(ChatEntry::user("hello"));
    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering.
    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .unwrap();

    // Then the minimap arrow is DarkGray (border_unfocused).
    let layout = AppLayout::new(frame_area(80, 24), 1, 12, 30);
    let (x, y) = arrow_cell_position(&layout);
    let buffer = terminal.backend().buffer();
    let cell = buffer.cell((x, y)).expect("minimap arrow cell");
    assert_eq!(cell.fg, Color::DarkGray);
}

#[rstest::rstest]
#[tokio::test]
async fn gutter_area_is_not_selectable() {
    // Given a TuiApp rendered in Chat tab with a 80x24 terminal.
    let mut app = render_test_app().await;
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

#[rstest::rstest]
#[tokio::test]
async fn cwd_input_popup_renders_and_is_selectable() {
    // Given a TuiApp rendered with CwdInput scope.
    let mut app = render_test_app().await;
    app.core
        .state
        .write_test()
        .frontend
        .scope_stack
        .push(FocusScope::CwdInput);
    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering.
    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .unwrap();

    // Then the cwd popup rect is registered as selectable.
    let popup_rect = jinn_domain::feat::cwd_input::render::cwd_input_popup_rect(frame_area(80, 24));
    let probe = app
        .selectable_rects
        .find_for_position(popup_rect.x + 1, popup_rect.y + 1);
    assert!(probe.is_some(), "cwd input popup rect should be selectable");
    assert_eq!(probe.unwrap(), popup_rect);
}

/// Column index of the chat-mode vertical border for an 80-wide terminal
/// with the default sidebar width (30). main(48) | minimap(1) | border(1) | sidebar(30).
const CHAT_BORDER_X_80: u16 = 49;

/// Enters the Dashboard tab by swapping the base scope, then renders once,
/// returning the terminal so the test can inspect its buffer.
async fn render_in_dashboard(
    width: u16,
    height: u16,
) -> ratatui::Terminal<ratatui::backend::TestBackend> {
    let mut app = render_test_app().await;
    app.core
        .state
        .write_test()
        .frontend
        .scope_stack
        .swap_base(FocusScope::Dashboard);
    let (mut terminal, _area) = setup_term(width, height);
    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .unwrap();
    terminal
}

#[rstest::rstest]
#[tokio::test]
async fn dashboard_renders_no_vertical_border_or_sidebar_gap() {
    // Given a TuiApp rendered in the Dashboard tab.
    let terminal = render_in_dashboard(80, 24).await;

    // When inspecting the column where the chat layout draws the sidebar border.
    let layout = AppLayout::new(frame_area(80, 24), 1, 12, 30);
    let buffer = terminal.backend().buffer();

    // Then the border glyph (│) is absent at the chat border column for
    // every content row — the dashboard owns the full width.
    for y in layout.content.y..(layout.content.y + layout.content.height) {
        let cell = buffer.cell((CHAT_BORDER_X_80, y)).expect("content cell");
        assert_ne!(
            cell.symbol(),
            "\u{2502}",
            "dashboard must not draw the chat sidebar border at column {CHAT_BORDER_X_80}",
        );
    }
}

#[rstest::rstest]
#[tokio::test]
async fn dashboard_renders_no_status_bar() {
    // Given a TuiApp rendered in the Dashboard tab.
    let terminal = render_in_dashboard(80, 24).await;
    let buffer = terminal.backend().buffer();

    // When scanning every cell for the status bar's signature glyphs.
    let width = 80;
    let height = 24;
    let status_bar_glyphs = ["\u{21BB}", "\u{2191}", "\u{2193}"];
    let mut found: Vec<String> = vec![];
    for y in 0..height {
        for x in 0..width {
            let sym = buffer.cell((x, y)).expect("cell").symbol();
            if status_bar_glyphs.contains(&sym) {
                found.push(format!("({x},{y})={sym}"));
            }
        }
    }

    // Then none of the status bar glyphs appear anywhere on the dashboard.
    assert!(
        found.is_empty(),
        "status bar glyphs found in dashboard: {}",
        found.join(", ")
    );
}

#[rstest::rstest]
#[tokio::test]
async fn dashboard_content_fills_full_width() {
    // Given a TuiApp rendered in the Dashboard tab.
    let terminal = render_in_dashboard(80, 24).await;
    let buffer = terminal.backend().buffer();

    // When reading the rightmost column of the tab-bar row.
    // The tab bar renders "Chat" and "Dashboard" labels; the highlighted
    // "Dashboard" tab must reach the rightmost column (no sidebar reserved).
    let rightmost = buffer.cell((79, 0)).expect("rightmost tab-bar cell");

    // Then the rightmost column is the default background reset cell, confirming
    // the tab bar spans the full width (a status-bar glyph or sidebar content
    // would instead occupy it).
    assert_eq!(
        rightmost.symbol(),
        " ",
        "rightmost column should be reset/blank, not sidebar or border content",
    );
}

#[rstest::rstest]
#[tokio::test]
async fn chat_layout_still_draws_vertical_border_for_sidebar() {
    // Given a TuiApp rendered in the default Chat tab (sidebar width 30).
    let mut app = render_test_app().await;
    let (mut terminal, _area) = setup_term(80, 24);
    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .unwrap();

    // When reading the cell at the chat border column on a content row.
    let layout = AppLayout::new(frame_area(80, 24), 1, 12, 30);
    let buffer = terminal.backend().buffer();
    let cell = buffer
        .cell((layout.border.x, layout.content.y + 1))
        .expect("chat border cell");

    // Then the vertical border glyph (│) is drawn — chat rendering is unchanged.
    assert_eq!(
        cell.symbol(),
        "\u{2502}",
        "chat tab must still render the sidebar border (regression guard)",
    );
}
