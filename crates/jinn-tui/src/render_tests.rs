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
    activate_render_slices(&mut app);
    app
}

/// Activates the slices whose screen regions the render tests assert on.
///
/// `TuiApp::test_builder` does not run slice activation, so a region
/// whose draw function was never registered renders nothing. That fails
/// loudly here — the expected cell comes back blank — rather than
/// silently, which is why these activations are explicit rather than
/// left to the builder.
pub(crate) fn activate_render_slices(app: &mut crate::TuiApp) {
    let state = app.core.state.clone();
    let services = &mut app.services;
    let mut host = jinn_slices::SliceHost::new(
        &services.slices,
        &mut services.viewport,
        &services.overlay_views,
        &services.key_routes,
        &services.trouper_system,
    );
    // The sidebar owns its sections cell and the previews these tests read.
    jinn_sidebar::activate(&mut host, state);
    // The chat log's own screen regions. Its `activate` cannot run here:
    // the test builder already registers the cells it mints, so a second
    // activation would abort on a taken slot. Registering the draw
    // functions is the part the builder does not do, and it is the part
    // these tests assert on.
    jinn_chat_log_view::render_regions::register(&services.slices);
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

    // Then a preview render was requested, without any key press. With only
    // the cursor able to trigger this, a session that loaded, or a width
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

    // When the worker answers and another frame renders with the cursor still put.
    cache_preview_as_answered(&app);
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

/// Answers the focused session's in-flight preview request with empty lines,
/// standing in for the worker a render test does not run. The signature is the
/// one the trigger itself computes, so this writes the cache entry the next
/// frame will actually look for rather than a stand-in.
fn cache_preview_as_answered(app: &crate::TuiApp) {
    let state = app.core.state.write();
    let id = state.active_session().session_id().clone();
    let width = state
        .frontend
        .with_sections(|s| s.sessions.preview_content_width, || 0);
    let signature = jinn_sidebar::sections::sessions::preview_load::preview_signature(
        state.active_session().history(),
        jinn_chat_log_view_msg::PREVIEW_REQUEST_ENTRY_COUNT,
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

/// Fills the preview cache for the focused session with `text` per entry, so
/// the next frame draws the *ready* popup rather than the loading one.
///
/// The real worker is not running in a render test, so the cache is filled with
/// the lines it would have published. Without this the popup is in its loading
/// state in both frames and the comparison proves nothing about content.
fn fill_preview_cache(app: &crate::TuiApp) {
    let state = app.core.state.write();
    let id = state.active_session().session_id().clone();
    let width = state
        .frontend
        .with_sections(|s| s.sessions.preview_content_width, || 0);
    let signature = jinn_sidebar::sections::sessions::preview_load::preview_signature(
        state.active_session().history(),
        jinn_chat_log_view_msg::PREVIEW_REQUEST_ENTRY_COUNT,
    );
    let theme = state.frontend.theme.clone();
    let ctx = jinn_chat_log_view::chat_log::RenderContext {
        content_width: width,
        is_selected: false,
        is_expanded: false,
        tool_entry_max_lines: 6,
        theme,
        paired_status: None,
        is_streaming: false,
        is_waiting_on_subagent: false,
    };
    let lines = jinn_chat_log_view::kernel_element::render_preview(
        state.active_session().history(),
        &ctx,
        jinn_chat_log_view_msg::PREVIEW_REQUEST_ENTRY_COUNT,
        jinn_chat_log_view_msg::PREVIEW_MAX_LINES,
    );
    let armed = state
        .frontend
        .update_sections(|s| s.sessions.preview.request(id.clone(), signature, width))
        .expect("the sections cell is attached");
    state.frontend.update_sections(|s| {
        s.sessions
            .preview
            .complete(id, armed, signature, width, std::sync::Arc::new(lines));
    });
}

/// The screen rows the session preview popup's own border glyphs sit on.
///
/// Scans for the popup's box-drawing characters rather than recomputing the
/// rect, so this reports what the render pass actually drew.
fn popup_border_rows(buffer: &ratatui::buffer::Buffer, popup: Rect) -> Vec<u16> {
    let x = popup.x;
    let width = popup.width;
    (0..popup.height)
        .map(|i| popup.y + i)
        .filter(|y| {
            (x..x + width).any(|c| {
                buffer
                    .cell((c, *y))
                    .is_some_and(|cell| matches!(cell.symbol(), "┌" | "┐" | "└" | "┘" | "│"))
            })
        })
        .collect()
}

#[rstest::rstest]
#[tokio::test]
async fn the_popup_keeps_its_borders_where_they_are_as_content_grows() {
    // Given a TuiApp with the sessions sidebar focused on a loaded session, in a
    // frame tall enough for the popup at its full, unclamped height.
    let mut app = render_test_app().await;
    {
        let (mut terminal, _area) = setup_term(100, 60);
        terminal.draw(|frame| app.render(frame)).unwrap();
    }
    focus_sessions_on_loaded_session(&app);
    {
        let mut state = app.core.state.write();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("a short message"));
    }
    // A frame to measure the width and request at it.
    {
        let (mut terminal, _area) = setup_term(100, 60);
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    // When the preview is short.
    fill_preview_cache(&app);
    let (mut terminal, _area) = setup_term(100, 60);
    terminal.draw(|frame| app.render(frame)).unwrap();
    let popup =
        jinn_sidebar::sections::sessions::session_preview_popup_rect(frame_area(100, 60), 35);
    let short_rows = popup_border_rows(terminal.backend().buffer(), popup);
    let short_buffer = terminal.backend().buffer().clone();

    // And the same session grows to a long history — the shape a streaming
    // reply takes.
    grow_session_to_long_history(&app);
    fill_preview_cache(&app);
    terminal.draw(|frame| app.render(frame)).unwrap();
    let long_rows = popup_border_rows(terminal.backend().buffer(), popup);
    let long_buffer = terminal.backend().buffer().clone();

    // Then the popup's border rows are identical, so the surface does not
    // resize under the cursor as a session's content changes.
    assert!(
        !short_rows.is_empty(),
        "the session preview popup drew no borders of its own"
    );
    assert_eq!(
        short_rows, long_rows,
        "the popup's border rows moved as the session's content grew"
    );
    // And the two frames really did draw different content, so this is a
    // comparison of two states rather than two identical pictures.
    assert_ne!(
        short_buffer, long_buffer,
        "the two frames rendered identically, so nothing was compared"
    );
}

/// Pushes forty assistant replies onto the active session, growing its
/// history to the length a streaming reply reaches.
fn grow_session_to_long_history(app: &crate::TuiApp) {
    let mut state = app.core.state.write();
    for i in 0..40 {
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant(format!("reply {i}")));
    }
}
