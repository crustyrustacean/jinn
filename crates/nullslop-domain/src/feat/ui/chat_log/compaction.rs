//! Compaction entry rendering — collapsible summary block.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::shared::{Pad, RenderContext, multiline_styled, pad_entry};

/// Renders a compaction entry as a collapsible block.
///
/// **Collapsed**: A single header line showing the message count and token estimate
/// with a hint to expand. Uses muted text styling.
///
/// **Expanded**: Header line (with collapse hint) + full summary text wrapped to
/// content width + model footer line.
pub fn to_lines(
    summary: &str,
    entries_compacted: usize,
    tokens_before: usize,
    model_used: &str,
    ctx: &RenderContext,
) -> Vec<Line<'static>> {
    let style = Style::default().fg(ctx.theme.muted_text);
    let hint = if ctx.is_expanded {
        "press e to collapse"
    } else {
        "press e to expand"
    };
    let header =
        format!("📜 Compacted ({entries_compacted} messages, ~{tokens_before} tokens) — {hint}");

    let mut lines = vec![Line::from(Span::styled(header, style))];

    if ctx.is_expanded {
        // Add a blank separator line.
        lines.push(Line::from(""));

        // Full summary text.
        let summary_lines = multiline_styled(summary, "", "", style);
        lines.extend(summary_lines);

        // Model footer.
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("model: {model_used}"),
            Style::default().fg(ctx.theme.muted_text),
        )));
    }

    pad_entry(&mut lines, Pad::Both);
    lines
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]

    use super::*;
    use crate::feat::theme::default_theme;

    fn render_ctx(is_expanded: bool) -> RenderContext {
        RenderContext {
            content_width: 80,
            _is_selected: false,
            is_expanded,
            tool_entry_max_lines: 20,
            theme: default_theme(),
            paired_status: None,
        }
    }

    #[test]
    fn collapsed_shows_header_with_hint() {
        // Given a collapsed compaction entry.
        let ctx = render_ctx(false);

        // When rendering.
        let lines = to_lines("summary text", 5, 1000, "test/model", &ctx);

        // Then the header line contains the compacted count and expand hint.
        let text: String = lines[1].spans.iter().map(|s| s.content.clone()).collect();
        assert!(text.contains("5 messages"));
        assert!(text.contains("~1000 tokens"));
        assert!(text.contains("press e to expand"));
    }

    #[test]
    fn expanded_shows_summary_text() {
        // Given an expanded compaction entry.
        let ctx = render_ctx(true);

        // When rendering.
        let lines = to_lines("key insight here", 3, 500, "test/model", &ctx);

        // Then the summary text appears in the output.
        let all_text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(all_text.contains("key insight here"));
    }

    #[test]
    fn expanded_shows_model_footer() {
        // Given an expanded compaction entry.
        let ctx = render_ctx(true);

        // When rendering.
        let lines = to_lines("summary", 3, 500, "anthropic/claude-sonnet", &ctx);

        // Then the model name appears in the output.
        let all_text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(all_text.contains("model: anthropic/claude-sonnet"));
    }

    #[test]
    fn collapsed_hides_summary_and_model() {
        // Given a collapsed compaction entry.
        let ctx = render_ctx(false);

        // When rendering.
        let lines = to_lines("secret summary", 3, 500, "test/model", &ctx);

        // Then the summary text is NOT in the output.
        let all_text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.clone()))
            .collect();
        assert!(!all_text.contains("secret summary"));
        assert!(!all_text.contains("model: test/model"));
    }

    #[test]
    fn padding_added_around_entry() {
        // Given a compaction entry.
        let ctx = render_ctx(false);

        // When rendering.
        let lines = to_lines("summary", 1, 100, "model", &ctx);

        // Then the first and last lines are blank padding.
        assert!(lines[0].spans.is_empty());
        assert!(lines.last().unwrap().spans.is_empty());
    }
}
