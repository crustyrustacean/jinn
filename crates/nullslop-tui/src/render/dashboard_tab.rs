//! Dashboard tab rendering.

use ratatui::Frame;
use ratatui::layout::Rect;

use nullslop_domain::{AppState, AppUiRegistry};

/// Renders the Dashboard tab, which fills the entire content area.
pub(super) fn render_dashboard_tab(
    ui_registry: &mut AppUiRegistry,
    frame: &mut Frame<'_>,
    content: Rect,
    state: &AppState,
    rects: &mut Vec<Rect>,
) {
    if let Some(element) = ui_registry.get_mut("dashboard") {
        element.render(frame, content, state);
        if element.is_selectable() {
            rects.push(content);
        }
    }
}
