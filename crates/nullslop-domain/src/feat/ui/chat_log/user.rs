//! User entry rendering — white on light gray background block.

use ratatui::style::Style;
use ratatui::text::Line;

use super::shared::{RenderContext, multiline_styled, pad_line_to_width};

pub fn to_lines(text: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let style = Style::default()
        .fg(ctx.theme.primary_text)
        .bg(ctx.theme.user_block_bg);
    let mut lines = multiline_styled(text, "", "", style);
    // Pad each line to full content width for BLOCK effect.
    for line in &mut lines {
        pad_line_to_width(
            line,
            ctx.content_width,
            Style::default().bg(ctx.theme.user_block_bg),
        );
    }
    lines
}
