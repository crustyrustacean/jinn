//! Assistant entry rendering — markdown-rendered text.

use ratatui::text::Line;

use super::markdown::render_markdown;
use super::shared::{RenderContext, pad_entry, Pad};

pub fn to_lines(text: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let mut lines = render_markdown(text, ctx.content_width, &ctx.theme);
    pad_entry(&mut lines, Pad::Both);
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
    fn assistant_to_lines_has_exactly_one_pad_above_and_below() {
        // Given assistant text with leading newlines (as reasoning models produce).
        let ctx = render_context();

        // When converting to lines.
        let lines = super::to_lines("\n\nHello! How can I help you today?", &ctx);

        // Then there are exactly 3 lines: pad + content + pad.
        assert_eq!(
            lines.len(),
            3,
            "assistant entry should have exactly 3 lines (pad + content + pad), got {}",
            lines.len()
        );
        // And the first line is the top pad.
        assert!(lines[0].spans.is_empty());
        // And the last line is the bottom pad.
        assert!(lines[2].spans.is_empty());
    }

    #[rstest::rstest]
    fn assistant_plain_text_no_extra_blank_lines() {
        // Given plain assistant text without any surrounding newlines.
        let ctx = render_context();

        // When converting to lines.
        let lines = super::to_lines("Hello!", &ctx);

        // Then there are exactly 3 lines: pad + content + pad.
        assert_eq!(
            lines.len(),
            3,
            "plain assistant should have 3 lines (pad + content + pad), got {}",
            lines.len()
        );
    }
}
