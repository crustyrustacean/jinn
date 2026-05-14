//! Queue display element rendering.

use ratatui::Frame;
use ratatui::layout::Rect;

use nullslop_domain::{AppState, AppUiRegistry};

/// Renders the queue display (dynamic rows showing pending messages).
pub(super) fn render_queue_display(
    ui_registry: &mut AppUiRegistry,
    frame: &mut Frame<'_>,
    queue_area: Rect,
    state: &AppState,
) {
    if let Some(element) = ui_registry.get_mut("queue-display") {
        element.render(frame, queue_area, state);
    }
}
