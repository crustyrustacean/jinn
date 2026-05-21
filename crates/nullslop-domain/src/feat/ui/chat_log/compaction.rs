//! Compaction entry rendering — compacted history header.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::shared::{Pad, RenderContext, pad_entry};

/// Renders a compaction entry as a single header line showing the message count
/// and token estimate. Uses muted text styling.
pub fn to_lines(
    entries_compacted: usize,
    tokens_before: usize,
    ctx: &RenderContext,
) -> Vec<Line<'static>> {
    let style = Style::default().fg(ctx.theme.muted_text);
    let header = format!("📜 Compacted ({entries_compacted} messages, ~{tokens_before} tokens)");

    let mut lines = vec![Line::from(Span::styled(header, style))];

    pad_entry(&mut lines, Pad::Both);
    lines
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]

    use super::*;
    use crate::feat::theme::default_theme;

    fn render_ctx() -> RenderContext {
        RenderContext {
            content_width: 80,
            _is_selected: false,
            is_expanded: false,
            tool_entry_max_lines: 20,
            theme: default_theme(),
            paired_status: None,
        }
    }

    #[test]
    fn header_shows_message_count_and_tokens() {
        // Given a compaction entry.
        let ctx = render_ctx();

        // When rendering.
        let lines = to_lines(5, 1000, &ctx);

        // Then the header line contains the compacted count and token estimate.
        let text: String = lines[1].spans.iter().map(|s| s.content.clone()).collect();
        assert!(text.contains("5 messages"));
        assert!(text.contains("~1000 tokens"));
    }

    #[test]
    fn padding_added_around_entry() {
        // Given a compaction entry.
        let ctx = render_ctx();

        // When rendering.
        let lines = to_lines(1, 100, &ctx);

        // Then the first and last lines are blank padding.
        assert!(lines[0].spans.is_empty());
        assert!(lines.last().unwrap().spans.is_empty());
    }
}
