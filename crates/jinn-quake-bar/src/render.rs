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
//! - session data row: applied auto-prune token total
//! - session data row: count of entries queued for prune
//! - bright divider
//! - command log (0..20 rows, viewport capped at 10)
//! - muted divider (a single `-`)
//! - input row: `> {text}` with a yellow `>` (focus accent)
//! - command log (0..20 rows)
//! - muted divider
//! - input row: `> {text}` with a yellow `>` (focus accent)
//! - bright divider

use jinn_slices::RenderFacts;
use jinn_theme::contrast::lighten;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_segmentation::UnicodeSegmentation;

use crate::state::QuakeBarState;

/// How much to lighten `quake_bar_bg` for the bright divider lines.
///
/// `> 1.0` brightens toward white; the dividers track the base color on demand
/// rather than living in the theme.
const DIVIDER_LIGHTEN_FACTOR: f32 = 2.0;

/// Fixed rows that are always present regardless of log size:
/// header, session data (prune pruned), session data (prune pending),
/// lifecycle data, bright divider, muted divider, input.
/// (The bottom bright divider was removed — the background color
/// contrast alone separates the bar from content below.)
/// Fixed rows that are always present regardless of log size:
/// header, bright divider, muted divider, input row, and the bottom
/// bright divider. (The session-fact rows were folded out with the
/// crate cut.)
const FIXED_ROWS: u16 = 7;

/// Maximum rows the command log viewport can occupy, regardless of
/// terminal height. Capping the viewport (rather than letting it grow
/// with the terminal) guarantees scroll is meaningful: on tall terminals
/// the log region stays bounded so PgUp/PgDn reveal older entries
/// instead of showing all 20 lines at once (which would make scroll a
/// permanent no-op).
const LOG_VIEWPORT_MAX: u16 = 10;

/// The yellow `>` prefix length (in cells) on the input row.
const INPUT_PREFIX_CELLS: u16 = 2;

/// Renders the quake bar as a full-width overlay at the top of `area`.
///
/// Reads the slice's cell through the render context's slices registry;
/// the cell is seeded by `quake_state_with_*` test helpers and, in the
/// app, by the slice's activation.
///
/// # Panics
///
/// Panics if the quake bar's cell was never registered — a broken
/// activation; the overlay cannot render without its payload.
pub fn render_quake_bar(frame: &mut Frame<'_>, area: Rect, ctx: &RenderFacts) {
    let theme = &ctx.theme;
    #[expect(
        clippy::expect_used,
        reason = "the overlay only renders when the quake scope registered its cell"
    )]
    let quake = ctx
        .slices
        .reader::<QuakeBarState>(&crate::state::quake_bar_slot())
        .expect("quake-bar overlay renders only when its cell is registered");
    let quake = quake.read();

    let bg = theme.quake_bar_bg;
    let bright = lighten(bg, DIVIDER_LIGHTEN_FACTOR);
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

    // Session-fact rows were folded out with the crate cut: the
    // pending-prune, pruned-total, and lifecycle lines read AppState
    // (kernel state). Re-adding them requires an application-facts
    // render context — see the playwright notes in the migration plan.

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

    // Muted divider: a single '-' above the input row (literal one cell).
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "-".to_owned(),
            Style::default().fg(theme.muted_text).bg(bg),
        )))
        .style(bg_style),
        single_row(quake_area, y),
    );
    y += 1;

    // Input row: yellow "> " prefix + editable text + live cursor.
    render_input_row(
        frame,
        quake_area,
        y,
        &quake,
        bg,
        theme.focus_accent,
        theme.primary_text,
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

/// Produces spans for a label centered within `width` cells, padded with `-`.
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
        Span::styled("-".repeat(left), Style::default().fg(sep_fg).bg(bg)),
        Span::styled(label.to_owned(), Style::default().fg(label_fg).bg(bg)),
        Span::styled("-".repeat(right), Style::default().fg(sep_fg).bg(bg)),
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

/// The overlay's total height for a terminal of `height`: fixed rows
/// plus up to [`LOG_VIEWPORT_MAX`] log rows, clamped to the terminal.
#[must_use]
pub fn quake_overlay_height(height: u16) -> u16 {
    (FIXED_ROWS + LOG_VIEWPORT_MAX).min(height)
}

/// Log rows available after the [`FIXED_ROWS`] are accounted for,
/// capped at [`LOG_VIEWPORT_MAX`] so the region stays bounded on tall
/// terminals.
fn available_log_rows(height: u16) -> usize {
    height.saturating_sub(FIXED_ROWS).min(LOG_VIEWPORT_MAX) as usize
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

    use jinn_slices::RenderFacts;
    use jinn_slices::Slices;
    use jinn_slices::TypedCell;
    use jinn_testutil::setup_term;

    /// Test wiring: slices registry with the quake cell seeded from the
    /// given log lines + input. Returns (state, slices) — the render fn
    /// resolves the cell through the ctx's slices.
    fn quake_ctx_with(lines: &[&str], input: &str) -> (Slices, TypedCell<QuakeBarState>) {
        let slices = Slices::new();
        let cell = slices
            .register(crate::state::quake_bar_slot(), QuakeBarState::default())
            .expect("fresh registry");
        cell.update(|s| {
            for line in lines {
                s.log.push((*line).to_owned());
            }
            s.input.text.input = input.to_owned();
            s.input.text.cursor_pos = input.len();
        });
        (slices, cell)
    }

    fn facts_for(slices: &Slices) -> RenderFacts {
        RenderFacts::new(jinn_theme::default_theme(), slices)
    }

    fn quake_state_with_log(lines: &[&str]) -> (Slices, TypedCell<QuakeBarState>) {
        quake_ctx_with(lines, "")
    }

    fn quake_state_with_input(input: &str) -> Slices {
        quake_ctx_with(&[], input).0
    }

    #[rstest::rstest]
    #[test]
    fn header_session_label_is_primary_text() {
        // Given a quake-bar state.
        let (slices, _cell) = quake_state_with_log(&[]);
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                let facts = facts_for(&slices);
                render_quake_bar(frame, area, &facts);
            })
            .unwrap();

        // Then the "Session" header cells use primary_text foreground.
        let buffer = terminal.backend().buffer().clone();
        let primary = facts_for(&slices).theme.primary_text;
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

    #[rstest::rstest]
    #[test]
    fn header_separator_equals_are_muted_text() {
        // Given a quake-bar state.
        let (slices, _cell) = quake_state_with_log(&[]);
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                let facts = facts_for(&slices);
                render_quake_bar(frame, area, &facts);
            })
            .unwrap();

        // Then a "-" separator cell uses muted_text foreground.
        let buffer = terminal.backend().buffer().clone();
        let muted = facts_for(&slices).theme.muted_text;
        let header_y = area.y;
        let mut found = false;
        for x in area.x..area.x + area.width {
            if let Some(cell) = buffer.cell((x, header_y))
                && cell.symbol() == "-"
                && cell.style().fg == Some(muted)
            {
                found = true;
            }
        }
        assert!(found, "'-' separators should be muted_text");
    }

    #[rstest::rstest]
    #[test]
    fn input_prefix_is_focus_accent_yellow() {
        // Given a quake-bar state with some input.
        let slices = quake_state_with_input("hi");
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                let facts = facts_for(&slices);
                render_quake_bar(frame, area, &facts);
            })
            .unwrap();

        // Then the ">" prefix cell uses focus_accent.
        let buffer = terminal.backend().buffer().clone();
        let focus = jinn_theme::default_theme().focus_accent;
        // The input row sits after header(1)+bright(1)+log(0)+muted(1) = 3 rows.
        let input_y = area.y + 3;
        let prefix_cell = buffer.cell((area.x, input_y)).expect("prefix cell");
        assert_eq!(prefix_cell.symbol(), ">");
        assert_eq!(
            prefix_cell.style().fg,
            Some(focus),
            "'>' prefix should be focus_accent"
        );
    }

    #[rstest::rstest]
    #[test]
    fn bright_divider_uses_lightened_background() {
        // Given a quake-bar state.
        let (slices, _cell) = quake_state_with_log(&[]);
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                let facts = facts_for(&slices);
                render_quake_bar(frame, area, &facts);
            })
            .unwrap();

        // Then the bright divider (row 1) uses lighten(quake_bar_bg) foreground.
        let buffer = terminal.backend().buffer().clone();
        let bg = jinn_theme::default_theme().quake_bar_bg;
        let expected_bright = jinn_theme::contrast::lighten(bg, DIVIDER_LIGHTEN_FACTOR);
        let bright_y = area.y + 1;
        let cell = buffer.cell((area.x + 5, bright_y)).expect("divider cell");
        assert_eq!(
            cell.style().fg,
            Some(expected_bright),
            "bright divider should use lighten(quake_bar_bg)"
        );
    }

    #[rstest::rstest]
    #[test]
    fn muted_divider_uses_muted_text() {
        // Given a quake-bar state.
        let (slices, _cell) = quake_state_with_log(&[]);
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                let facts = facts_for(&slices);
                render_quake_bar(frame, area, &facts);
            })
            .unwrap();

        // Then the muted divider (row 2, empty log) uses muted_text
        // foreground.
        let buffer = terminal.backend().buffer().clone();
        let muted = facts_for(&slices).theme.muted_text;
        let muted_y = area.y + 2;
        let cell = buffer.cell((area.x, muted_y)).expect("divider cell");
        assert_eq!(
            cell.style().fg,
            Some(muted),
            "muted divider should use muted_text"
        );
    }

    #[rstest::rstest]
    #[test]
    fn overlay_spans_full_width_with_quake_bar_background() {
        // Given a quake-bar state.
        let (slices, _cell) = quake_state_with_log(&[]);
        let (mut terminal, area) = setup_term(60, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                let facts = facts_for(&slices);
                render_quake_bar(frame, area, &facts);
            })
            .unwrap();

        // Then the leftmost and rightmost header cells carry the quake_bar_bg.
        let buffer = terminal.backend().buffer().clone();
        let bg = jinn_theme::default_theme().quake_bar_bg;
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

    #[rstest::rstest]
    #[test]
    fn command_log_lines_render_below_bright_divider() {
        // Given a quake-bar state with a logged line.
        let (slices, _cell) = quake_state_with_log(&["hello world"]);
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                let facts = facts_for(&slices);
                render_quake_bar(frame, area, &facts);
            })
            .unwrap();

        // Then the log row (row 2: header, bright, then the first log
        // line) contains the logged text.
        let buffer = terminal.backend().buffer().clone();
        let log_y = area.y + 2;
        let symbols: String = (area.x..area.x + area.width)
            .filter_map(|x| buffer.cell((x, log_y)).map(|c| c.symbol().to_owned()))
            .collect();
        assert!(
            symbols.contains("hello world"),
            "logged line should render in the log region"
        );
    }

    #[rstest::rstest]
    #[test]
    fn scroll_up_changes_which_log_line_is_visible() {
        // Given a quake bar with a full 20-line log, pinned to the bottom.
        let (slices, cell) = quake_state_with_log(&[]);
        for i in 0..20 {
            let line = format!("line-{i}");
            cell.update(|s| s.log.push(line));
        }
        // Short terminal so the log viewport (height 12 - FIXED_ROWS 7 = 5)
        // is smaller than the 20-line log, making scroll observable.
        let (mut terminal, area) = setup_term(80, 12);

        // Snapshot the last visible log line before scrolling.
        terminal
            .draw(|frame| {
                let facts = facts_for(&slices);
                render_quake_bar(frame, area, &facts);
            })
            .unwrap();
        let before = terminal.backend().buffer().clone();
        // Last log row = header(1) + bright divider(1) + (viewport-1),
        // where viewport = height - FIXED_ROWS.
        let log_rows = (area.height.saturating_sub(FIXED_ROWS)) as usize;
        let last_log_y = area.y + 2 + log_rows.saturating_sub(1) as u16;
        let newest_before: String = (area.x..area.x + area.width)
            .filter_map(|x| before.cell((x, last_log_y)).map(|c| c.symbol().to_owned()))
            .collect();

        // When scrolling up once.
        {
            let cell = slices
                .reader::<QuakeBarState>(&crate::state::quake_bar_slot())
                .expect("seeded");
            cell.update(|s| s.log.scroll_up());
        }

        // Then the rendered bottom log line changes (the window moved up).
        terminal
            .draw(|frame| {
                let facts = facts_for(&slices);
                render_quake_bar(frame, area, &facts);
            })
            .unwrap();
        let after = terminal.backend().buffer().clone();
        let newest_after: String = (area.x..area.x + area.width)
            .filter_map(|x| after.cell((x, last_log_y)).map(|c| c.symbol().to_owned()))
            .collect();
        assert_ne!(
            newest_before, newest_after,
            "scroll_up must change the rendered log window"
        );
    }
}
