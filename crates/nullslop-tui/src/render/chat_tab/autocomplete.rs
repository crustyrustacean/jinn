//! Autocomplete popup overlay rendering.

use ratatui::Frame;
use ratatui::layout::Rect;

use nullslop_domain::AppState;

/// Renders the autocomplete popup overlay (transient, not a UiElement).
pub(super) fn render_autocomplete(frame: &mut Frame<'_>, input: Rect, state: &AppState) {
    if state.active_chat_input().autocomplete().is_some() {
        nullslop_domain::feat::chat_input::autocomplete_render::render_autocomplete_popup(
            frame, input, state,
        );
    }
}
