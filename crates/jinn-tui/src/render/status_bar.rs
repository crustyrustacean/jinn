//! Status bar rendering.

use ratatui::Frame;
use ratatui::layout::Rect;

use jinn_domain::{AppState, AppUiRegistry};

/// Renders the status bar at the bottom of the main column.
pub(super) fn render_status_bar(
    ui_registry: &mut AppUiRegistry,
    frame: &mut Frame<'_>,
    status_bar: Rect,
    state: &AppState,
) {
    if let Some(element) = ui_registry.get_mut("status-bar") {
        element.render(frame, status_bar, state);
    }
}
