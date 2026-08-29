//! Terminal screen rendering — mirrors an `interactive_term` session.
//!
//! Draws the actor-mirrored screen ([`TerminalTabState`]) into a rect: the
//! styled cell grid (colors, attributes, wide characters) when available,
//! falling back to the plain-text rows when a mirror predates the cells.
//! The program's cursor is drawn only when the program shows it (TUIs hide
//! it while repainting).
//!
//! [`TerminalTabState`]: jinn_domain::feat::interactive_term::terminal_tab_state::TerminalTabState

use jinn_domain::RenderCtx;
use jinn_domain::feat::interactive_term::emulator::{TermCell, TermColor};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Renders the active session's terminal screen into `area`.
pub fn render_terminal_tab(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx<'_>) {
    let terminal = &ctx.state.frontend.terminal;
    let theme = &ctx.state.frontend.theme;

    // The view shows the active chat session's mirrored terminal.
    let chat = ctx.state.session.active_session_id().clone();
    let Some(mirror) = terminal.mirror(&chat) else {
        render_empty(frame, area, theme.focus_accent);
        return;
    };

    if mirror.cells.cells.is_empty() {
        render_plain_text(frame, area, &mirror.screen);
    } else {
        render_cells(frame, area, &mirror.cells);
    }

    // Cursor: only when the program shows it (TUIs hide it while repainting)
    // and the cursor is inside the visible area.
    if !mirror.cursor_hidden {
        let (row, col) = mirror.cursor;
        let x = area.x.saturating_add(col);
        let y = area.y.saturating_add(row);
        if col < area.width && row < area.height {
            frame.set_cursor_position((x, y));
        }
    }
}

/// Draws the styled cell grid cell-by-cell (colors and attributes).
///
/// `Blank` cells are left untouched; [`TermCell::WideSpacer`] cells are
/// skipped — the wide `ch` already occupies the leading slot and ratatui's
/// buffer advances past the second column on its own.
fn render_cells(
    frame: &mut Frame<'_>,
    area: Rect,
    cells: &jinn_domain::feat::interactive_term::emulator::ScreenCells,
) {
    let buf = frame.buffer_mut();
    for row in 0..cells.rows.min(area.height) {
        for col in 0..cells.cols.min(area.width) {
            let Some(TermCell::Styled { ch, style }) = cells.get(row, col) else {
                continue;
            };
            let x = area.x + col;
            let y = area.y + row;
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(ch.encode_utf8(&mut [0u8; 4]));
                cell.set_style(to_ratatui_style(style));
            }
        }
    }
}

/// Draws plain-text rows when no cell grid is mirrored yet.
fn render_plain_text(frame: &mut Frame<'_>, area: Rect, text: &str) {
    if text.trim().is_empty() {
        let theme_hint = Color::Indexed(8);
        render_empty(frame, area, theme_hint);
        return;
    }
    let lines: Vec<Line<'_>> = text
        .lines()
        .map(|row| Line::from(Span::raw(row.to_owned())))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// Maps the emulator's cell style to a ratatui style.
fn to_ratatui_style(style: &jinn_domain::feat::interactive_term::emulator::CellStyle) -> Style {
    let mut out = Style::default();
    if let Some(fg) = to_ratatui_color(style.fg) {
        out = out.fg(fg);
    }
    if let Some(bg) = to_ratatui_color(style.bg) {
        out = out.bg(bg);
    }
    let mut mods = Modifier::empty();
    if style.bold {
        mods |= Modifier::BOLD;
    }
    if style.italic {
        mods |= Modifier::ITALIC;
    }
    if style.underline {
        mods |= Modifier::UNDERLINED;
    }
    if style.inverse {
        mods |= Modifier::REVERSED;
    }
    out.add_modifier(mods)
}

/// Maps a terminal color; `None` leaves ratatui's default (`Reset`).
fn to_ratatui_color(color: TermColor) -> Option<Color> {
    match color {
        TermColor::Default => None,
        TermColor::Idx(i) => Some(Color::Indexed(i)),
        TermColor::Rgb(r, g, b) => Some(Color::Rgb(r, g, b)),
    }
}

/// Draws a hint line when there is no session or the screen is blank.
fn render_empty(frame: &mut Frame<'_>, area: Rect, accent: ratatui::style::Color) {
    let hint = Paragraph::new(Line::from(Span::styled(
        "no active terminal session — ask the agent to run `interactive_term`",
        Style::default().fg(accent).add_modifier(Modifier::ITALIC),
    )));
    frame.render_widget(hint, area);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use super::*;
    use jinn_domain::common::app_state::{AppState, FocusScope};
    use jinn_domain::feat::interactive_term::emulator::ScreenCells;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Position;

    fn app_with_terminal_screen(screen: &str, cursor: (u16, u16)) -> AppState {
        let mut state = AppState::default();
        state
            .frontend
            .scope_stack
            .swap_base(FocusScope::TerminalView);
        state.frontend.terminal.apply_screen(
            state.session.active_session_id(),
            "term-1",
            screen.to_owned(),
            ScreenCells::default(),
            cursor,
            false,
        );
        state
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn renders_screen_text_into_buffer() {
        // Given an app with a mirrored terminal screen.
        let state = app_with_terminal_screen("hello from vim", (0, 0));
        let app = crate::TuiApp::test_builder().state(state).build().await;

        // When rendering on a test backend.
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let area = Rect::new(0, 0, 80, 24);
        let guard = app.core.state.read();
        let ctx = RenderCtx::new(&guard);
        terminal
            .draw(|f| render_terminal_tab(f, area, &ctx))
            .expect("draw");

        // Then the buffer contains the screen text.
        let buffer = terminal.backend().buffer();
        let row: String = (0..15)
            .map(|x| buffer[(x, 0)].symbol().to_owned())
            .collect();
        assert!(row.contains("hello from vim"), "row was: {row:?}");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn renders_hint_when_no_session() {
        // Given an app with no mirrored session.
        let mut state = AppState::default();
        state
            .frontend
            .scope_stack
            .swap_base(FocusScope::TerminalView);
        let app = crate::TuiApp::test_builder().state(state).build().await;

        // When rendering the terminal tab.
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let area = Rect::new(0, 0, 80, 24);
        let guard = app.core.state.read();
        let ctx = RenderCtx::new(&guard);
        terminal
            .draw(|f| render_terminal_tab(f, area, &ctx))
            .expect("draw");

        // Then the buffer shows the empty-session hint.
        let buffer = terminal.backend().buffer();
        let row: String = (0..60)
            .map(|x| buffer[(x, 0)].symbol().to_owned())
            .collect();
        assert!(row.contains("no active terminal session"), "row: {row:?}");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn renders_styled_cells_with_colors_and_attributes() {
        // Given an app whose mirror carries a styled cell grid: red bold "R"
        // followed by default-colored plain text.
        let mut state = AppState::default();
        state
            .frontend
            .scope_stack
            .swap_base(FocusScope::TerminalView);
        let styled = {
            use jinn_domain::feat::interactive_term::emulator::{
                CellStyle, ScreenCells, TermCell, TermColor,
            };
            let mut cells = vec![
                TermCell::Styled {
                    ch: 'R',
                    style: CellStyle {
                        fg: TermColor::Idx(1),
                        bold: true,
                        ..CellStyle::default()
                    },
                },
                TermCell::Styled {
                    ch: 'x',
                    style: CellStyle::default(),
                },
            ];
            cells.resize(200, TermCell::Blank);
            ScreenCells {
                rows: 24,
                cols: 80,
                cells,
            }
        };
        {
            let mut term = state.frontend.terminal.clone();
            term.apply_screen(
                state.session.active_session_id(),
                "term-1",
                "Rx".to_owned(),
                styled,
                (0, 2),
                false,
            );
            state.frontend.terminal = term;
        }
        let app = crate::TuiApp::test_builder().state(state).build().await;

        // When rendering on a test backend.
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let area = Rect::new(0, 0, 80, 24);
        let guard = app.core.state.read();
        let ctx = RenderCtx::new(&guard);
        terminal
            .draw(|f| render_terminal_tab(f, area, &ctx))
            .expect("draw");

        // Then the styled cell carries the red bold style and the plain cell
        // carries no modifiers.
        let buffer = terminal.backend().buffer();
        let red = buffer[(0, 0)].clone();
        assert_eq!(red.symbol(), "R");
        assert_eq!(red.fg, Color::Indexed(1));
        assert!(red.modifier.contains(ratatui::style::Modifier::BOLD));
        let plain = buffer[(1, 0)].clone();
        assert_eq!(plain.symbol(), "x");
        assert_eq!(plain.modifier, ratatui::style::Modifier::empty());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn wide_char_spacer_cells_are_skipped_not_rendered() {
        // Given a mirror whose grid contains a wide char followed by its
        // spacer (as the emulator emits for double-width glyphs).
        let mut state = AppState::default();
        state
            .frontend
            .scope_stack
            .swap_base(FocusScope::TerminalView);
        let styled = {
            use jinn_domain::feat::interactive_term::emulator::{CellStyle, ScreenCells, TermCell};
            let mut cells = vec![
                TermCell::Styled {
                    ch: '漢',
                    style: CellStyle::default(),
                },
                TermCell::WideSpacer,
            ];
            cells.resize(200, TermCell::Blank);
            ScreenCells {
                rows: 24,
                cols: 80,
                cells,
            }
        };
        {
            let mut term = state.frontend.terminal.clone();
            term.apply_screen(
                state.session.active_session_id(),
                "term-1",
                "漢".to_owned(),
                styled,
                (0, 2),
                false,
            );
            state.frontend.terminal = term;
        }
        let app = crate::TuiApp::test_builder().state(state).build().await;

        // When rendering on a test backend.
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let area = Rect::new(0, 0, 80, 24);
        let guard = app.core.state.read();
        let ctx = RenderCtx::new(&guard);
        terminal
            .draw(|f| render_terminal_tab(f, area, &ctx))
            .expect("draw");

        // Then the wide glyph renders at column 0 and the spacer column was
        // not overwritten with a symbol (skipped; buffer default remains).
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].symbol(), "漢");
        assert_eq!(buffer[(1, 0)].symbol(), " ");
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn cursor_position_is_set_when_visible_and_skipped_when_hidden() {
        // Given a mirror with an unhidden cursor at (1, 3).
        let mut state = AppState::default();
        state
            .frontend
            .scope_stack
            .swap_base(FocusScope::TerminalView);
        {
            let mut term = state.frontend.terminal.clone();
            term.apply_screen(
                state.session.active_session_id(),
                "term-1",
                "hello".to_owned(),
                ScreenCells::default(),
                (1, 3),
                false,
            );
            state.frontend.terminal = term;
        }
        let app = crate::TuiApp::test_builder().state(state).build().await;

        // When rendering.
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let area = Rect::new(0, 0, 80, 24);
        let guard = app.core.state.read();
        let ctx = RenderCtx::new(&guard);
        terminal
            .draw(|f| render_terminal_tab(f, area, &ctx))
            .expect("draw");

        // Then the cursor sits at (1, 3).
        assert_eq!(terminal.backend().cursor_position(), Position::from((3, 1)));
    }
}
