//! Annotation entry rendering - displays url_citation sources from server-side
//! web search tools (e.g. OpenRouter's `openrouter:web_search`).
//!
//! Renders a header line followed by one line per citation: the title followed
//! by the source URL in muted text.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::shared::{Pad, RenderContext, pad_entry};

/// Render a grouped annotation block: a header line, then one line per citation
/// showing `<title> <url>`.
pub fn to_lines(
    citations: &[jinn_provider::UrlCitation],
    ctx: &RenderContext,
) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(citations.len() + 1);

    let header_style = Style::default().fg(ctx.theme.muted_text);
    let body_style = Style::default().fg(ctx.theme.primary_text);
    let url_style = Style::default().fg(ctx.theme.muted_text);

    // Header line.
    lines.push(Line::from(Span::styled(
        format!("Sources ({})", citations.len()),
        header_style,
    )));

    // One line per citation.
    for citation in citations {
        let title = if citation.title.is_empty() {
            "(untitled)"
        } else {
            citation.title.as_str()
        };
        lines.push(Line::from(vec![
            Span::styled(format!("• {title} "), body_style),
            Span::styled(citation.url.clone(), url_style),
        ]));
    }

    pad_entry(&mut lines, Pad::Both);
    lines
}
