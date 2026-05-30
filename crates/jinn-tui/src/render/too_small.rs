//! "Terminal too small" fallback rendering.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Paragraph, Wrap};

use super::app_layout::{MIN_HEIGHT, MIN_WIDTH};
use crate::TuiApp;

/// Renders a "terminal too small" message and clears selectable rects.
pub(super) fn render_too_small(frame: &mut Frame<'_>, area: Rect, app: &mut TuiApp) {
    let msg = format!("Terminal too small\n{MIN_WIDTH}x{MIN_HEIGHT} minimum");
    let paragraph = Paragraph::new(msg).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
    // Clear selectable rects when terminal is too small.
    app.selectable_rects.rebuild(vec![]);
}
