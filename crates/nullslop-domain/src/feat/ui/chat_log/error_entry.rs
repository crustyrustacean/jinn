//! Error entry rendering — red text.

use ratatui::style::Style;
use ratatui::text::Line;

use super::shared::{RenderContext, multiline_styled, pad_entry, Pad};

pub fn to_lines(text: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let mut lines = multiline_styled(text, "", "", Style::default().fg(ctx.theme.error_text));
    pad_entry(&mut lines, Pad::Both);
    lines
}
