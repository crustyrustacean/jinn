//! Tool call entry rendering — light gray text on dark green background block.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::shared::{RenderContext, pad_line_to_width};

pub fn to_lines(name: &str, arguments: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let style = Style::default()
        .fg(ctx.theme.tool_block_fg)
        .bg(ctx.theme.tool_success_bg);
    let arguments = super::shared::unescape_newlines(arguments);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feat::ui::chat_log::shared::RenderContext;

    fn render_context() -> RenderContext {
        RenderContext {
            content_width: 80,
            _is_selected: false,
            is_pinned: false,
            is_expanded: false,
            tool_result_max_lines: 5,
            theme: crate::feat::theme::default_theme(),
        }
    }

    #[rstest::rstest]
    fn tool_call_with_literal_newline_renders_multiple_lines() {
        // Given a tool call with arguments containing literal \n.
        let ctx = render_context();

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
}
