//! Session lifecycle picker rendering.

use crate::common::app_state::AppState;
use nullslop_selection_widget::SelectionWidget;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;

/// Renders the session lifecycle picker overlay using [`SelectionWidget`].
///
/// Shows all available lifecycles (including the implicit blank) with
/// descriptions and an args indicator.
pub fn render_session_lifecycle_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let widget = SelectionWidget::new(&state.frontend.session_lifecycle_picker)
        .title(Line::from(" Session Lifecycle "))
        .footer(Line::from(" Enter to select, ESC to cancel "));
    widget.render(frame, area);
}
