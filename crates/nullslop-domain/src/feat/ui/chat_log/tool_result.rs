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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feat::ui::chat_log::shared::RenderContext;

    fn render_context(max_lines: u16, is_expanded: bool) -> RenderContext {
        RenderContext {
            content_width: 80,
            _is_selected: false,
            is_pinned: false,
            is_expanded,
            tool_result_max_lines: max_lines,
        }
    }

    #[rstest::rstest]
    fn truncated_tool_result_shows_indicator() {
        // Given a 10-line tool result with max_lines=5, not expanded.
        let ctx = render_context(5, false);
        let content = (1..=10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");

        // When converting to lines.
        let lines = to_lines("bash", &content, true, &ctx);

        // Then some line contains the truncation indicator.
        let has_indicator = lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|s| s.content.contains("---(5 more lines)---"))
        });
        assert!(
            has_indicator,
            "truncated tool result should contain '---(5 more lines)---'"
        );
    }

    #[rstest::rstest]
    fn expanded_tool_result_shows_all_lines() {
        // Given a 10-line tool result with max_lines=5, expanded.
        let ctx = render_context(5, true);
        let content = (1..=10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");

        // When converting to lines.
        let lines = to_lines("bash", &content, true, &ctx);

        // Then some line contains "line 10".
        let has_last_line = lines.iter().any(|line| {
            line.spans.iter().any(|s| s.content.contains("line 10"))
        });
        assert!(has_last_line, "expanded tool result should contain 'line 10'");

        // And no line contains "more lines".
        let has_indicator = lines.iter().any(|line| {
            line.spans.iter().any(|s| s.content.contains("more lines"))
        });
        assert!(
            !has_indicator,
            "expanded tool result should not show truncation indicator"
        );
    }

    #[rstest::rstest]
    fn short_tool_result_not_truncated() {
        // Given a 3-line tool result with max_lines=5, not expanded.
        let ctx = render_context(5, false);
        let content = "line 1\nline 2\nline 3".to_owned();

        // When converting to lines.
        let lines = to_lines("bash", &content, true, &ctx);

        // Then no line contains "more lines".
        let has_indicator = lines.iter().any(|line| {
            line.spans.iter().any(|s| s.content.contains("more lines"))
        });
        assert!(
            !has_indicator,
            "short tool result should not be truncated"
        );
    }

    #[rstest::rstest]
    fn tool_result_name_on_first_line() {
        // Given a tool result with name "bash" and content "output".
        let ctx = render_context(5, false);

        // When converting to lines.
        let lines = to_lines("bash", "output", true, &ctx);

        // Then the first line contains "bash".
        let name_content: String = lines[0]
            .spans
            .iter()
            .map(|s| s.content.clone())
            .collect();
        assert!(
            name_content.starts_with("bash"),
            "first line should start with tool name"
        );

        // And the second line contains "output".
        let content_line: String = lines[1]
            .spans
            .iter()
            .map(|s| s.content.clone())
            .collect();
        assert!(
            content_line.starts_with("output"),
            "second line should start with content"
        );
    }
}
