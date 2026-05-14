//! Tool result entry rendering — light gray text on dark green/red background block.
//!
//! Format:
//! ```text
//! <name>
//! <content line 1>
//! <content line 2>
//! ---(N more lines)---
//! ```

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use super::shared::{pad_line_to_width, RenderContext};

/// Foreground color for tool result text.
const TOOL_RESULT_FG: Color = Color::Rgb(0x58, 0x5F, 0x6A);
/// Foreground color for truncation indicator.
const TRUNCATION_FG: Color = Color::Rgb(0x53, 0x53, 0x53);
/// Background color for successful tool result BLOCK.
const TOOL_RESULT_SUCCESS_BG: Color = Color::Rgb(0x28, 0x32, 0x28);
/// Background color for failed tool result BLOCK.
const TOOL_RESULT_FAILURE_BG: Color = Color::Rgb(0x3C, 0x28, 0x28);

pub fn to_lines(
    name: &str,
    content: &str,
    success: bool,
    ctx: &RenderContext,
) -> Vec<Line<'static>> {
    let bg = if success {
        TOOL_RESULT_SUCCESS_BG
    } else {
        TOOL_RESULT_FAILURE_BG
    };
    let style = Style::default().fg(TOOL_RESULT_FG).bg(bg);

    // Name line.
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(name.to_owned(), style)));

    // Content lines.
    let text = content.trim_start_matches('\n').to_owned();
    let all_lines: Vec<&str> = text.split('\n').collect();

    let show_all = ctx.is_expanded
        || u16::try_from(all_lines.len()).unwrap_or(u16::MAX) <= ctx.tool_result_max_lines;

    if show_all {
        for line_text in &all_lines {
            lines.push(Line::from(Span::styled((*line_text).to_owned(), style)));
        }
    } else {
        let max = ctx.tool_result_max_lines as usize;
        let remaining = all_lines.len() - max;
        for line_text in &all_lines[..max] {
            lines.push(Line::from(Span::styled((*line_text).to_owned(), style)));
        }
        // Truncation indicator line.
        let truncation_style = Style::default().fg(TRUNCATION_FG).bg(bg);
        lines.push(Line::from(Span::styled(
            format!("---({remaining} more lines)---"),
            truncation_style,
        )));
    }

    // Pad each line to full content width for BLOCK effect.
    for line in &mut lines {
        pad_line_to_width(line, ctx.content_width, Style::default().bg(bg));
    }
    lines
}
