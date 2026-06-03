//! Sidebar rendering.

use ratatui::Frame;
use ratatui::layout::Rect;

use jinn_domain::RenderCtx;
use jinn_domain::feat::ui::sidebar::Sidebar;

/// Renders the sidebar and registers it as selectable when focused.
pub fn render_sidebar(
    sidebar: &mut Sidebar,
    frame: &mut Frame<'_>,
    sidebar_rect: Rect,
    sidebar_focused: bool,
    ctx: &RenderCtx,
    rects: &mut Vec<Rect>,
) {
    sidebar.render(frame, sidebar_rect, ctx);
    if sidebar_focused {
        rects.push(sidebar_rect);
    }
}
