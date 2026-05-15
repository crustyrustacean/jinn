//! Assistant entry rendering — white text, no background.

use ratatui::style::Style;
use ratatui::text::Line;

use super::shared::{RenderContext, multiline_styled};

pub fn to_lines(text: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let style = Style::default().fg(ctx.theme.primary_text);
    multiline_styled(text, "", "", style)
}
