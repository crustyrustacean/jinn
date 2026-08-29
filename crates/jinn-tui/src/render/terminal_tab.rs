//! Terminal tab rendering — mirrors the active `interactive_term` session.
//!
//! Draws the actor-mirrored screen ([`TerminalTabState`]) into the tab's
//! content rect: plain-text rows as a [`Paragraph`], plus the program's
//! cursor position when it is visible. Colors and attributes render from the
//! emulator's styled cells when a live session can be queried; the plain-text
//! path keeps the tab useful when only the text mirror is available (e.g.
//! right after takeover before the next drain).

use jinn_domain::RenderCtx;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Renders the terminal tab into `area` (the content rect of the tab).
pub fn render_terminal_tab(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx<'_>) {
    let terminal = &ctx.state.frontend.terminal;
    let theme = &ctx.state.frontend.theme;

    let Some(_session_id) = terminal.session_id.as_deref() else {
        render_empty(frame, area, theme.focus_accent);
        return;
    };

    let text = terminal.screen();
    if text.trim().is_empty() {
        render_empty(frame, area, theme.focus_accent);
        return;
    }

    let lines: Vec<Line<'_>> = text
        .lines()
        .map(|row| Line::from(Span::raw(row.to_owned())))
        .collect();

    let para = Paragraph::new(lines);
    frame.render_widget(para, area);

    // Cursor: only when the program shows it (TUIs hide it while repainting)
    // and the cursor is inside the visible area.
    if !terminal.cursor_hidden {
        let (row, col) = terminal.cursor;
        let x = area.x.saturating_add(col);
        let y = area.y.saturating_add(row);
        if col < area.width && row < area.height {
            frame.set_cursor_position((x, y));
        }
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
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn app_with_terminal_screen(screen: &str, cursor: (u16, u16)) -> AppState {
        let mut state = AppState::default();
        state
            .frontend
            .scope_stack
            .swap_base(FocusScope::TerminalView);
        state
            .frontend
            .terminal
            .apply_screen("term-1", screen.to_owned(), cursor, false);
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
}
