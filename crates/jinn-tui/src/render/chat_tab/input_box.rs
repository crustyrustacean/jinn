//! Input box element rendering.

use ratatui::Frame;
use ratatui::layout::Rect;

use jinn_domain::{AppState, AppUiRegistry};

/// Renders the chat input box element.
pub(super) fn render_input_box(
    ui_registry: &mut AppUiRegistry,
    frame: &mut Frame<'_>,
    input: Rect,
    state: &AppState,
) {
    if let Some(element) = ui_registry.get_mut("chat-input-box") {
        element.render(frame, input, state);
    }
}
