//! Chat log rendering and area computation.

use ratatui::Frame;
use ratatui::layout::Rect;

use nullslop_domain::{AppState, AppUiRegistry};

/// Renders the chat log element and registers it as selectable when not sidebar-focused.
pub(super) fn render_chat_log(
    ui_registry: &mut AppUiRegistry,
    frame: &mut Frame<'_>,
    chat_log_area: Rect,
    content_area: Rect,
    sidebar_focused: bool,
    state: &AppState,
    rects: &mut Vec<Rect>,
) {
    if let Some(element) = ui_registry.get_mut("chat-log") {
        element.render(frame, chat_log_area, state);
        if element.is_selectable() && !sidebar_focused {
            rects.push(content_area);
        }
    }
}
