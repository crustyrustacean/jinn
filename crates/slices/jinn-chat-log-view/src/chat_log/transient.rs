//! Transient entry rendering - markdown-rendered text with padding.
//!
//! Transient entries are UI-only messages (welcome hints, status notifications, etc.)
//! that are excluded from prompt assembly and LLM context. They carry markdown text
//! rendered at display time for proper reflow on resize.

use super::markdown::render_markdown;
use super::shared::{Pad, RenderContext, pad_entry};

/// Render transient markdown text with padding.
///
/// Renders the markdown text through the markdown renderer using the current
/// content width, then adds blank padding above and below for visual spacing.
pub fn to_lines(text: &str, ctx: &RenderContext) -> Vec<ratatui::text::Line<'static>> {
    let text = super::shared::strip_ansi(text);
    let mut lines = render_markdown(&text, ctx.content_width, &ctx.theme);
    pad_entry(&mut lines, Pad::Both);
    lines
}
