//! Assistant entry rendering — markdown-rendered text.

use ratatui::text::Line;

use super::markdown::render_markdown;
use super::shared::RenderContext;

pub fn to_lines(text: &str, ctx: &RenderContext) -> Vec<Line<'static>> {
    render_markdown(text, ctx.content_width, &ctx.theme)
}
