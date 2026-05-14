//! Tool call entry rendering — light gray text on dark green background block.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use super::shared::{pad_line_to_width, RenderContext};

/// Foreground color for tool call text.
const TOOL_CALL_FG: Color = Color::Rgb(0x58, 0x5F, 0x6A);
/// Background color for tool call BLOCK.
const TOOL_CALL_BG: Color = Color::Rgb(0x28, 0x32, 0x28);

pub fn to_lines(name: &str, arguments: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let style = Style::default().fg(TOOL_CALL_FG).bg(TOOL_CALL_BG);
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
        pad_line_to_width(line, ctx.content_width, Style::default().bg(TOOL_CALL_BG));
    }
    lines
}
