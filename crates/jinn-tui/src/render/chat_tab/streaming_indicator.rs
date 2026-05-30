//! Streaming indicator element rendering.

use ratatui::Frame;
use ratatui::layout::Rect;

use jinn_domain::{AppState, AppUiRegistry};

/// Renders the streaming indicator (1 row at bottom of content area).
pub(super) fn render_streaming_indicator(
    ui_registry: &mut AppUiRegistry,
    frame: &mut Frame<'_>,
    indicator_area: Rect,
    state: &AppState,
) {
    if let Some(element) = ui_registry.get_mut("streaming-indicator") {
        element.render(frame, indicator_area, state);
    }
}
