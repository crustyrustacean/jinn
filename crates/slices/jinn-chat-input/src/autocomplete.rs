//! The autocomplete popup — a transient overlay drawn above the input
//! box while a completion is being previewed.
//!
//! Registered as part of the chat input box's draw function rather than
//! as a separate screen region, because it is anchored to the input
//! box's rect and exists only while that box is focused.

use jinn_kernel::AppState;
use jinn_slices::DrawContext;
use ratatui::Frame;
use ratatui::layout::Rect;

/// Renders the autocomplete popup overlay (transient, not a UiElement).
pub fn paint_autocomplete(frame: &mut Frame<'_>, input: Rect, ctx: &dyn DrawContext<AppState>) {
    if ctx
        .state()
        .active_session()
        .with_input(|i| i.autocomplete().is_some(), || false)
    {
        crate::autocomplete_render::render_autocomplete_popup(frame, input, ctx.state());
    }
}
