//! Thinking entry rendering — dark gray.

use ratatui::style::Style;
use ratatui::text::Line;

use super::shared::{RenderContext, multiline_styled, pad_entry, Pad};

pub fn to_lines(text: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let mut lines = multiline_styled(text, "", "", Style::default().fg(ctx.theme.muted_text));
    pad_entry(&mut lines, Pad::Top);
    lines
}

#[cfg(test)]
mod tests {
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
    fn thinking_to_lines_has_top_pad_only() {
        // Given thinking text with a trailing newline (as reasoning models produce).
        let ctx = render_context();

        // When converting to lines.
        let lines = super::to_lines("The user is greeting me.\n", &ctx);

        // Then there are exactly 2 lines: pad + content (no bottom pad).
        assert_eq!(
            lines.len(),
            2,
            "thinking entry should have exactly 2 lines (pad + content), got {}",
            lines.len()
        );
        // And the first line is the top pad.
        assert!(lines[0].spans.is_empty());
        // And the last line has content (not a bottom pad).
        assert!(!lines[1].spans.is_empty());
    }

    #[rstest::rstest]
    fn thinking_multiline_trailing_newline_no_extra_blank() {
        // Given multi-line thinking text with trailing newline.
        let ctx = render_context();

        // When converting to lines.
        let lines = super::to_lines("line one\nline two\n", &ctx);

        // Then there are exactly 3 lines: pad + 2 content (no bottom pad).
        assert_eq!(
            lines.len(),
            3,
            "multi-line thinking should have 3 lines (pad + 2 content), got {}",
            lines.len()
        );
    }
}
