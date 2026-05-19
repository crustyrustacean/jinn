//! Streaming indicator element with animated throbber.
//!
//! Renders an animated ASCII spinner alongside "Working..." when the active
//! session is busy (sending or streaming), and renders nothing when idle.
//! Queue count is shown when messages are waiting.

use std::time::{Duration, Instant};

use crate::common::app_state::AppState;
use crate::common::ui_element::UiElement;
use crate::feat::session::chat_session::SessionPhase;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use throbber_widgets_tui::{Throbber, ThrobberState, WhichUse};

/// Minimum time between animation frame advances.
const ANIMATION_INTERVAL: Duration = Duration::from_millis(80);

/// Displays an animated streaming indicator when the active session is sending or streaming.
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
        if self.last_animation_step.elapsed() >= ANIMATION_INTERVAL {
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

impl UiElement<AppState> for StreamingIndicatorElement {
    fn name(&self) -> String {
        "streaming-indicator".to_owned()
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        let session = state.active_session();
        let queue_len = session.queue_len();

        let is_busy = matches!(session.phase(), SessionPhase::Sending | SessionPhase::Streaming);
        if !is_busy {
            return;
        }

        let label = if queue_len > 0 {
            format!(" Working... ({queue_len} queued)")
        } else {
            " Working...".to_owned()
        };

        let throbber = Throbber::default()
            .label(&label)
            .style(Style::default().fg(state.frontend.theme.streaming))
            .throbber_style(Style::default().fg(state.frontend.theme.streaming))
            .throbber_set(throbber_widgets_tui::ASCII)
            .use_type(WhichUse::Spin);

        frame.render_stateful_widget(throbber, area, &mut self.throbber_state);

        // Advance the animation step only when enough time has elapsed.
        self.maybe_advance_animation();
    }
}

#[cfg(test)]
mod tests {
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
}
