//! Render for the quake bar overlay.
//!
//! A full-width drop-down console pinned to the top of the screen. It overlays
//! whatever is below (rendered last in the render tree, over a [`Clear`]). There
//! are no side or top borders — the two bright dividers (`lighten(quake_bar_bg)`)
//! and one muted divider frame the sections internally.
//!
//! Layout, top to bottom:
//! - header: centered "Session" (left half) `|` "Global" (right half),
//!   `=` separators in muted text
//! - session data row: count of entries queued for prune
//! - bright divider
//! - command log (0..20 rows)
//! - muted divider
//! - input row: `> {text}` with a yellow `>` (focus accent)
//! - bright divider

use crate::common::render_ctx::RenderCtx;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_segmentation::UnicodeSegmentation;

use super::state::QuakeBarState;

/// How much to lighten `quake_bar_bg` for the bright divider lines.
///
/// `> 1.0` brightens toward white; the dividers track the base color on demand
/// rather than living in the theme.
const DIVIDER_LIGHTEN_FACTOR: f32 = 2.0;

/// Fixed rows that are always present regardless of log size:
/// header, session data, bright divider, muted divider, input, bright divider.
const FIXED_ROWS: u16 = 6;

/// The yellow `>` prefix length (in cells) on the input row.
const INPUT_PREFIX_CELLS: u16 = 2;

/// Renders the quake bar as a full-width overlay at the top of `area`.
pub fn render_quake_bar(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    let state = ctx.state;
    let quake = &state.frontend.quake_bar;
    let theme = &state.frontend.theme;

    let bg = theme.quake_bar_bg;
    let bright = crate::feat::theme::contrast::lighten(bg, DIVIDER_LIGHTEN_FACTOR);
    let bg_style = Style::default().bg(bg);

    // Log viewport: as many rows as fit, capped implicitly by the log's 20-line max.
    let log_viewport = available_log_rows(area.height);
    let visible_log = quake.log.visible_lines(log_viewport);

    let total_height = FIXED_ROWS
        .saturating_add(visible_log.len() as u16)
        .min(area.height);
    let quake_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: total_height,
    };

    frame.render_widget(Clear, quake_area);

    let mut y = quake_area.y;

    // Header: centered Session (left) | Global (right).
    y = render_header(
        frame,
        quake_area,
        y,
        bg,
        theme.primary_text,
        theme.muted_text,
    );

    // Session data row: count of distinct entries pending prune (shield/compaction never reach the buffer).
    let pending = state.active_session().accumulated_prune_count();
    let data = Line::from(Span::styled(
        format!("Prune candidates queued: {pending}"),
        Style::default().fg(theme.primary_text).bg(bg),
    ));
    frame.render_widget(
        Paragraph::new(data).style(bg_style),
        single_row(quake_area, y),
    );
    y += 1;

    // Bright divider.
    frame.render_widget(
        Paragraph::new(divider_line(quake_area.width, '─', bright, bg)).style(bg_style),
        single_row(quake_area, y),
    );
    y += 1;

    // Command log rows.
    for line in visible_log {
        let entry = Line::from(Span::styled(
            line.clone(),
            Style::default().fg(theme.primary_text).bg(bg),
        ));
        frame.render_widget(
            Paragraph::new(entry).style(bg_style),
            single_row(quake_area, y),
        );
        y += 1;
    }

    // Muted divider.
    frame.render_widget(
        Paragraph::new(divider_line(quake_area.width, '-', theme.muted_text, bg)).style(bg_style),
        single_row(quake_area, y),
    );
    y += 1;

    // Input row: yellow "> " prefix + editable text + live cursor.
    y = render_input_row(
        frame,
        quake_area,
        y,
        quake,
        bg,
        theme.focus_accent,
        theme.primary_text,
    );

    // Final bright divider.
    frame.render_widget(
        Paragraph::new(divider_line(quake_area.width, '─', bright, bg)).style(bg_style),
        single_row(quake_area, y),
    );
}

/// Renders the header row, returning the y of the next row.
fn render_header(
    frame: &mut Frame<'_>,
    area: Rect,
    y: u16,
    bg: Color,
    primary: Color,
    muted: Color,
) -> u16 {
    let line = header_line(area.width, primary, muted, bg);
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(bg)),
        single_row(area, y),
    );
    y + 1
}

/// Renders the input row and positions the text cursor, returning the next y.
fn render_input_row(
    frame: &mut Frame<'_>,
    area: Rect,
    y: u16,
    quake: &QuakeBarState,
    bg: Color,
    focus: Color,
    primary: Color,
) -> u16 {
    let text = &quake.input.text.input;
    let spans = vec![
        Span::styled("> ", Style::default().fg(focus).bg(bg)),
        Span::styled(text.clone(), Style::default().fg(primary).bg(bg)),
    ];
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(bg)),
        single_row(area, y),
    );

    let graphemes_before = text
        .get(..quake.input.text.cursor_pos)
        .map_or(0, |s| s.graphemes(true).count());
    let cursor_x = INPUT_PREFIX_CELLS
        .saturating_add(graphemes_before as u16)
        .min(area.width.saturating_sub(1));
    frame.set_cursor_position((area.x.saturating_add(cursor_x), y));

    y + 1
}

/// Builds the header: "Session" centered in the left half (with `=` separators),
/// a `|`, then "Global" centered in the right half.
fn header_line(width: u16, primary: Color, muted: Color, bg: Color) -> Line<'static> {
    let w = width as usize;
    // Reserve 1 cell for the "|" separator between halves.
    let left_half = w / 2;
    let right_half = w.saturating_sub(left_half).saturating_sub(1);

    let mut spans = centered_label("Session", left_half, muted, primary, bg);
    spans.push(Span::styled(
        "|".to_owned(),
        Style::default().fg(muted).bg(bg),
    ));
    spans.extend(centered_label("Global", right_half, muted, primary, bg));
    Line::from(spans)
}

/// Produces spans for a label centered within `width` cells, padded with `=`.
fn centered_label(
    label: &str,
    width: usize,
    sep_fg: Color,
    label_fg: Color,
    bg: Color,
) -> Vec<Span<'static>> {
    let label_len = label.chars().count();
    let total_sep = width.saturating_sub(label_len);
    let left = total_sep / 2;
    let right = total_sep - left;
    vec![
        Span::styled("=".repeat(left), Style::default().fg(sep_fg).bg(bg)),
        Span::styled(label.to_owned(), Style::default().fg(label_fg).bg(bg)),
        Span::styled("=".repeat(right), Style::default().fg(sep_fg).bg(bg)),
    ]
}

/// Builds a full-width divider line of repeated `ch`, styled with `fg`/`bg`.
fn divider_line(width: u16, ch: char, fg: Color, bg: Color) -> Line<'static> {
    let text: String = std::iter::repeat_n(ch, usize::from(width)).collect();
    Line::from(Span::styled(text, Style::default().fg(fg).bg(bg)))
}

/// Returns the single-row rect at `y` spanning the full quake bar width.
fn single_row(area: Rect, y: u16) -> Rect {
    Rect {
        x: area.x,
        y,
        width: area.width,
        height: 1,
    }
}

/// Log rows available after the [`FIXED_ROWS`] are accounted for.
fn available_log_rows(height: u16) -> usize {
    height.saturating_sub(FIXED_ROWS) as usize
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

    use super::*;
    use crate::common::app_state::{AppState, FocusScope};
    use jinn_testutil::setup_term;

    fn quake_state_with_log(lines: &[&str]) -> AppState {
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::QuakeBar);
        for line in lines {
            state.frontend.quake_bar.log.push((*line).to_owned());
        }
        state
    }

    fn quake_state_with_input(input: &str) -> AppState {
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::QuakeBar);
        state.frontend.quake_bar.input.text.input = input.to_owned();
        state.frontend.quake_bar.input.text.cursor_pos = input.len();
        state
    }

    #[test]
    fn header_session_label_is_primary_text() {
        // Given a quake-bar state.
        let state = quake_state_with_log(&[]);
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                let ctx = RenderCtx::new(&state);
                render_quake_bar(frame, area, &ctx);
            })
            .unwrap();

        // Then the "Session" header cells use primary_text foreground.
        let buffer = terminal.backend().buffer().clone();
        let primary = state.frontend.theme.primary_text;
        let header_y = area.y;
        let mut found = false;
        for x in area.x..area.x + area.width {
            if let Some(cell) = buffer.cell((x, header_y))
                && cell.symbol() == "S"
                && cell.style().fg == Some(primary)
            {
                found = true;
            }
        }
        assert!(found, "Session header should be primary_text");
    }

    #[test]
    fn header_separator_equals_are_muted_text() {
        // Given a quake-bar state.
        let state = quake_state_with_log(&[]);
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                let ctx = RenderCtx::new(&state);
                render_quake_bar(frame, area, &ctx);
            })
            .unwrap();

        // Then a "=" separator cell uses muted_text foreground.
        let buffer = terminal.backend().buffer().clone();
        let muted = state.frontend.theme.muted_text;
        let header_y = area.y;
        let mut found = false;
        for x in area.x..area.x + area.width {
            if let Some(cell) = buffer.cell((x, header_y))
                && cell.symbol() == "="
                && cell.style().fg == Some(muted)
            {
                found = true;
            }
        }
        assert!(found, "'=' separators should be muted_text");
    }

    #[test]
    fn session_row_shows_prune_candidate_count() {
        // Given a quake-bar state.
        let state = quake_state_with_log(&[]);
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                let ctx = RenderCtx::new(&state);
                render_quake_bar(frame, area, &ctx);
            })
            .unwrap();

        // Then the data row contains the "Prune candidates queued" label.
        let buffer = terminal.backend().buffer().clone();
        let data_y = area.y + 1;
        let symbols: String = (area.x..area.x + area.width)
            .filter_map(|x| buffer.cell((x, data_y)).map(|c| c.symbol().to_owned()))
            .collect();
        assert!(
            symbols.contains("Prune candidates queued"),
            "session data row should show the prune-candidate count label"
        );
    }

    #[test]
    fn input_prefix_is_focus_accent_yellow() {
        // Given a quake-bar state with some input.
        let state = quake_state_with_input("hi");
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                let ctx = RenderCtx::new(&state);
                render_quake_bar(frame, area, &ctx);
            })
            .unwrap();

        // Then the ">" prefix cell uses focus_accent.
        let buffer = terminal.backend().buffer().clone();
        let focus = state.frontend.theme.focus_accent;
        // The input row sits after header(1)+data(1)+bright(1)+log(0)+muted(1) = 4 rows.
        let input_y = area.y + 4;
        let prefix_cell = buffer.cell((area.x, input_y)).expect("prefix cell");
        assert_eq!(prefix_cell.symbol(), ">");
        assert_eq!(
            prefix_cell.style().fg,
            Some(focus),
            "'>' prefix should be focus_accent"
        );
    }

    #[test]
    fn bright_divider_uses_lightened_background() {
        // Given a quake-bar state.
        let state = quake_state_with_log(&[]);
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                let ctx = RenderCtx::new(&state);
                render_quake_bar(frame, area, &ctx);
            })
            .unwrap();

        // Then the first bright divider (row 2) uses lighten(quake_bar_bg) foreground.
        let buffer = terminal.backend().buffer().clone();
        let bg = state.frontend.theme.quake_bar_bg;
        let expected_bright = crate::feat::theme::contrast::lighten(bg, DIVIDER_LIGHTEN_FACTOR);
        let bright_y = area.y + 2;
        let cell = buffer.cell((area.x + 5, bright_y)).expect("divider cell");
        assert_eq!(
            cell.style().fg,
            Some(expected_bright),
            "bright divider should use lighten(quake_bar_bg)"
        );
    }

    #[test]
    fn muted_divider_uses_muted_text() {
        // Given a quake-bar state.
        let state = quake_state_with_log(&[]);
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                let ctx = RenderCtx::new(&state);
                render_quake_bar(frame, area, &ctx);
            })
            .unwrap();

        // Then the muted divider (row 3, no log) uses muted_text foreground.
        let buffer = terminal.backend().buffer().clone();
        let muted = state.frontend.theme.muted_text;
        let muted_y = area.y + 3;
        let cell = buffer.cell((area.x + 5, muted_y)).expect("divider cell");
        assert_eq!(
            cell.style().fg,
            Some(muted),
            "muted divider should use muted_text"
        );
    }

    #[test]
    fn overlay_spans_full_width_with_quake_bar_background() {
        // Given a quake-bar state.
        let state = quake_state_with_log(&[]);
        let (mut terminal, area) = setup_term(60, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                let ctx = RenderCtx::new(&state);
                render_quake_bar(frame, area, &ctx);
            })
            .unwrap();

        // Then the leftmost and rightmost header cells carry the quake_bar_bg.
        let buffer = terminal.backend().buffer().clone();
        let bg = state.frontend.theme.quake_bar_bg;
        let header_y = area.y;
        let left = buffer.cell((area.x, header_y)).expect("left cell");
        let right = buffer
            .cell((area.x + area.width - 1, header_y))
            .expect("right cell");
        assert_eq!(
            left.style().bg,
            Some(bg),
            "left edge should be quake_bar_bg"
        );
        assert_eq!(
            right.style().bg,
            Some(bg),
            "right edge should be quake_bar_bg"
        );
    }

    #[test]
    fn command_log_lines_render_below_bright_divider() {
        // Given a quake-bar state with a logged line.
        let state = quake_state_with_log(&["hello world"]);
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                let ctx = RenderCtx::new(&state);
                render_quake_bar(frame, area, &ctx);
            })
            .unwrap();

        // Then the log row (row 3) contains the logged text.
        let buffer = terminal.backend().buffer().clone();
        let log_y = area.y + 3;
        let symbols: String = (area.x..area.x + area.width)
            .filter_map(|x| buffer.cell((x, log_y)).map(|c| c.symbol().to_owned()))
            .collect();
        assert!(
            symbols.contains("hello world"),
            "logged line should render in the log region"
        );
    }
}
