//! Render for the pruner accumulation threshold popup.

use jinn_slices::RenderFacts;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::intent::pruner_accumulation_slot;
use super::state::PrunerAccumulationInputState;

const POPUP_H_PAD_FRAC: f32 = 0.20;
const POPUP_MIN_WIDTH: u16 = 30;

/// Computes the centered popup rectangle.
#[must_use]
pub fn pruner_accumulation_popup_rect(area: Rect) -> Rect {
    let popup_width = ((f32::from(area.width) * (1.0 - 2.0 * POPUP_H_PAD_FRAC)).ceil() as u16)
        .max(POPUP_MIN_WIDTH)
        .min(area.width);
    let popup_height = 3u16.min(area.height);
    #[expect(clippy::integer_division, reason = "cell positions are integers")]
    let popup_x = area.width.saturating_sub(popup_width) / 2;
    #[expect(clippy::integer_division, reason = "cell positions are integers")]
    let popup_y = area.height.saturating_sub(popup_height) / 3;
    Rect::new(popup_x, popup_y, popup_width, popup_height)
}

/// Returns the registered geometry for the popup scope.
#[must_use]
pub fn pruner_accumulation_overlay_rect(area: &Rect) -> Option<Rect> {
    Some(pruner_accumulation_popup_rect(*area))
}

/// Renders the title, editable threshold, and cursor.
///
/// # Panics
///
/// Panics if the scope has not registered its cell. The overlay is only
/// mounted for that scope, so a missing cell is a wiring bug rather than
/// a state a user can reach.
#[expect(
    clippy::expect_used,
    reason = "the overlay only renders when the scope registered its cell"
)]
pub fn render_pruner_accumulation(frame: &mut Frame<'_>, area: Rect, ctx: &RenderFacts) {
    let cell = ctx
        .slices
        .reader::<PrunerAccumulationInputState>(&pruner_accumulation_slot())
        .expect("pruner accumulation overlay renders only with its registered cell");
    let state = cell.read();
    let theme = &ctx.theme;
    let popup_area = area;

    let title = Line::from(Span::styled(
        " Pruner Accumulation Threshold ",
        Style::default().fg(theme.popup_title),
    ));
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_unfocused));
    frame.render_widget(Clear, popup_area);
    frame.render_widget(block, popup_area);

    let inner = Rect {
        x: popup_area.x + 1,
        y: popup_area.y + 1,
        width: popup_area.width.saturating_sub(2),
        height: popup_area.height.saturating_sub(2),
    };
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let line = Line::from(vec![
        Span::styled("> ", Style::default().fg(theme.focus_accent)),
        Span::raw(&state.text.input),
    ]);
    frame.render_widget(
        Paragraph::new(line),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    let cursor_x =
        (2 + state.text.graphemes_before_cursor() as u16).min(inner.width.saturating_sub(1));
    frame.set_cursor_position((inner.x.saturating_add(cursor_x), inner.y));
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        reason = "test code, fresh-registry setup is infallible"
    )]
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[rstest::rstest]
    fn renders_title_and_seeded_threshold() {
        // Given a registry containing a seeded threshold cell.
        let slices = jinn_slices::Slices::new();
        let cell = slices
            .register(
                pruner_accumulation_slot(),
                PrunerAccumulationInputState::default(),
            )
            .expect("fresh registry has the pruner slot free");
        cell.update(|state| state.text.set("10000".to_owned()));
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal");

        // When the popup renders.
        terminal
            .draw(|frame| {
                let ctx = RenderFacts::new(jinn_theme::default_theme(), &slices);
                render_pruner_accumulation(
                    frame,
                    pruner_accumulation_popup_rect(frame.area()),
                    &ctx,
                );
            })
            .expect("test draw succeeds");

        // Then the existing title and value remain visible.
        let buffer = terminal.backend().buffer();
        let text: String = buffer
            .content
            .iter()
            .map(|cell| cell.symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(text.contains("Pruner Accumulation Threshold"));
        assert!(text.contains("10000"));
    }
}
