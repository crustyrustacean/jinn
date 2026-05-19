//! Skill entry rendering — collapsible block showing skill name and content.
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

use super::shared::{RenderContext, pad_line_to_width};

/// Renders a skill entry as a collapsible block.
///
/// When collapsed, shows only the skill name and a truncation indicator.
/// When expanded, shows the full content.
pub fn to_lines(name: &str, content: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let bg = ctx.theme.tool_success_bg;
    let style = Style::default().fg(ctx.theme.tool_block_fg).bg(bg);

    let mut lines = Vec::new();

    // Name line (always shown).
    lines.push(Line::from(Span::styled(name.to_owned(), style)));

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
        let truncation_style = Style::default().fg(ctx.theme.truncation_fg).bg(bg);
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
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::feat::ui::chat_log::shared::RenderContext;

    fn render_context(is_expanded: bool) -> RenderContext {
        RenderContext {
            content_width: 80,
            _is_selected: false,
            is_expanded,
            tool_entry_max_lines: 5,
            theme: crate::feat::theme::default_theme(),
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
    fn name_line_has_block_background() {
        // Given a skill entry.
        let ctx = render_context(false);

        // When converting to lines.
        let lines = to_lines("test-skill", "content", &ctx);

        // Then the name line spans have tool_success_bg background.
        let theme = crate::feat::theme::default_theme();
        let has_bg = lines[0]
            .spans
            .iter()
            .any(|s| s.style.bg == Some(theme.tool_success_bg));
        assert!(has_bg, "name line should have tool_success_bg background");
    }
}
