//! Renders the chat input prompt line.
//!
//! Shows the user's in-progress message below a `>` prompt. When the user is
//! actively typing (input mode), the prompt and border are highlighted in yellow and
//! the cursor appears at the current cursor position within the text. When browsing
//! (normal mode), the prompt is shown without highlighting and no cursor is displayed.
//!
//! Long lines are word-wrapped at the available width, with continuation lines indented
//! by two spaces. When the content exceeds the visible area, it scrolls to keep the
//! cursor visible.

use crate::common::app_state::AppState;
use crate::common::ui_element::UiElement;
use crate::feat::chat_input::state::wrap::WrappedLine;
use crate::protocol::Mode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_segmentation::UnicodeSegmentation;

/// Display element for the user's message composition area.
#[derive(Debug)]
pub struct ChatInputBoxElement;

impl UiElement<AppState> for ChatInputBoxElement {
    fn name(&self) -> String {
        "chat-input-box".to_owned()
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        let input_mode = state.frontend.scope_stack.current().mode() == Mode::Input;
        let theme = &state.frontend.theme;

        let prompt_style = if input_mode {
            Style::default()
                .fg(theme.focus_accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::BOLD)
        };

        let border_style = if input_mode {
            Style::default().fg(theme.focus_accent)
        } else {
            Style::default().fg(theme.border_unfocused)
        };

        let text_style = Style::default();

        let block = Block::default()
            .borders(Borders::BOTTOM)
            .border_style(border_style);
        let inner = block.inner(area);
        let max_visible_lines = inner.height as usize;

        let lines = build_wrapped_lines(
            state.active_chat_input().text(),
            &state.active_chat_input().wrapped_lines(),
            state.active_chat_input().scroll_offset(),
            max_visible_lines,
            prompt_style,
            text_style,
        );

        let input_widget = Paragraph::new(lines).block(block);
        frame.render_widget(input_widget, area);

        // Render scroll position indicators if content overflows.
        let total_lines = state.active_chat_input().wrapped_lines().len();
        let scroll_offset = state.active_chat_input().scroll_offset();
        render_scroll_indicators(
            frame,
            inner,
            total_lines,
            scroll_offset,
            max_visible_lines,
            theme.age_fresh,
            theme.scroll_indicator_bg,
        );

        // Position cursor when in input mode.
        if input_mode {
            let (row, col) = state.active_chat_input().cursor_row_col();
            let scroll_offset = state.active_chat_input().scroll_offset();
            let visual_row = row.saturating_sub(scroll_offset);
            let prefix_width: usize = 2; // "> " = 2 columns
            let cursor_x = inner.x + (prefix_width + col) as u16;
            let cursor_y = inner.y + visual_row as u16;
            frame.set_cursor_position((cursor_x, cursor_y));
        }
    }
}

/// Build visual lines from wrapped line data, applying scroll offset and visibility limit.
///
/// The first visual line gets a `> ` prompt prefix, all others get `  ` indentation.
fn build_wrapped_lines<'a>(
    text: &str,
    wrapped: &[WrappedLine],
    scroll_offset: usize,
    max_visible_lines: usize,
    prompt_style: Style,
    text_style: Style,
) -> Vec<Line<'a>> {
    if text.is_empty() {
        return vec![Line::from(vec![Span::styled("> ", prompt_style)])];
    }

    let graphemes: Vec<&str> = text.graphemes(true).collect();
    let mut lines = Vec::new();

    for (row, line) in wrapped.iter().enumerate() {
        if row < scroll_offset {
            continue;
        }
        if lines.len() >= max_visible_lines {
            break;
        }

        let prefix = if row == 0 { "> " } else { "  " };
        let content: String = graphemes[line.grapheme_start..line.grapheme_end].join("");
        lines.push(Line::from(vec![
            Span::styled(prefix, prompt_style),
            Span::styled(content, text_style),
        ]));
    }

    // If all lines were scrolled past, show at least the prompt.
    if lines.is_empty() {
        lines.push(Line::from(vec![Span::styled("> ", prompt_style)]));
    }

    lines
}

/// Render scroll position indicators when content exceeds the visible area.
///
/// Shows `↑ N` on the top-right when lines are hidden above, and `↓ N` on the
/// bottom-right when lines are hidden below. Styled like the chat log indicator
/// (dark gray on black).
#[expect(clippy::similar_names, reason = "fg/bg pair naming is intentional")]
fn render_scroll_indicators(
    frame: &mut Frame<'_>,
    inner: Rect,
    total_lines: usize,
    scroll_offset: usize,
    max_visible_lines: usize,
    indicator_fg: ratatui::style::Color,
    indicator_bg: ratatui::style::Color,
) {
    let lines_above = scroll_offset;
    let lines_below = total_lines
        .saturating_sub(scroll_offset)
        .saturating_sub(max_visible_lines);

    if lines_above == 0 && lines_below == 0 {
        return;
    }

    let style = Style::default().fg(indicator_fg).bg(indicator_bg);

    if lines_above > 0 {
        let label = format!("↑ {lines_above}");
        render_indicator_overlay(frame, &label, inner, inner.y, style);
    }

    if lines_below > 0 {
        let label = format!("↓ {lines_below}");
        let bottom_y = inner.y + inner.height.saturating_sub(1);
        render_indicator_overlay(frame, &label, inner, bottom_y, style);
    }
}

/// Render a single indicator label as a right-aligned overlay on the given row.
fn render_indicator_overlay(frame: &mut Frame<'_>, label: &str, inner: Rect, y: u16, style: Style) {
    let indicator_line = Line::from(Span::styled(label, style));
    let indicator_width = u16::try_from(indicator_line.width())
        .unwrap_or(inner.width)
        .min(inner.width);
    let indicator = Paragraph::new(indicator_line);
    let indicator_area = Rect {
        x: inner.x + inner.width.saturating_sub(indicator_width),
        y,
        width: indicator_width,
        height: 1,
    };
    frame.render_widget(indicator, indicator_area);
}

#[cfg(test)]
mod tests {
    use nullslop_testutil::setup_term;
    use ratatui::layout::Position;
    use super::*;
    use crate::common::app_state::FocusScope;
    use crate::feat::theme::default_theme;

    #[rstest::rstest]
    fn name_returns_chat_input_box() {
        // Given a ChatInputBoxElement.
        let element = ChatInputBoxElement;

        // When querying the name.
        let name = element.name();

        // Then it is "chat-input-box".
        assert_eq!(name, "chat-input-box");
    }

    #[rstest::rstest]
    fn render_draws_input_buffer() {
        // Given a ChatInputBoxElement with "hello" in state (Normal mode).
        let mut element = ChatInputBoxElement;
        let state = {
            let mut s = AppState::default();
            s.active_chat_input_mut().insert_text("hello");
            s
        };

        let (mut terminal, area) = setup_term(40, 3);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the buffer contains the ">" prompt character.
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((0, 0)).expect("cell should exist");
        assert_eq!(cell.symbol(), ">");
    }

    #[rstest::rstest]
    fn render_input_mode_yellow_prompt() {
        // Given a ChatInputBoxElement in Input mode with "hi" in buffer.
        let mut element = ChatInputBoxElement;
        let state = {
            let mut s = AppState::default();
            s.frontend.scope_stack.push(FocusScope::Input);
            s.active_chat_input_mut().insert_text("hi");
            s
        };

        let (mut terminal, area) = setup_term(40, 3);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the ">" prompt is yellow.
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((0, 0)).expect("cell should exist");
        assert_eq!(cell.symbol(), ">");
        assert_eq!(cell.style().fg, Some(default_theme().focus_accent));
    }

    #[rstest::rstest]
    fn render_input_mode_yellow_border() {
        // Given a ChatInputBoxElement in Input mode.
        let mut element = ChatInputBoxElement;
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Input);

        let (mut terminal, area) = setup_term(40, 3);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the bottom border is yellow.
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((0, 2)).expect("cell should exist");
        assert_eq!(cell.style().fg, Some(default_theme().focus_accent));
    }

    #[rstest::rstest]
    fn render_input_mode_cursor_at_end_of_text() {
        // Given a ChatInputBoxElement in Input mode with "abc" in buffer.
        let mut element = ChatInputBoxElement;
        let state = {
            let mut s = AppState::default();
            s.frontend.scope_stack.push(FocusScope::Input);
            s.active_chat_input_mut().insert_text("abc");
            s
        };

        let (mut terminal, area) = setup_term(40, 3);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then cursor is at position (5, 0): inner.x=0 + "> "=2 + "abc"=3.
        terminal
            .backend_mut()
            .assert_cursor_position(Position { x: 5, y: 0 });
    }

    #[rstest::rstest]
    fn render_cursor_at_mid_buffer() {
        // Given a ChatInputBoxElement in Input mode with "abc" and cursor at position 1.
        let mut element = ChatInputBoxElement;
        let state = {
            let mut s = AppState::default();
            s.frontend.scope_stack.push(FocusScope::Input);
            s.active_chat_input_mut().insert_text("abc");
            s.active_chat_input_mut().move_cursor_to_start();
            s.active_chat_input_mut().move_cursor_right(); // cursor at 1 (between 'a' and 'b')
            s
        };

        let (mut terminal, area) = setup_term(40, 3);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then cursor is at position (3, 0): inner.x=0 + "> "=2 + cursor_pos=1.
        terminal
            .backend_mut()
            .assert_cursor_position(Position { x: 3, y: 0 });
    }

    #[rstest::rstest]
    fn render_cursor_at_home() {
        // Given a ChatInputBoxElement in Input mode with "hi" and cursor moved to start.
        let mut element = ChatInputBoxElement;
        let state = {
            let mut s = AppState::default();
            s.frontend.scope_stack.push(FocusScope::Input);
            s.active_chat_input_mut().insert_text("hi");
            s.active_chat_input_mut().move_cursor_to_start();
            s
        };

        let (mut terminal, area) = setup_term(40, 3);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then cursor is at position (2, 0): inner.x=0 + "> "=2 + cursor_pos=0.
        terminal
            .backend_mut()
            .assert_cursor_position(Position { x: 2, y: 0 });
    }

    #[rstest::rstest]
    fn multiline_first_line_has_prefix() {
        // Given a ChatInputBoxElement with "hello\nworld" in buffer (Normal mode).
        let mut element = ChatInputBoxElement;
        let state = {
            let mut s = AppState::default();
            s.active_chat_input_mut().insert_text("hello\nworld");
            s
        };

        let (mut terminal, area) = setup_term(40, 5);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then line 0 (row 0) has "> " prefix and "hello".
        let buffer = terminal.backend().buffer().clone();
        let cell = buffer.cell((0, 0)).expect("cell should exist");
        assert_eq!(cell.symbol(), ">");
        let h_cell = buffer.cell((2, 0)).expect("cell should exist");
        assert_eq!(h_cell.symbol(), "h");
    }

    #[rstest::rstest]
    fn multiline_second_line_has_indent() {
        // Given a ChatInputBoxElement with "hello\nworld" in buffer (Normal mode).
        let mut element = ChatInputBoxElement;
        let state = {
            let mut s = AppState::default();
            s.active_chat_input_mut().insert_text("hello\nworld");
            s
        };

        let (mut terminal, area) = setup_term(40, 5);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then line 1 (row 1) has "  " indent and "world".
        let buffer = terminal.backend().buffer().clone();
        let indent_cell = buffer.cell((0, 1)).expect("cell should exist");
        assert_eq!(indent_cell.symbol(), " ");
        let w_cell = buffer.cell((2, 1)).expect("cell should exist");
        assert_eq!(w_cell.symbol(), "w");
    }

    #[rstest::rstest]
    fn render_multiline_cursor_on_second_line() {
        // Given a ChatInputBoxElement in Input mode with "ab\ncd" and cursor at end.
        let mut element = ChatInputBoxElement;
        let state = {
            let mut s = AppState::default();
            s.frontend.scope_stack.push(FocusScope::Input);
            s.active_chat_input_mut().insert_text("ab\ncd");
            s
        };

        let (mut terminal, area) = setup_term(40, 5);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then cursor is at position (4, 1): row 1, col 2.
        // inner.x=0, indent=2, col=2 → x=4, y=inner.y + 1 = 0 + 1 = 1.
        terminal
            .backend_mut()
            .assert_cursor_position(Position { x: 4, y: 1 });
    }

    #[rstest::rstest]
    fn render_multiline_cursor_between_newlines() {
        // Given a ChatInputBoxElement in Input mode with "a\n\nb" and cursor on the empty middle line.
        let mut element = ChatInputBoxElement;
        let state = {
            let mut s = AppState::default();
            s.frontend.scope_stack.push(FocusScope::Input);
            s.active_chat_input_mut().insert_text("a\n\nb");
            // Cursor is at end (pos 4). Move back 1 to be on the empty middle line.
            s.active_chat_input_mut().move_cursor_left(); // now at pos 3, which is after the second \n, before 'b'
            // Actually: "a\n\nb" → graphemes: a(0) \n(1) \n(2) b(3). cursor at 3 = before 'b'.
            // Move left once more to be at pos 2 = after first \n, on empty line.
            s.active_chat_input_mut().move_cursor_left();
            s
        };

        let (mut terminal, area) = setup_term(40, 5);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then cursor is on row 1 (empty middle line), col 0.
        // inner.y=0, row=1 → y=1, indent=2, col=0 → x=2.
        terminal
            .backend_mut()
            .assert_cursor_position(Position { x: 2, y: 1 });
    }

    #[rstest::rstest]
    fn render_wraps_long_text() {
        // Given "hello world" in a narrow terminal (width 10) so it wraps.
        let mut element = ChatInputBoxElement;
        let state = {
            let mut s = AppState::default();
            s.active_chat_input_mut().insert_text("hello world");
            // Set wrap width to simulate narrow terminal: 10 - 2 prefix = 8
            s.active_chat_input_mut().set_wrap_width(8);
            s
        };

        let (mut terminal, area) = setup_term(10, 5);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the text is rendered across multiple visual lines.
        let buffer = terminal.backend().buffer().clone();
        // Row 0 should have "> hello " (prefix + first part of wrapped text).
        let cell = buffer.cell((2, 0)).expect("cell should exist");
        assert_eq!(cell.symbol(), "h");
        // Row 1 should have continuation with "world".
        let w_cell = buffer.cell((2, 1)).expect("cell should exist");
        assert_eq!(w_cell.symbol(), "w");
    }

    #[rstest::rstest]
    fn render_cursor_on_wrapped_continuation() {
        // Given "hello world" in a narrow terminal with cursor on wrapped line.
        let mut element = ChatInputBoxElement;
        let state = {
            let mut s = AppState::default();
            s.frontend.scope_stack.push(FocusScope::Input);
            s.active_chat_input_mut().insert_text("hello world");
            s.active_chat_input_mut().set_wrap_width(8);
            s
        };

        let (mut terminal, area) = setup_term(10, 5);

        // When rendering (cursor is at end, which is on the wrapped continuation line).
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then cursor is on row 1 (the continuation line).
        let _buffer = terminal.backend().buffer().clone();
        // The cursor should be visible on the second visual line.
        // Cursor at pos 11 (end). Wrapped lines: "> hello " and "  world".
        // Row 0 = "> hello " (8 graphemes), Row 1 = "  world" (5 graphemes).
        // cursor_row_col returns (1, 5) — row 1, col 5.
        // visual_row = 1, cursor_y = inner.y + 1 = 1.
        // cursor_x = inner.x + 2 + 5 = 7.
        terminal
            .backend_mut()
            .assert_cursor_position(Position { x: 7, y: 1 });
    }

    #[rstest::rstest]
    fn indicator_shows_up_arrow_when_lines_hidden_above() {
        // Given a narrow terminal with 3 visible rows and 5 total lines, scrolled to offset 2.
        let mut element = ChatInputBoxElement;
        let state = {
            let mut s = AppState::default();
            // 5 logical lines, narrow width so each wraps to 1 visual line.
            s.active_chat_input_mut()
                .insert_text("line1\nline2\nline3\nline4\nline5");
            s.active_chat_input_mut().set_wrap_width(38);
            s.active_chat_input_mut().set_scroll_offset(2);
            s
        };

        let (mut terminal, area) = setup_term(40, 4);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the up arrow indicator appears on the top-right.
        // "↑ 2" = 3 display cols, right-aligned on row 0 at x = 40 - 3 = 37.
        let buffer = terminal.backend().buffer().clone();
        let arrow_cell = buffer.cell((37, 0)).expect("cell should exist");
        assert_eq!(arrow_cell.symbol(), "↑");
        assert_eq!(arrow_cell.style().fg, Some(default_theme().age_fresh));
        assert_eq!(
            arrow_cell.style().bg,
            Some(default_theme().scroll_indicator_bg)
        );
        let num_cell = buffer.cell((39, 0)).expect("cell should exist");
        assert_eq!(num_cell.symbol(), "2");
    }

    #[rstest::rstest]
    fn indicator_shows_down_arrow_when_lines_hidden_below() {
        // Given a narrow terminal with 3 visible rows and 5 total lines, scrolled to offset 0.
        let mut element = ChatInputBoxElement;
        let state = {
            let mut s = AppState::default();
            s.active_chat_input_mut()
                .insert_text("line1\nline2\nline3\nline4\nline5");
            s.active_chat_input_mut().set_wrap_width(38);
            // scroll_offset = 0, so lines_above = 0, lines_below = 5 - 0 - 3 = 2.
            s
        };

        let (mut terminal, area) = setup_term(40, 4);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the down arrow indicator appears on the bottom-right of the inner area.
        // inner area: rows 0..2 (3 rows), bottom row is y=2.
        // "↓ 2" = 3 display cols, right-aligned at x = 40 - 3 = 37, y = 2.
        let buffer = terminal.backend().buffer().clone();
        let arrow_cell = buffer.cell((37, 2)).expect("cell should exist");
        assert_eq!(arrow_cell.symbol(), "↓");
        assert_eq!(arrow_cell.style().fg, Some(default_theme().age_fresh));
        assert_eq!(
            arrow_cell.style().bg,
            Some(default_theme().scroll_indicator_bg)
        );
        let num_cell = buffer.cell((39, 2)).expect("cell should exist");
        assert_eq!(num_cell.symbol(), "2");
    }

    #[rstest::rstest]
    fn indicator_shows_both_arrows_when_viewport_in_middle() {
        // Given a narrow terminal with 3 visible rows and 7 total lines, scrolled to offset 2.
        let mut element = ChatInputBoxElement;
        let state = {
            let mut s = AppState::default();
            s.active_chat_input_mut()
                .insert_text("line1\nline2\nline3\nline4\nline5\nline6\nline7");
            s.active_chat_input_mut().set_wrap_width(38);
            s.active_chat_input_mut().set_scroll_offset(2);
            // lines_above = 2, lines_below = 7 - 2 - 3 = 2.
            s
        };

        let (mut terminal, area) = setup_term(40, 4);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then both indicators appear.
        let buffer = terminal.backend().buffer().clone();

        // Up arrow on top-right (row 0). "↑ 2" = 3 display cols → x = 37.
        let up_cell = buffer.cell((37, 0)).expect("cell should exist");
        assert_eq!(up_cell.symbol(), "↑");
        assert_eq!(up_cell.style().fg, Some(default_theme().age_fresh));

        // Down arrow on bottom-right of inner area (row 2). "↓ 2" = 3 display cols → x = 37.
        let down_cell = buffer.cell((37, 2)).expect("cell should exist");
        assert_eq!(down_cell.symbol(), "↓");
        assert_eq!(down_cell.style().fg, Some(default_theme().age_fresh));
    }

    #[rstest::rstest]
    fn no_indicators_when_content_fits() {
        // Given a terminal where all content fits without scrolling.
        let mut element = ChatInputBoxElement;
        let state = {
            let mut s = AppState::default();
            s.active_chat_input_mut().insert_text("hello");
            s.active_chat_input_mut().set_wrap_width(38);
            s
        };

        let (mut terminal, area) = setup_term(40, 3);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then no arrow indicators appear on the right edge.
        let buffer = terminal.backend().buffer().clone();
        // Check that the rightmost cell on row 0 is NOT an arrow.
        let right_cell = buffer.cell((39, 0)).expect("cell should exist");
        assert_ne!(right_cell.symbol(), "↑");
        assert_ne!(right_cell.symbol(), "↓");
    }
}
