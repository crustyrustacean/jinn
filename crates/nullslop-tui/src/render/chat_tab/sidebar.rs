//! Sidebar rendering.

use ratatui::Frame;
use ratatui::layout::Rect;

use nullslop_domain::AppState;
use nullslop_domain::feat::ui::sidebar::Sidebar;

/// Renders the sidebar and registers it as selectable when focused.
pub(super) fn render_sidebar(
    sidebar: &mut Sidebar,
    frame: &mut Frame<'_>,
    sidebar_rect: Rect,
    sidebar_focused: bool,
    state: &AppState,
    rects: &mut Vec<Rect>,
) {
    sidebar.render(frame, sidebar_rect, state);
    if sidebar_focused {
        rects.push(sidebar_rect);
    }
}
