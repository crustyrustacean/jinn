#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test file, panics are acceptable"
)]

use super::render::*;
use jinn_chat_log_view::chat_log::GUTTER_WIDTH;
use jinn_domain::protocol::ChatEntry;
use jinn_slices::FocusScope;
use jinn_testutil::setup_term;
use ratatui::layout::Rect;
use ratatui::style::Color;

/// Creates a minimal `TuiApp` for render testing.
///
async fn render_test_app() -> crate::TuiApp {
    let services = jinn_domain::Services::new_fake().await;
    services
        .slices
        .register(
            jinn_mcp_msg::mcp_runtime_slot(),
            jinn_mcp_msg::McpRuntimeState::default(),
        )
        .expect("MCP runtime cell is registered exactly once");
    crate::TuiApp::test_builder()
        .services(services)
        .build()
        .await
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
    app.core.state.write().frontend.scope_clear_overlays();
    app.core
        .state
        .write()
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
        .write()
        .frontend
        .scope_push(FocusScope::Input);
    app.core
        .state
        .write()
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
#[expect(clippy::panic, reason = "test: a failed finalize must fail the test")]
async fn cwd_input_popup_renders_and_is_selectable() {
    // Given a TuiApp rendered with the cwd popup's dynamic scope, the cwd
    // slice activated so its overlay + cell are registered.
    let mut app = render_test_app().await;
    {
        let services = &mut app.services;
        let mut host = jinn_slices::SliceHost::new(
            &services.slices,
            &mut services.viewport,
            &services.overlay_views,
            &services.key_routes,
            &services.trouper_system,
        );
        jinn_cwd::activate(&mut host);
        if let Err(error) = host.finalize(&|_key| None) {
            panic!("cwd slice finalize failed: {error}");
        }
    }
    app.core
        .state
        .write()
        .frontend
        .scope_push(FocusScope::Dynamic(jinn_cwd::cwd_scope()));
    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering.
    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .unwrap();

    // Then the cwd popup rect is registered as selectable.
    let popup_rect = jinn_cwd::cwd_input_popup_rect(frame_area(80, 24));
    let probe = app
        .selectable_rects
        .find_for_position(popup_rect.x + 1, popup_rect.y + 1);
    assert!(probe.is_some(), "cwd input popup rect should be selectable");
    assert_eq!(probe.unwrap(), popup_rect);
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

#[rstest::rstest]
#[tokio::test]
async fn which_key_help_renders_above_the_terminal_overlay() {
    // Given an app with the terminal overlay open in view mode and the
    // which-key help activated (as if `?` had been pressed).
    let mut app = render_test_app().await;
    app.core
        .state
        .write()
        .frontend
        .scope_swap_base(jinn_slices::FocusScope::Dynamic(jinn_term_msg::view_scope()));
    app.which_key.active = true;

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering.
    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .unwrap();

    // Then the help popup's title survives — the overlay did not paint
    // over it.
    let buf = terminal.backend().buffer();
    let rendered: String = buf
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect();
    assert!(
        rendered.contains("Shortcuts"),
        "which-key help must render above the terminal overlay, got: {rendered}"
    );
}
