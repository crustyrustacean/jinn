//! Scroll indicator - renders "↑ N lines above" when scrolled up.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::feat::theme::Theme;

/// Render the scroll indicator widget if the viewport is not at the bottom.
///
/// Shows "↑ N lines above" in the bottom-right corner of the chat area.
pub(crate) fn render_scroll_indicator(
    frame: &mut Frame<'_>,
    area: Rect,
    clamped: u16,
    max_offset: u16,
    theme: &Theme,
) {
    if clamped >= max_offset {
        return;
    }

    let hidden = max_offset - clamped;
    let label = format!(" ↑ {hidden} lines above ");
    let label_len = label.len();
    let indicator = Paragraph::new(Line::from(Span::styled(
        label,
        Style::default()
            .fg(theme.muted_text)
            .bg(theme.scroll_indicator_bg),
    )));
    let indicator_width = u16::try_from(label_len)
        .unwrap_or(area.width)
        .min(area.width);
    let indicator_area = Rect {
        x: area.x + area.width.saturating_sub(indicator_width),
        y: area.y + area.height.saturating_sub(1),
        width: indicator_width,
        height: 1,
    };
    frame.render_widget(indicator, indicator_area);
}
