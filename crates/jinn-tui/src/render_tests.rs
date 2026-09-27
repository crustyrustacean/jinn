#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test file, panics are acceptable"
)]

use super::render::*;
use jinn_chat_log_view::chat_log::GUTTER_WIDTH;
use jinn_kernel::protocol::ChatEntry;
use jinn_slices::FocusScope;
use jinn_testutil::setup_term;
use ratatui::layout::Rect;
use ratatui::style::Color;

/// Creates a minimal `TuiApp` for render testing.
///
async fn render_test_app() -> crate::TuiApp {
    let services = jinn_kernel::Services::new_fake().await;
    services
        .slices
        .register(
            jinn_mcp_msg::mcp_runtime_slot(),
            jinn_mcp_msg::McpRuntimeState::default(),
        )
        .expect("MCP runtime cell is registered exactly once");
    let mut app = crate::TuiApp::test_builder()
        .services(services)
        .build()
        .await;
    activate_sidebar(&mut app);
    app
}

/// Activates the sidebar slice, which registers the sections cell the preview
/// state lives in. Without it `update_sections` is a no-op and every sidebar
/// assertion silently passes against an empty cell.
fn activate_sidebar(app: &mut crate::TuiApp) {
    let state = app.core.state.clone();
    let services = &mut app.services;
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    jinn_sidebar::activate(&mut host, state);
    host.finalize(&|_scope, _hook| {});
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
        host.finalize(&|_scope, _hook| {});
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

/// Focuses the sessions sidebar on a loaded session with the cursor on it, the
/// state the popup draws in when a user navigates to a session to preview it.
fn focus_sessions_on_loaded_session(app: &crate::TuiApp) {
    let mut state = app.core.state.write();
    let id = state.active_session().session_id().clone();
    if let Some(session) = state.session.get_mut(&id) {
        session.set_session_state(jinn_session_store_msg::SessionState::Loaded);
    }
    state
        .frontend
        .scope_push(jinn_sidebar_msg::SidebarSectionId::Sessions.focus_scope());
    state
        .frontend
        .update_sections(|s| s.sessions.selected_index = Some(0));
    drop(state);
}

#[rstest::rstest]
#[tokio::test]
async fn a_stationary_cursor_gets_its_preview_requested() {
    // Given a TuiApp with the sessions sidebar focused on a loaded session, the
    // cursor on it, and no preview rendered yet. Nothing will move the cursor.
    let mut app = render_test_app().await;
    // A frame first: it is what activates the sidebar and attaches its cell,
    // so setup written before it would be discarded.
    {
        let (mut terminal, _area) = setup_term(80, 24);
        terminal.draw(|frame| app.render(frame)).unwrap();
    }
    focus_sessions_on_loaded_session(&app);
    {
        let mut state = app.core.state.write();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
    }

    // When a frame renders.
    let (mut terminal, _area) = setup_term(80, 24);
    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .unwrap();

    // Then a preview render was requested, without any key press.
    // When only the cursor triggered this, a session that loaded, or a width
    // measured, after the last key left the popup spinning until the user
    // nudged the cursor.
    let in_flight = app
        .core
        .state
        .read()
        .frontend
        .with_sections(|s| s.sessions.preview.in_flight_len(), || 0);
    assert!(
        in_flight > 0,
        "the render pass did not request a preview for the session under a \
         stationary cursor, so the popup would spin until the cursor moved"
    );
}

#[rstest::rstest]
#[tokio::test]
async fn a_cached_preview_is_not_requested_again_every_frame() {
    // Given a TuiApp whose session preview has already been rendered and cached.
    let mut app = render_test_app().await;
    // A frame first: it is what activates the sidebar and attaches its cell,
    // so setup written before it would be discarded.
    {
        let (mut terminal, _area) = setup_term(80, 24);
        terminal.draw(|frame| app.render(frame)).unwrap();
    }
    focus_sessions_on_loaded_session(&app);
    {
        let mut state = app.core.state.write();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
    }
    let (mut terminal, _area) = setup_term(80, 24);
    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .unwrap();

    // When the worker answers and more frames render with the cursor still put.
    {
        let state = app.core.state.write();
        let id = state.active_session().session_id().clone();
        let width = state
            .frontend
            .with_sections(|s| s.sessions.preview_content_width, || 0);
        // The signature the trigger itself computes, so this is the cache entry
        // the next frame will actually look for rather than a stand-in.
        let signature = jinn_sidebar::sections::sessions::preview_load::preview_signature(
            state.active_session().history(),
            jinn_chat_log_view_msg::PREVIEW_ENTRY_COUNT,
        );
        let armed = state
            .frontend
            .update_sections(|s| s.sessions.preview.request(id.clone(), signature, width))
            .expect("the sections cell is attached");
        state.frontend.update_sections(|s| {
            s.sessions.preview.complete(
                id.clone(),
                armed,
                signature,
                width,
                std::sync::Arc::new(Vec::new()),
            );
        });
    }
    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .unwrap();

    // Then no further render is in flight. Requesting per frame would flood the
    // render pool with work that is already done.
    let in_flight = app
        .core
        .state
        .read()
        .frontend
        .with_sections(|s| s.sessions.preview.in_flight_len(), || 0);
    assert_eq!(
        in_flight, 0,
        "the render pass kept re-requesting a preview it already holds"
    );
}
