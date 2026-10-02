//! The animated "Loading session..." status line.
//!
//! Separate from the conversation pipeline in [`super::history`] because it is a
//! different widget with a different job: the chat log paints entries, and this
//! paints one line of status on the chat log's bottom row while a session is
//! loading, before any history exists to draw. The two share no layout — the
//! chat log's area already excludes this row — so the only thing that passes
//! between them is the frame.

use std::time::Instant;

use jinn_theme::Theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use throbber_widgets_tui::{Throbber, ThrobberState, WhichUse};

/// The text shown while a session is loading, with a leading space so it
/// clears the spinner glyph.
pub(crate) const LOADING_LABEL: &str = " Loading session...";

/// The row the loading line is drawn on, counted up from the chat log's bottom.
///
/// The chat log's own area already excludes the indicator and bottom-line rows
/// that `render_chat_tab` reserves, so its last row sits directly above the
/// chat bar. Anchoring there keeps the message clear of both the indicator
/// and the input box.
const LOADING_ROW_FROM_BOTTOM: u16 = 1;

/// The loading status line and the animation state driving it.
///
/// The animation state lives here rather than on [`ChatLogElement`] because the
/// two fields are only ever read and written together: a widget that owns its
/// own frame pacing does not need its caller to pass `&mut` to two fields and
/// keep them in step.
#[derive(Debug, Default)]
pub(crate) struct LoadingIndicator {
    /// Drives the throbber's animation.
    throbber_state: ThrobberState,
    /// Wall-clock of the last animation advance, so the spinner only steps
    /// once the animation interval has elapsed.
    last_advance: Option<Instant>,
}

impl LoadingIndicator {
    /// Renders an animated "Loading session..." line while a session loads.
    ///
    /// Drawn on the chat log's last row — the row directly above the chat bar — so
    /// the message reads as a status line under the conversation rather than
    /// floating in the middle of it. Coloured with the same theme color as the
    /// streaming indicator's spinner.
    pub(crate) fn paint(&mut self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let Some(row) = area
            .height
            .checked_sub(LOADING_ROW_FROM_BOTTOM)
            .filter(|row| *row > 0)
        else {
            return;
        };
        let row_area = Rect {
            y: area.y.saturating_add(row),
            height: 1,
            ..area
        };

        let style = Style::default().fg(theme.streaming);
        let throbber = Throbber::default()
            .label(LOADING_LABEL)
            .style(style)
            .throbber_style(style)
            .throbber_set(throbber_widgets_tui::ASCII)
            .use_type(WhichUse::Spin);

        // `Throbber` renders left-aligned with no alignment option, so centre the
        // line by starting it half the leftover space in. When the log is narrower
        // than the label there is no slack, and it simply starts at the edge.
        let glyph_and_label = u16::try_from(LOADING_LABEL.len())
            .unwrap_or(u16::MAX)
            .saturating_add(1);
        let slack = row_area.width.saturating_sub(glyph_and_label);
        let start = row_area.x.saturating_add(slack / 2);
        let width = row_area.width.min(glyph_and_label).max(1).min(
            row_area
                .x
                .saturating_add(row_area.width)
                .saturating_sub(start),
        );
        if width == 0 {
            return;
        }
        let centered = Rect {
            x: start,
            width,
            ..row_area
        };
        frame.render_stateful_widget(throbber, centered, &mut self.throbber_state);

        // Advance the animation only once the interval has elapsed, matching the
        // streaming indicator's pacing.
        let now = Instant::now();
        if self
            .last_advance
            .is_none_or(|last| now.duration_since(last) >= jinn_slices::SPINNER_INTERVAL)
        {
            self.throbber_state.calc_next();
            self.last_advance = Some(now);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use super::*;
    use crate::kernel_element::history::ChatLogElement;
    use jinn_kernel::common::app_state::AppState;
    use jinn_kernel::common::render_ctx::RenderCtx;
    use jinn_kernel::common::ui_element::UiElement;
    use jinn_testutil::{buffer_row, setup_term};
    use ratatui::style::Color;

    /// Renders the element once with a session load in flight.
    fn render_loading_into(state: &AppState, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let mut element = ChatLogElement::new();
        let (mut terminal, area) = setup_term(width, height);
        {
            let slices = jinn_slices::Slices::new();
            let overlay_views = jinn_slices::OverlayViews::new();
            let ctx = RenderCtx::new_with_default_config(state, &slices, &overlay_views);
            terminal
                .draw(|frame| element.render(frame, area, &ctx))
                .expect("draw");
        }
        terminal.backend().buffer().clone()
    }

    /// A session with a load in flight.
    fn loading_state() -> AppState {
        let mut state = AppState::default();
        state.session.begin_load(jinn_core_types::SessionId::new());
        assert!(state.session.is_loading(), "guard must be set");
        state
    }

    #[rstest::rstest]
    fn loading_line_shows_the_label() {
        // Given a session that is loading.
        let state = loading_state();

        // When rendering the chat log.
        let buffer = render_loading_into(&state, 30, 10);

        // Then the loading label appears.
        let rows: Vec<String> = (0..10)
            .map(|y| {
                (0..30)
                    .map(|x| {
                        buffer
                            .cell((x, y))
                            .map_or("?", ratatui::buffer::Cell::symbol)
                    })
                    .collect()
            })
            .collect();
        assert!(
            rows.iter().any(|row| row.contains("Loading session...")),
            "expected the loading label, got: {rows:?}"
        );
    }

    #[rstest::rstest]
    fn loading_line_sits_directly_above_the_chat_input() {
        // Given a session that is loading in a 10-row area.
        let state = loading_state();

        // When rendering the chat log.
        let buffer = render_loading_into(&state, 30, 10);

        // Then the label sits on the chat log's last row, directly above the
        // indicator and chat bar.
        let label_row = buffer_row(&buffer, 10 - LOADING_ROW_FROM_BOTTOM, 30);
        assert!(
            label_row.contains("Loading session..."),
            "expected the label on row {}, got: {label_row}",
            10 - LOADING_ROW_FROM_BOTTOM
        );
    }

    #[rstest::rstest]
    fn loading_label_uses_the_streaming_color() {
        // Given a session that is loading.
        let state = loading_state();

        // When rendering the chat log.
        let buffer = render_loading_into(&state, 30, 10);

        // Then the label's cells carry the streaming theme color, not the
        // muted grey it used to use.
        let y = 10 - LOADING_ROW_FROM_BOTTOM;
        let label_start = buffer_row(&buffer, y, 30)
            .find("Loading session...")
            .expect("label present") as u16;
        let fg = buffer.cell((label_start, y)).expect("cell").fg;
        assert_eq!(fg, state.frontend.theme.streaming);
        assert_ne!(fg, Color::Gray, "the loading label must not stay grey");
    }

    #[rstest::rstest]
    fn loading_line_is_centred_across_the_log() {
        // Given a session that is loading in a 30-column log.
        let state = loading_state();

        // When rendering the chat log.
        let buffer = render_loading_into(&state, 30, 10);

        // Then the whole line — spinner glyph and label — is centred, leaving
        // roughly equal blank space on either side.
        let y = 10 - LOADING_ROW_FROM_BOTTOM;
        let row = buffer_row(&buffer, y, 30);
        let label_start = row.find("Loading session...").expect("label present") as u16;
        // The spinner glyph sits one column left of the label, which itself
        // starts with a space, so the line begins two columns earlier.
        let start = label_start.saturating_sub(2);
        let end = label_start.saturating_add(LOADING_LABEL.trim().len() as u16);
        let left_gap = start;
        let right_gap = 30u16.saturating_sub(end);
        // The glyph is followed by a space and the label by its own leading
        // space, so the drawn line is two cells wider than the text itself;
        // allow for that when comparing the gaps.
        assert!(
            left_gap.abs_diff(right_gap) <= 2,
            "line should be centred: left gap {left_gap}, right gap {right_gap}, row: {row:?}"
        );
    }

    #[rstest::rstest]
    fn loading_line_is_hidden_when_the_area_is_too_short() {
        // Given a session that is loading in an area with no room for the row.
        let state = loading_state();

        // When rendering into a one-row area.
        let buffer = render_loading_into(&state, 30, 1);

        // Then nothing is drawn.
        assert_eq!(buffer_row(&buffer, 0, 30).trim(), "");
    }
}
