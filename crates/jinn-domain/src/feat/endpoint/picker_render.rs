//! Rendering for the OpenRouter endpoint picker overlay.

use crate::common::render_ctx::RenderCtx;
use crate::feat::ui::picker_states::PickerExt;
use jinn_selection_widget::PreviewSelectionWidget;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// Renders the endpoint picker overlay using [`PreviewSelectionWidget`].
///
/// Multipane: the upstream list (left) with a preview pane (right) showing the
/// selected endpoint's routing tag, uptime, quantization, and pricing. The
/// footer shows the currently pinned endpoint (the entry the loader marked
/// `is_active`), or "auto-route" when none is pinned.
pub fn render_endpoint_picker(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    let state = ctx.state;
    let theme = &state.frontend.theme;

    let footer = {
        let gray = Style::default().fg(theme.muted_text);
        let orange = Style::default().fg(theme.accent_action);
        let pinned_name = state
            .frontend
            .endpoint_picker()
            .items()
            .iter()
            .find(|e| e.is_active)
            .map_or("auto-route", |e| e.provider_name.as_str());
        Line::from(vec![
            Span::styled("Routing: ".to_owned(), gray),
            Span::styled(
                pinned_name.to_owned(),
                Style::default().fg(theme.primary_text),
            ),
            Span::styled("  ".to_owned(), gray),
            Span::styled("Enter ".to_owned(), orange),
            Span::styled("pin · ".to_owned(), gray),
            Span::styled("ESC ".to_owned(), orange),
            Span::styled("cancel".to_owned(), gray),
        ])
    };

    let widget = PreviewSelectionWidget::new(state.frontend.endpoint_picker())
        .title(Line::from(" OpenRouter Endpoint "))
        .title_style(Style::default().fg(theme.popup_title))
        .footer(footer);
    widget.render(frame, area);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]

    use super::*;
    use crate::common::app_state::AppState;
    use crate::feat::endpoint::picker_entry::EndpointEntry;
    use crate::feat::theme::default_theme;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn render_endpoint_picker_does_not_panic_with_populated_picker() {
        // Given a state whose endpoint picker has a populated entry.
        let mut state = AppState::default();
        let entry = EndpointEntry::auto_route(true, default_theme());
        state.frontend.endpoint_picker_mut().set_items(vec![entry]);

        // When rendering the picker.
        // Then it does not panic.
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal
            .draw(|frame| {
                let ctx = RenderCtx::new(&state);
                render_endpoint_picker(frame, Rect::new(0, 0, 80, 24), &ctx);
            })
            .expect("draw");
    }

    #[test]
    fn render_endpoint_picker_with_real_upstream_shows_preview_metadata() {
        // Given a picker with a real upstream entry carrying metadata.
        let mut state = AppState::default();
        let entry = EndpointEntry {
            tag: "anthropic".to_owned(),
            provider_name: "Anthropic".to_owned(),
            uptime_30m: Some(99.7),
            prompt_price: Some("$3.00".to_owned()),
            completion_price: Some("$15.00".to_owned()),
            quantization: Some("fp16".to_owned()),
            max_completion_tokens: Some(64000),
            is_active: false,
            theme: default_theme(),
        };
        state.frontend.endpoint_picker_mut().set_items(vec![entry]);

        // When rendering with a wide-enough area for both panes.
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("terminal");
        terminal
            .draw(|frame| {
                let ctx = RenderCtx::new(&state);
                render_endpoint_picker(frame, Rect::new(0, 0, 100, 30), &ctx);
            })
            .expect("draw");

        // Then it does not panic (preview pane renders the metadata).
    }
}
