//! Overlay rendering for the session-lifecycle argument popup.
//!
//! The popup shows the snapshotted command template, highlights parameters as
//! they are filled, and keeps the editable argument line at the bottom. Its
//! geometry is registered from the cell-backed view so command-line count and
//! popup height stay aligned.

use jinn_session_lifecycle_msg::ArgInputState;
use jinn_session_lifecycle_msg::arg_input_slot;
use jinn_session_lifecycle_msg::command_template::split_preserving_quotes;
use jinn_slices::RenderFacts;
use jinn_slices::cell::TypedCell;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

const POPUP_H_PAD_FRAC: f32 = 0.10;
const POPUP_MIN_WIDTH: u16 = 40;
const TITLE: &str = " New Session (set script args) ";

fn content_rows(state: &ArgInputState) -> u16 {
    let display_args = split_preserving_quotes(&state.text.input);
    let line_count = state.template.display_line_segments(&display_args).len();
    u16::try_from(line_count)
        .unwrap_or(u16::MAX)
        .saturating_add(2)
}

fn param_color(theme: &jinn_theme::Theme) -> Color {
    theme.accent_action
}

fn compute_popup_rect(area: Rect, content_rows: u16) -> Rect {
    let popup_width = ((f32::from(area.width) * (1.0 - 2.0 * POPUP_H_PAD_FRAC)).ceil() as u16)
        .max(POPUP_MIN_WIDTH)
        .min(area.width);
    let popup_height = content_rows.saturating_add(2).min(area.height);
    #[expect(clippy::integer_division, reason = "cell positions are integers")]
    let popup_x = area.width.saturating_sub(popup_width) / 2;
    #[expect(clippy::integer_division, reason = "cell positions are integers")]
    let popup_y = area.height.saturating_sub(popup_height) / 3;
    Rect::new(popup_x, popup_y, popup_width, popup_height)
}

/// Computes the popup rectangle from the registered argument cell.
#[must_use]
pub fn arg_input_overlay_rect(area: Rect, cell: &TypedCell<ArgInputState>) -> Rect {
    compute_popup_rect(area, content_rows(&cell.read()))
}

/// Renders the lifecycle argument overlay over its registered popup rectangle.
///
/// # Panics
///
/// Panics when the lifecycle argument cell is absent. The renderer is only
/// reachable after activation registered that cell.
#[expect(
    clippy::expect_used,
    reason = "the overlay only renders when the lifecycle scope registered its cell"
)]
pub fn render_arg_input(frame: &mut Frame<'_>, area: Rect, ctx: &RenderFacts) {
    let cell = ctx
        .slices
        .reader(&arg_input_slot())
        .expect("lifecycle argument overlay renders only when its cell is registered");
    let state = cell.read();
    draw(frame, area, &state, &ctx.theme);
}

fn draw(frame: &mut Frame<'_>, popup_area: Rect, state: &ArgInputState, theme: &jinn_theme::Theme) {
    frame.render_widget(Clear, popup_area);
    if state.template.display().is_empty() {
        frame.render_widget(minimal_block(theme), popup_area);
        return;
    }
    frame.render_widget(popup_block(theme), popup_area);

    let inner = Rect {
        x: popup_area.x.saturating_add(1),
        y: popup_area.y.saturating_add(1),
        width: popup_area.width.saturating_sub(2),
        height: popup_area.height.saturating_sub(2),
    };
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    draw_template_lines(frame, inner, state, theme);
    draw_input_line(frame, inner, state, theme);
}

fn minimal_block(theme: &jinn_theme::Theme) -> Block<'static> {
    Block::default()
        .title(TITLE)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_unfocused))
}

fn popup_block(theme: &jinn_theme::Theme) -> Block<'static> {
    Block::default()
        .title(Line::from(Span::styled(
            TITLE,
            Style::default().fg(theme.popup_title),
        )))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_unfocused))
}

fn draw_template_lines(
    frame: &mut Frame<'_>,
    inner: Rect,
    state: &ArgInputState,
    theme: &jinn_theme::Theme,
) {
    let display_args = split_preserving_quotes(&state.text.input);
    let lines = state.template.display_line_segments(&display_args);
    let max_y = inner.y.saturating_add(inner.height);
    let mut y = inner.y;
    for segments in lines {
        if y >= max_y {
            break;
        }
        let spans = segments
            .iter()
            .map(|segment| match segment.param_index {
                Some(_) => Span::styled(
                    segment.text.clone(),
                    Style::default().fg(param_color(theme)),
                ),
                None => Span::raw(segment.text.clone()),
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(inner.x, y, inner.width, 1),
        );
        y = y.saturating_add(1);
    }
    if y < max_y {
        let separator = Span::styled(
            "─".repeat(usize::from(inner.width)),
            Style::default().fg(theme.border_unfocused),
        );
        frame.render_widget(
            Paragraph::new(Line::from(separator)),
            Rect::new(inner.x, y, inner.width, 1),
        );
    }
}

fn draw_input_line(
    frame: &mut Frame<'_>,
    inner: Rect,
    state: &ArgInputState,
    theme: &jinn_theme::Theme,
) {
    let input_y = inner.y.saturating_add(inner.height).saturating_sub(1);
    let prefix = Span::styled("> ", Style::default().fg(theme.focus_accent));
    let input = Span::raw(&state.text.input);
    frame.render_widget(
        Paragraph::new(Line::from(vec![prefix, input])),
        Rect::new(inner.x, input_y, inner.width, 1),
    );
    let cursor_x = 2u16
        .saturating_add(u16::try_from(state.text.graphemes_before_cursor()).unwrap_or(u16::MAX))
        .min(inner.width.saturating_sub(1));
    frame.set_cursor_position((inner.x.saturating_add(cursor_x), input_y));
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        reason = "test module, panics are acceptable"
    )]

    use super::*;
    use jinn_session_lifecycle_msg::CommandTemplate;
    use jinn_slices::Slices;
    use jinn_testutil::setup_term;

    #[rstest::rstest]
    fn rect_height_accounts_for_multiline_template() {
        // Given a two-line command template and a popup-sized terminal.
        let terminal_area = Rect::new(0, 0, 80, 24);
        let slices = Slices::new();
        let cell = slices
            .register(
                arg_input_slot(),
                ArgInputState::new(
                    "research".to_owned(),
                    CommandTemplate::parse("echo one && echo $1"),
                ),
            )
            .expect("fresh registry has an empty lifecycle argument slot");
        let expected = compute_popup_rect(terminal_area, content_rows(&cell.read()));

        // When computing the overlay geometry.
        let actual = arg_input_overlay_rect(terminal_area, &cell);

        // Then the geometry includes both command rows plus separator and input.
        assert_eq!(actual, expected);
        assert_eq!(expected.height, 6);
    }

    #[rstest::rstest]
    fn renders_template_arguments_and_input() {
        // Given a registered cell with a named argument already filled.
        let (mut terminal, area) = setup_term(80, 24);
        let slices = Slices::new();
        let cell = slices
            .register(
                arg_input_slot(),
                ArgInputState::new(
                    "research".to_owned(),
                    CommandTemplate::parse("mkdir <branch>"),
                ),
            )
            .expect("fresh registry has an empty lifecycle argument slot");
        cell.update(|state| state.text.set("my-feature".to_owned()));

        // When rendering the registered overlay view.
        terminal
            .draw(|frame| {
                let ctx = RenderFacts::new(jinn_theme::default_theme(), &slices);
                render_arg_input(frame, area, &ctx);
            })
            .expect("test terminal draw succeeds");

        // Then the substituted argument and editable input are visible.
        let text = buffer_text(&terminal);
        assert!(text.contains("mkdir my-feature"));
        assert!(text.contains("> my-feature"));
    }

    #[rstest::rstest]
    fn unfilled_parameter_uses_accent_color() {
        // Given a registered cell with an unfilled named parameter.
        let (mut terminal, area) = setup_term(80, 24);
        let slices = Slices::new();
        slices
            .register(
                arg_input_slot(),
                ArgInputState::new(
                    "research".to_owned(),
                    CommandTemplate::parse("mkdir <branch>"),
                ),
            )
            .expect("fresh registry has an empty lifecycle argument slot");

        // When rendering the overlay.
        terminal
            .draw(|frame| {
                let ctx = RenderFacts::new(jinn_theme::default_theme(), &slices);
                render_arg_input(frame, area, &ctx);
            })
            .expect("test terminal draw succeeds");

        // Then the placeholder is styled with the action accent.
        let buffer = terminal.backend().buffer().clone();
        let theme = jinn_theme::default_theme();
        assert!(
            buffer
                .content()
                .iter()
                .any(|cell| { cell.symbol() == "<" && cell.fg == theme.accent_action })
        );
    }

    #[rstest::rstest]
    fn empty_template_renders_minimal_popup() {
        // Given the inert registered state that exists before picker selection.
        let (mut terminal, area) = setup_term(80, 24);
        let slices = Slices::new();
        slices
            .register(arg_input_slot(), ArgInputState::empty())
            .expect("fresh registry has an empty lifecycle argument slot");

        // When rendering the overlay.
        terminal
            .draw(|frame| {
                let ctx = RenderFacts::new(jinn_theme::default_theme(), &slices);
                render_arg_input(frame, area, &ctx);
            })
            .expect("test terminal draw succeeds");

        // Then the popup still shows the stable title.
        let text = buffer_text(&terminal);
        assert!(text.contains("New Session (set script args)"));
    }

    fn buffer_text(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }
}
