//! User entry rendering — markdown-rendered text on a block background.

use ratatui::style::Style;
use ratatui::text::Line;

use super::markdown::render_markdown;
use super::shared::{RenderContext, pad_line_to_width};

pub fn to_lines(text: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let mut lines = render_markdown(text, ctx.content_width, &ctx.theme);
    // Apply user block background to every line and pad to full width.
    let bg = Style::default().bg(ctx.theme.user_block_bg);
    for line in &mut lines {
        // Patch each span to include the user block background while preserving
        // inline markdown styling (bold, code, etc.).
        for span in &mut line.spans {
            span.style = span.style.patch(bg);
        }
        pad_line_to_width(line, ctx.content_width, bg);
    }
    lines
}
