//! Skill entry rendering - collapsible block showing skill name and content.
//!
//! Format (collapsed):
//! ```text
//! <skill-name>
//! ---(N more lines)---
//! ```
//!
//! Format (expanded):
//! ```text
//! <skill-name>
//! <content line 1>
//! <content line 2>
//! ...
//! ```

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::shared::{RenderContext, pad_line_to_width, truncate_to_width};

/// Renders a skill entry as a collapsible block.
///
/// When collapsed, shows only the skill name and a truncation indicator.
/// When expanded, shows the full content.
pub fn to_lines(name: &str, content: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let name = super::shared::strip_ansi(name);
    let content = super::shared::strip_ansi(content);
    let style = Style::default()
        .fg(ctx.theme.tool_fg)
        .bg(ctx.theme.tool_success_bg);

    let mut lines = Vec::new();

    // Name line (always shown).
    let name_text = truncate_to_width(&name, ctx.content_width as usize);
    lines.push(Line::from(Span::styled(name_text, style)));

    let text = content.trim_start_matches('\n');
    let all_lines: Vec<&str> = text.split('\n').collect();

    if ctx.is_expanded {
        // Show full content.
        for line_text in &all_lines {
            lines.push(Line::from(Span::styled((*line_text).to_owned(), style)));
        }
    } else {
        // Show only the truncation indicator.
        let remaining = all_lines.len();
        let truncation_style = Style::default()
            .fg(ctx.theme.truncation_fg)
            .bg(ctx.theme.tool_success_bg);
        lines.push(Line::from(Span::styled(
            format!("---({remaining} more lines)---"),
            truncation_style,
        )));
    }

    // Pad all lines to full content width so background spans the entire row.
    let bg_style = Style::default().bg(ctx.theme.tool_success_bg);
    for line in &mut lines {
        pad_line_to_width(line, ctx.content_width, bg_style);
    }

    lines
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use crate::feat::ui::chat_log::shared::RenderContext;

    fn render_context(is_expanded: bool) -> RenderContext {
        RenderContext {
            content_width: 80,
            is_selected: false,
            is_expanded,
            tool_entry_max_lines: 5,
            theme: crate::feat::theme::default_theme(),
            paired_status: None,
            is_streaming: false,
        }
    }

    #[rstest::rstest]
    fn collapsed_shows_name_and_indicator() {
        // Given a skill entry with multi-line content, not expanded.
        let ctx = render_context(false);
        let content = "line 1\nline 2\nline 3\nline 4\nline 5";

        // When converting to lines.
        let lines = to_lines("my-skill", content, &ctx);

        // Then the first line is the skill name.
        let name_text: String = lines[0].spans.iter().map(|s| s.content.clone()).collect();
        assert!(
            name_text.starts_with("my-skill"),
            "first line should start with skill name, got: {name_text}"
        );

        // And the second line contains the truncation indicator.
        let indicator_text: String = lines[1].spans.iter().map(|s| s.content.clone()).collect();
        assert!(
            indicator_text.contains("5 more lines"),
            "second line should contain '5 more lines', got: {indicator_text}"
        );

        // And there are exactly 2 lines.
        assert_eq!(lines.len(), 2, "collapsed should have 2 lines");
    }

    #[rstest::rstest]
    fn expanded_shows_full_content() {
        // Given a skill entry with multi-line content, expanded.
        let ctx = render_context(true);
        let content = "line 1\nline 2\nline 3";

        // When converting to lines.
        let lines = to_lines("my-skill", content, &ctx);

        // Then all content lines are visible.
        // 1 name line + 3 content lines = 4 lines.
        assert_eq!(lines.len(), 4, "expanded should show all lines");

        // And no line contains "more lines".
        let has_indicator = lines
            .iter()
            .any(|line| line.spans.iter().any(|s| s.content.contains("more lines")));
        assert!(
            !has_indicator,
            "expanded should not show truncation indicator"
        );

        // And the last line contains the last content.
        let last_text: String = lines[3].spans.iter().map(|s| s.content.clone()).collect();
        assert!(
            last_text.starts_with("line 3"),
            "last line should contain 'line 3', got: {last_text}"
        );
    }

    #[rstest::rstest]
    fn collapsed_with_single_line_shows_one_more() {
        // Given a skill entry with one line of content, not expanded.
        let ctx = render_context(false);

        // When converting to lines.
        let lines = to_lines("test-skill", "only line", &ctx);

        // Then the indicator says "1 more lines".
        let indicator_text: String = lines[1].spans.iter().map(|s| s.content.clone()).collect();
        assert!(
            indicator_text.contains("1 more lines"),
            "should show '1 more lines', got: {indicator_text}"
        );
    }

    #[rstest::rstest]
    fn name_line_has_correct_styling() {
        // Given a skill entry.
        let ctx = render_context(false);
        let theme = crate::feat::theme::default_theme();

        // When converting to lines.
        let lines = to_lines("test-skill", "content", &ctx);

        // Then the name line has tool_fg foreground and tool_success_bg background.
        let name_line = &lines[0];
        let has_fg = name_line
            .spans
            .iter()
            .any(|s| s.style.fg == Some(theme.tool_fg));
        let has_bg = name_line
            .spans
            .iter()
            .any(|s| s.style.bg == Some(theme.tool_success_bg));
        assert!(has_fg, "name line should have tool_fg foreground");
        assert!(has_bg, "name line should have tool_success_bg background");
    }
}
