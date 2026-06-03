//! Autocomplete popup overlay rendering.

use ratatui::Frame;
use ratatui::layout::Rect;

use jinn_domain::RenderCtx;

/// Renders the autocomplete popup overlay (transient, not a UiElement).
pub(super) fn render_autocomplete(frame: &mut Frame<'_>, input: Rect, ctx: &RenderCtx) {
    if ctx.state.active_chat_input().autocomplete().is_some() {
        jinn_domain::feat::chat_input::autocomplete_render::render_autocomplete_popup(
            frame, input, ctx.state,
        );
    }
}
