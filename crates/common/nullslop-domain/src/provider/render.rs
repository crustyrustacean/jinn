//! Provider picker rendering — renders the provider picker overlay.

use nullslop_component::AppState;
use nullslop_selection_widget::SelectionWidget;
use ratatui::Frame;
use ratatui::layout::Rect;

use super::entries;

/// Renders the provider picker overlay using [`SelectionWidget`].
///
/// Telescope-style layout: bordered popup with filter input at top,
/// horizontal separator, scrollable results, and a footer line.
pub fn render_provider_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let footer = entries::format_footer(
        state.provider.last_refreshed_at.as_ref(),
        area.width as usize,
    );
    let widget = SelectionWidget::new(&state.provider.provider_picker)
        .title(ratatui::text::Line::from(" Model "))
        .footer(footer);
    widget.render(frame, area);
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod render_tests;
