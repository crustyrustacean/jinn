//! Tool call entry rendering — light gray text on dark green background block.
//!
//! Truncation follows the same pattern as tool results:
//! - Shows up to `tool_entry_max_lines` content lines (default 6).
//! - When the content exceeds that, a `---(N more lines)---` indicator replaces
//!   the remaining lines.
//! - When expanded (`ctx.is_expanded`), all lines are shown.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::shared::{Pad, RenderContext, pad_entry_with, pad_line_to_width};

pub fn to_lines(name: &str, arguments: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let style = Style::default()
        .fg(ctx.theme.tool_fg)
        .bg(ctx.theme.tool_success_bg);
    let arguments = super::shared::unescape_newlines(arguments);
    let content = format!("{name}({arguments})");
    let text = content.trim_start_matches('\n');
    let all_lines: Vec<&str> = text.split('\n').collect();

    let show_all = ctx.is_expanded
        || u16::try_from(all_lines.len()).unwrap_or(u16::MAX) <= ctx.tool_entry_max_lines;

    let mut lines = Vec::new();
    if show_all {
        for line_text in &all_lines {
            lines.push(Line::from(Span::styled((*line_text).to_owned(), style)));
        }
    } else {
        let max = ctx.tool_entry_max_lines as usize;
        let remaining = all_lines.len() - max;
        for line_text in &all_lines[..max] {
            lines.push(Line::from(Span::styled((*line_text).to_owned(), style)));
        }
        // Truncation indicator line.
        let truncation_style = Style::default()
            .fg(ctx.theme.truncation_fg)
            .bg(ctx.theme.tool_success_bg);
        lines.push(Line::from(Span::styled(
            format!("---({remaining} more lines)---"),
            truncation_style,
        )));
    }

    // Pad each line to full content width for BLOCK effect.
    for line in &mut lines {
        pad_line_to_width(
            line,
            ctx.content_width,
            Style::default().bg(ctx.theme.tool_success_bg),
        );
    }

    // Add padding above and below with the tool block background.
    let pad_bg = Style::default().bg(ctx.theme.tool_success_bg);
    let pad_line = Line::from(Span::styled(" ".repeat(ctx.content_width as usize), pad_bg));
    pad_entry_with(&mut lines, Pad::Both, pad_line);
    lines
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::feat::ui::chat_log::shared::RenderContext;

    fn render_context(max_lines: u16, is_expanded: bool) -> RenderContext {
        RenderContext {
            content_width: 80,
            _is_selected: false,
            is_expanded,
            tool_entry_max_lines: max_lines,
            theme: crate::feat::theme::default_theme(),
        }
    }

    #[rstest::rstest]
    fn tool_call_with_literal_newline_renders_multiple_lines() {
        // Given a tool call with arguments containing literal \n.
        let ctx = render_context(5, false);

        // When converting to lines.
        let lines = to_lines("write", r#"{"path":"f.txt","content":"a\nb"}"#, &ctx);

        // Then the result has multiple lines (not just one).
        assert!(
            lines.len() > 1,
            "tool call with literal \\n should produce multiple lines, got {}",
            lines.len()
        );

        // And no line contains the literal two-character \n.
        for line in &lines {
            let text: String = line.spans.iter().map(|s| s.content.clone()).collect();
            assert!(
                !text.contains("\\n"),
                "line should not contain literal \\n, got: {text}"
            );
        }
    }

    #[rstest::rstest]
    fn truncated_tool_call_shows_indicator() {
        // Given a 10-line tool call with max_lines=6, not expanded.
        let ctx = render_context(6, false);
        let args: String = (1..=10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\\n");

        // When converting to lines.
        let lines = to_lines("bash", &args, &ctx);

        // Then some line contains the truncation indicator.
        let has_indicator = lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|s| s.content.contains("---(4 more lines)---"))
        });
        assert!(
            has_indicator,
            "truncated tool call should contain '---(4 more lines)---'"
        );
    }

    #[rstest::rstest]
    fn expanded_tool_call_shows_all_lines() {
        // Given a 10-line tool call with max_lines=6, expanded.
        let ctx = render_context(6, true);
        let args: String = (1..=10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\\n");

        // When converting to lines.
        let lines = to_lines("bash", &args, &ctx);

        // Then some line contains "line 10".
        let has_last_line = lines
            .iter()
            .any(|line| line.spans.iter().any(|s| s.content.contains("line 10")));
        assert!(has_last_line, "expanded tool call should contain 'line 10'");

        // And no line contains "more lines".
        let has_indicator = lines
            .iter()
            .any(|line| line.spans.iter().any(|s| s.content.contains("more lines")));
        assert!(
            !has_indicator,
            "expanded tool call should not show truncation indicator"
        );
    }

    #[rstest::rstest]
    fn short_tool_call_not_truncated() {
        // Given a 3-line tool call with max_lines=6, not expanded.
        let ctx = render_context(6, false);
        let args = "line 1\\nline 2\\nline 3";

        // When converting to lines.
        let lines = to_lines("bash", args, &ctx);

        // Then no line contains "more lines".
        let has_indicator = lines
            .iter()
            .any(|line| line.spans.iter().any(|s| s.content.contains("more lines")));
        assert!(!has_indicator, "short tool call should not be truncated");
    }
}
