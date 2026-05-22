//! Session picker rendering — renders the tree-structured session picker overlay.

use crate::common::app_state::AppState;
use nullslop_selection_widget::TreePickerWidget;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;

/// Renders the session picker overlay using [`TreePickerWidget`].
///
/// Telescope-style layout: bordered popup with filter input at top,
/// horizontal separator, scrollable session entries in tree order, and a footer.
pub fn render_session_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let footer = Line::styled(
        "CTRL+N to create a new session",
        Style::default().fg(Color::Rgb(255, 165, 0)),
    );
    let widget = TreePickerWidget::new(&state.frontend.session_picker)
        .title(Line::from(" Sessions "))
        .title_style(Style::default().fg(state.frontend.theme.popup_title))
        .tree_prefix_color(state.frontend.theme.muted_text)
        .footer(footer);
    widget.render(frame, area);
}
