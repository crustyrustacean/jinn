//! Tool result entry rendering — green/red with check/cross icon.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use super::shared::multiline_styled;

pub fn to_lines(
    name: &str,
    content: &str,
    success: bool,
    pinned: bool,
    is_selected: bool,
    is_expanded: bool,
    tool_result_max_lines: u16,
) -> Vec<Line<'static>> {
    let icon = if success { "✅" } else { "❌" };
    let prefix = if pinned { "📌 " } else { "  " };
    let style = if success {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Red)
    };

    let full_text = format!("{icon} {name}: {content}");
    let text = full_text.trim_start_matches('\n');
    let all_lines: Vec<&str> = text.split('\n').collect();

    if is_expanded
        || u16::try_from(all_lines.len()).unwrap_or(u16::MAX) <= tool_result_max_lines
    {
        multiline_styled(full_text, prefix, "  ", style, is_selected)
    } else {
        let max = tool_result_max_lines as usize;
        let remaining = all_lines.len() - max;
        let truncated_text: String = all_lines[..max].join("\n");
        let mut lines = multiline_styled(truncated_text, prefix, "  ", style, is_selected);
        lines.push(Line::from(Span::styled(
            format!("  ({remaining} more lines)"),
            Style::default().fg(Color::DarkGray),
        )));
        lines
    }
}
