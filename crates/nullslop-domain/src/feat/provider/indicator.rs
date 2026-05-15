//! Streaming indicator element with animated throbber.
//!
//! Renders an animated ASCII spinner alongside "📤 Sending..." when the active
//! session has dispatched a message but no tokens have arrived yet, "🧠 Streaming..."
//! when tokens are arriving, and renders nothing when the session is idle.
//! Queue count is shown when messages are waiting.

use crate::common::app_state::AppState;
use crate::common::ui_element::UiElement;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use throbber_widgets_tui::{Throbber, ThrobberState, WhichUse};

/// Displays an animated streaming indicator when the active session is sending or streaming.
#[derive(Debug)]
pub struct StreamingIndicatorElement {
    /// Visual-only state for the throbber animation step.
    throbber_state: ThrobberState,
}

impl StreamingIndicatorElement {
    /// Creates a new streaming indicator element.
    pub fn new() -> Self {
        Self {
            throbber_state: ThrobberState::default(),
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

        let label = if session.is_sending() {
            if queue_len > 0 {
                format!(" 📤 Sending... ({queue_len} queued)")
            } else {
                " 📤 Sending...".to_owned()
            }
        } else if session.is_streaming() {
            if queue_len > 0 {
                format!(" 🧠 Streaming... ({queue_len} queued)")
            } else {
                " 🧠 Streaming...".to_owned()
            }
        } else {
            return;
        };

        let throbber = Throbber::default()
            .label(&label)
            .style(Style::default().fg(state.frontend.theme.streaming))
            .throbber_style(Style::default().fg(state.frontend.theme.streaming))
            .throbber_set(throbber_widgets_tui::ASCII)
            .use_type(WhichUse::Spin);

        frame.render_stateful_widget(throbber, area, &mut self.throbber_state);

        // Advance the animation step for the next frame.
        self.throbber_state.calc_next();
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
