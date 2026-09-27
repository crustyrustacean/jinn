//! Streaming indicator element with animated throbber.
//!
//! Renders an animated ASCII spinner alongside "Working..." when the active
//! session is busy (sending, streaming, or compacting), and renders nothing
//! when idle. Queue count is shown when messages are waiting (not during
//! compaction).

use std::time::Instant;

use jinn_kernel::common::app_state::AppState;
use jinn_kernel::common::render_ctx::RenderCtx;
use jinn_kernel::common::ui_element::UiElement;
use jinn_session_msg::PhaseKind;
use jinn_slices::DrawContext;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use throbber_widgets_tui::{Throbber, ThrobberState, WhichUse};

/// Displays an animated streaming indicator when the active session is sending, streaming, or compacting.
#[derive(Debug)]
pub struct StreamingIndicatorElement {
    /// Visual-only state for the throbber animation step.
    throbber_state: ThrobberState,
    /// Timestamp of the last animation frame advance.
    last_animation_step: Instant,
}

impl StreamingIndicatorElement {
    /// Creates a new streaming indicator element.
    pub fn new() -> Self {
        Self {
            throbber_state: ThrobberState::default(),
            last_animation_step: Instant::now(),
        }
    }

    /// Advances the animation frame if enough time has elapsed.
    fn maybe_advance_animation(&mut self) {
        if self.last_animation_step.elapsed() >= jinn_slices::SPINNER_INTERVAL {
            self.throbber_state.calc_next();
            self.last_animation_step = Instant::now();
        }
    }
}

impl Default for StreamingIndicatorElement {
    fn default() -> Self {
        Self::new()
    }
}

/// Paints the streaming indicator into `area`.
///
/// The indicator holds throbber animation state, so the registered
/// draw function keeps exactly one element behind interior mutability
/// and the animation advances across frames.
pub fn paint(
    element: &mut StreamingIndicatorElement,
    frame: &mut Frame<'_>,
    area: Rect,
    ctx: &dyn DrawContext<AppState>,
) {
    element.render_body(frame, area, ctx.state());
}

impl StreamingIndicatorElement {
    /// The indicator's draw body, without the [`UiElement`] plumbing.
    fn render_body(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        let session = state.active_session();
        let phase = session.phase();

        let is_busy = session.is_busy();
        let is_phase_busy = matches!(phase, PhaseKind::Sending | PhaseKind::Streaming);

        if !is_busy && !is_phase_busy {
            return;
        }

        let label = if is_busy {
            " Working..."
        } else {
            " Streaming..."
        };

        let throbber = Throbber::default()
            .label(label)
            .style(Style::default().fg(state.frontend.theme.streaming))
            .throbber_style(Style::default().fg(state.frontend.theme.streaming))
            .throbber_set(throbber_widgets_tui::ASCII)
            .use_type(WhichUse::Spin);

        frame.render_stateful_widget(throbber, area, &mut self.throbber_state);

        // Advance the animation step only when enough time has elapsed.
        self.maybe_advance_animation();
    }
}

impl UiElement for StreamingIndicatorElement {
    fn name(&self) -> String {
        "streaming-indicator".to_owned()
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
        self.render_body(frame, area, ctx.state);
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use jinn_kernel::AppState;

    use super::*;

    #[rstest::rstest]
    fn name_returns_streaming_indicator() {
        // Given a StreamingIndicatorElement.
        let element = StreamingIndicatorElement::new();

        // When querying the name.
        let name = element.name();

        // Then it is "streaming-indicator".
        assert_eq!(name, "streaming-indicator");
    }

    #[rstest::rstest]
    fn renders_streaming_label_during_sending_phase() {
        // Given a session in Sending phase.
        use jinn_testutil::{buffer_row, setup_term};

        let mut element = StreamingIndicatorElement::new();
        let mut state = AppState::default();
        state.active_session_mut().begin_sending();
        let (mut terminal, area) = setup_term(30, 1);

        // When rendering the element.
        terminal
            .draw(|frame| {
                let slices = jinn_slices::Slices::new();
                let overlay_views = jinn_slices::OverlayViews::new();
                let ctx = RenderCtx::new_with_default_config(&state, &slices, &overlay_views);
                element.render(frame, area, &ctx);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = buffer_row(&buffer, 0, 30);

        // Then the label shows "Streaming...".
        assert!(
            row.contains("Streaming..."),
            "expected Streaming..., got: {row}"
        );
    }

    #[rstest::rstest]
    fn does_not_render_during_idle_phase() {
        // Given a session in Idle phase (default).
        use jinn_testutil::setup_term;

        let mut element = StreamingIndicatorElement::new();
        let state = AppState::default();
        let (mut terminal, area) = setup_term(30, 1);

        // When rendering the element.
        terminal
            .draw(|frame| {
                let slices = jinn_slices::Slices::new();
                let overlay_views = jinn_slices::OverlayViews::new();
                let ctx = RenderCtx::new_with_default_config(&state, &slices, &overlay_views);
                element.render(frame, area, &ctx);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();

        // Then the rendered area is empty (all spaces).
        let content: String = (0..30)
            .filter_map(|x| buffer.cell((x, 0)).map(ratatui::buffer::Cell::symbol))
            .collect();
        assert!(
            content.trim().is_empty(),
            "expected empty buffer, got: {content}"
        );
    }

    #[rstest::rstest]
    fn renders_working_for_marking_busy_when_idle() {
        // Given a session with busy counter set but phase Idle.
        use jinn_testutil::{buffer_row, setup_term};

        let mut element = StreamingIndicatorElement::new();
        let mut state = AppState::default();
        state.active_session_mut().begin_busy();
        let (mut terminal, area) = setup_term(30, 1);

        // When rendering the element.
        terminal
            .draw(|frame| {
                let slices = jinn_slices::Slices::new();
                let overlay_views = jinn_slices::OverlayViews::new();
                let ctx = RenderCtx::new_with_default_config(&state, &slices, &overlay_views);
                element.render(frame, area, &ctx);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = buffer_row(&buffer, 0, 30);

        // Then the label shows "Working...".
        assert!(
            row.contains("Working..."),
            "expected Working..., got: {row}"
        );
    }
}

#[cfg(test)]
mod registration_tests {
    use super::StreamingIndicatorElement;
    use jinn_kernel::common::AppUiRegistry;
    use jinn_kernel::common::ui_element::UiElement;

    #[rstest::rstest]
    fn register_adds_streaming_indicator() {
        // Given an empty registry.
        let mut registry = AppUiRegistry::new();

        // When registering the inference slice's UI elements.
        crate::register(&mut registry);

        // Then exactly 1 element was added (the streaming indicator).
        assert_eq!(
            registry.iter_mut().count(),
            1,
            "inference::register should add the streaming indicator"
        );
    }

    #[rstest::rstest]
    fn element_is_constructible() {
        // Given nothing.
        // When constructing the element.
        let element = StreamingIndicatorElement::new();

        // Then it is registered under its lookup name.
        assert_eq!(element.name(), "streaming-indicator");
    }
}
