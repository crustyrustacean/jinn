//! Tool call entry rendering — light gray text on dark green background block.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::shared::{RenderContext, pad_line_to_width};

pub fn to_lines(name: &str, arguments: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let style = Style::default()
        .fg(ctx.theme.tool_block_fg)
        .bg(ctx.theme.tool_success_bg);
    let content = format!("{name}({arguments})");
    let text = content.trim_start_matches('\n');
    let segments = text.split('\n');

    let mut lines = Vec::new();
    for segment in segments {
        let line = Line::from(Span::styled(segment.to_owned(), style));
        lines.push(line);
    }

    // Pad each line to full content width for BLOCK effect.
    for line in &mut lines {
        pad_line_to_width(
            line,
            ctx.content_width,
            Style::default().bg(ctx.theme.tool_success_bg),
        );
    }
    lines
}
