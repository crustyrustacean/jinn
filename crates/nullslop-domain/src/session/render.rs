//! Session picker rendering — renders the session picker overlay.

use crate::component::AppState;
use nullslop_selection_widget::SelectionWidget;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;

/// Renders the session picker overlay using [`SelectionWidget`].
///
/// Telescope-style layout: bordered popup with filter input at top,
/// horizontal separator, scrollable session entries, and a footer showing
/// the CTRL+N shortcut.
pub fn render_session_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let footer = Line::styled(
        "CTRL+N to create a new session",
        Style::default().fg(Color::Rgb(255, 165, 0)),
    );
    let widget = SelectionWidget::new(&state.frontend.session_picker)
        .title(Line::from(" Sessions "))
        .footer(footer);
    widget.render(frame, area);
}
