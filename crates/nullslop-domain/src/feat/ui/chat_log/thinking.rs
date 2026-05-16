//! Thinking entry rendering — dark gray.

use ratatui::style::Style;
use ratatui::text::Line;

use super::shared::{RenderContext, multiline_styled};

pub fn to_lines(text: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let mut lines = multiline_styled(text, "", "", Style::default().fg(ctx.theme.muted_text));
    lines.insert(0, Line::from(""));
    lines.push(Line::from(""));
    lines
}
