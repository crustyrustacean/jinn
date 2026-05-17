//! Session picker rendering — renders the session picker and fork picker overlays.

use crate::common::app_state::AppState;
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

/// Renders the session fork picker overlay using [`SelectionWidget`].
///
/// Shows User and Assistant entries from the active session, color-coded
/// by entry kind. The footer shows active filters and shortcuts.
pub fn render_session_fork_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let mut filter_labels = Vec::new();
    if state.frontend.fork_show_user {
        filter_labels.push("User");
    }
    if state.frontend.fork_show_assistant {
        filter_labels.push("Assistant");
    }
    let filter_text = if filter_labels.is_empty() {
        "No filters active".to_owned()
    } else {
        format!("Messages: {}", filter_labels.join(", "))
    };

    let count = state.frontend.fork_picker.items().len();
    let footer = Line::styled(
        format!(
            "{filter_text} ({count} entries) | CTRL+U user · CTRL+A assistant"
        ),
        Style::default().fg(Color::Rgb(255, 165, 0)),
    );
    let widget = SelectionWidget::new(&state.frontend.fork_picker)
        .title(Line::from(" Fork Session "))
        .footer(footer);
    widget.render(frame, area);
}
