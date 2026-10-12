//! Inter-session message rendering — the message, in the subagent color.
//!
//! The entry's text is self-describing: the sender bakes the
//! `[Message from session <id> (<title>)]: ` attribution into it at send
//! time, so rendering is the plain text, styled like subagent chatter. The
//! kind survives as the styling key; no header, no ids, no route chrome.

use ratatui::style::Style;
use ratatui::text::Line;

use super::shared::{Pad, RenderContext, multiline_styled, pad_entry};

pub fn to_lines(text: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    let text = super::shared::strip_ansi(text);
    let mut lines = multiline_styled(&text, "", "", Style::default().fg(ctx.theme.subagent_fg));
    pad_entry(&mut lines, Pad::Both);
    lines
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        reason = "test code"
    )]

    use super::*;
    use jinn_theme::default_theme;

    fn render_ctx() -> RenderContext {
        RenderContext {
            content_width: 80,
            is_selected: false,
            is_expanded: false,
            tool_entry_max_lines: 20,
            theme: default_theme(),
            paired_status: None,
            is_streaming: false,
            is_waiting_on_subagent: false,
        }
    }

    fn all_text(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.content.clone()))
            .collect()
    }

    #[rstest::rstest]
    fn the_text_renders_plain_with_no_header() {
        // Given a message whose text already carries the sender's attribution.
        let ctx = render_ctx();

        // When rendering.
        let lines = to_lines(
            "[Message from session abc-123]: handing off the stage",
            &ctx,
        );

        // Then the full text renders and no route chrome is added.
        let text = all_text(&lines);
        assert!(
            text.contains("[Message from session abc-123]: handing off the stage"),
            "the attributed text renders: {text:?}"
        );
        assert!(!text.contains('↔'), "no header arrow: {text:?}");
        assert!(!text.contains("→"), "no route arrow: {text:?}");
    }

    #[rstest::rstest]
    fn the_text_renders_in_the_subagent_color() {
        // Given a rendered inter-session message.
        let ctx = render_ctx();

        // When rendering.
        let lines = to_lines("[Message from session abc]: stage done", &ctx);

        // Then every content span carries the subagent foreground.
        let theme = default_theme();
        assert!(
            lines
                .iter()
                .flat_map(|line| line.spans.iter())
                .all(|span| span.style.fg == Some(theme.subagent_fg)),
            "text should be subagent_fg"
        );
    }

    #[rstest::rstest]
    fn multiline_text_renders_one_line_per_segment() {
        // Given a message with an embedded newline.
        let ctx = render_ctx();

        // When rendering.
        let lines = to_lines("[Message from session abc]: first\nsecond", &ctx);

        // Then the two content segments render as two styled lines (padded
        // above and below by the shared entry padding).
        let content: Vec<&Line<'static>> =
            lines.iter().filter(|line| !line.spans.is_empty()).collect();
        assert_eq!(content.len(), 2);
        let theme = default_theme();
        assert!(
            content
                .iter()
                .flat_map(|line| line.spans.iter())
                .all(|span| span.style.fg == Some(theme.subagent_fg))
        );
    }
}
