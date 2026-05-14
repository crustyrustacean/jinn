//! Which-key popup overlay rendering.

use ratatui::Frame;
use ratatui::style::{Color, Style};
use ratatui_which_key::{PopupPosition, WhichKey};

use crate::app::WhichKeyInstance;

/// Renders the which-key popup overlay.
pub(super) fn render_which_key(frame: &mut Frame<'_>, state: &mut WhichKeyInstance) {
    let widget = WhichKey::new()
        .position(PopupPosition::BottomRight)
        .border_style(Style::default().fg(Color::Yellow));
    let buf = frame.buffer_mut();
    widget.render(buf, state);
}
