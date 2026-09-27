//! Vertical border line between main column and sidebar.

use jinn_kernel::RenderCtx;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;

/// Draws the vertical border line (`│`) between the main column and sidebar.
///
/// Which accent it draws with is the focused scope's registered render
/// hint, so a slice claims its own accent at activation rather than
/// being special-cased here by name.
pub fn render_border(frame: &mut Frame<'_>, border: Rect, ctx: &RenderCtx) {
    let focus_scope = ctx.state.frontend.scope();
    let theme = &ctx.state.frontend.theme;

    let accent = match focus_scope {
        jinn_slices::FocusScope::Normal => jinn_slices::Accent::Focused,
        jinn_slices::FocusScope::Dynamic(id) => ctx.slices.hint_for(&id).accent,
        _ => jinn_slices::Accent::Unfocused,
    };
    let border_color = match accent {
        jinn_slices::Accent::Focused => theme.focus_accent,
        jinn_slices::Accent::Acting => theme.sidebar_resize_accent,
        jinn_slices::Accent::Unfocused => theme.border_unfocused,
    };
    let border_style = Style::default().fg(border_color);
    for y in border.y..(border.y + border.height) {
        if let Some(cell) = frame.buffer_mut().cell_mut((border.x, y)) {
            cell.set_symbol("\u{2502}");
            cell.set_style(border_style);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        reason = "test code, panics are acceptable"
    )]
    use jinn_testutil::setup_term;
    use ratatui::layout::Rect;
    use ratatui::style::Color;

    use crate::render::app_layout::AppLayout;

    fn frame_area(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn separator_is_yellow_when_sidebar_focused() {
        // Given a TuiApp rendered with Sidebar scope. The accent comes
        // from the scope hint the sidebar registers at activation, so the
        // slice's render wiring has to be live for the accent to be the
        // one a real app draws.
        let mut app = crate::TuiApp::test_builder().build().await;
        crate::render_tests::activate_render_slices(&mut app);
        app.core
            .state
            .write()
            .frontend
            .scope_push(jinn_sidebar_msg::SidebarSectionId::Persona.focus_scope());
        let (mut terminal, _area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                app.render(frame);
            })
            .unwrap();

        // Then the vertical separator is Yellow.
        let layout = AppLayout::new(frame_area(80, 24), 1, 12, 30);
        let buffer = terminal.backend().buffer();
        let cell = buffer
            .cell((layout.border.x, layout.border.y + 5))
            .expect("separator cell");
        assert_eq!(cell.symbol(), "\u{2502}");
        assert_eq!(cell.fg, Color::Yellow);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn separator_is_darkgray_when_normal() {
        // Given a TuiApp rendered with Normal scope.
        let mut app = crate::TuiApp::test_builder().build().await;
        let (mut terminal, _area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                app.render(frame);
            })
            .unwrap();

        // Then the vertical separator is DarkGray.
        let layout = AppLayout::new(frame_area(80, 24), 1, 12, 30);
        let buffer = terminal.backend().buffer();
        let cell = buffer
            .cell((layout.border.x, layout.border.y + 5))
            .expect("separator cell");
        assert_eq!(cell.fg, Color::DarkGray);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn separator_is_green_when_resizing() {
        // Given a TuiApp rendered with the sidebar resize scope.
        let mut app = crate::TuiApp::test_builder().build().await;
        // The accent comes from the scope hint the sidebar registers at
        // activation, so the slice's render wiring has to be live for the
        // border to draw the accent a real app draws.
        crate::render_tests::activate_render_slices(&mut app);
        app.core
            .state
            .write()
            .frontend
            .scope_push(jinn_slices::FocusScope::Dynamic(
                jinn_sidebar_msg::SidebarSectionId::resize_scope_id(),
            ));
        let (mut terminal, _area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                app.render(frame);
            })
            .unwrap();

        // Then the vertical separator is Green (sidebar_resize_accent).
        let layout = AppLayout::new(frame_area(80, 24), 1, 12, 30);
        let buffer = terminal.backend().buffer();
        let cell = buffer
            .cell((layout.border.x, layout.border.y + 5))
            .expect("separator cell");
        assert_eq!(cell.symbol(), "\u{2502}");
        assert_eq!(cell.fg, Color::Green);
    }
}
