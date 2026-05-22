//! Vertical border line between main column and sidebar.

use nullslop_domain::FocusScope;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

/// Draws the vertical border line (`│`) between the main column and sidebar.
///
/// The color reflects sidebar focus state and resize mode.
pub(super) fn render_border(
    frame: &mut Frame<'_>,
    border: Rect,
    focus_scope: &FocusScope,
    focus_accent: Color,
    border_unfocused: Color,
    sidebar_resize_accent: Color,
) {
    let border_color = match focus_scope {
        FocusScope::SidebarResize => sidebar_resize_accent,
        FocusScope::SidebarPersona
        | FocusScope::SidebarPins
        | FocusScope::SidebarSessions => focus_accent,
        _ => border_unfocused,
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
    use nullslop_testutil::setup_term;
    use ratatui::layout::Rect;
    use ratatui::style::Color;

    use crate::render::app_layout::AppLayout;

    fn frame_area(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }

    #[rstest::rstest]
    fn separator_is_yellow_when_sidebar_focused() {
        // Given a TuiApp rendered with Sidebar scope.
        let mut app = crate::TuiApp::test_builder().build();
        app.core
            .state
            .write()
            .frontend
            .scope_stack
            .push(nullslop_domain::FocusScope::SidebarPersona);
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
    fn separator_is_darkgray_when_normal() {
        // Given a TuiApp rendered with Normal scope.
        let mut app = crate::TuiApp::test_builder().build();
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
    fn separator_is_green_when_resizing() {
        // Given a TuiApp rendered with SidebarResize scope.
        let mut app = crate::TuiApp::test_builder().build();
        app.core
            .state
            .write()
            .frontend
            .scope_stack
            .push(nullslop_domain::FocusScope::SidebarResize);
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
