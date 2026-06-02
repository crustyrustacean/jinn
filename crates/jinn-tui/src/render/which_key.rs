//! Which-key popup overlay rendering.

use ratatui::Frame;
use ratatui::style::Style;
use ratatui_which_key::{PopupPosition, WhichKey};

use jinn_domain::RenderCtx;

use crate::app::WhichKeyInstance;

/// Renders the which-key popup overlay.
pub(super) fn render_which_key(
    frame: &mut Frame<'_>,
    state: &mut WhichKeyInstance,
    ctx: &RenderCtx,
) {
    let focus_accent = ctx.state.frontend.theme.focus_accent;
    let widget = WhichKey::new()
        .position(PopupPosition::BottomRight)
        .border_style(Style::default().fg(focus_accent));
    let buf = frame.buffer_mut();
    widget.render(buf, state);
}
