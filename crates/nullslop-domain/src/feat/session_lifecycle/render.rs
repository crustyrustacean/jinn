//! Session lifecycle picker and arg input popup rendering.

use crate::common::app_state::AppState;
use crate::feat::session_lifecycle::command_template::CommandTemplate;
use nullslop_selection_widget::SelectionWidget;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_segmentation::UnicodeSegmentation;

/// Renders the session lifecycle picker overlay using [`SelectionWidget`].
///
/// Shows all available lifecycles (including the implicit blank) with
/// descriptions and an args indicator.
pub fn render_session_lifecycle_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let widget = SelectionWidget::new(&state.frontend.session_lifecycle_picker)
        .title(Line::from(" Session Lifecycle "))
        .footer(Line::from(" Enter to select, ESC to cancel "));
    widget.render(frame, area);
}

/// Renders the arg input popup for a lifecycle with positional parameters.
///
/// Shows a centered popup with:
/// - Title: "Session Lifecycle Args"
/// - Command template line (e.g., `./script.sh <1> <2>`)
/// - One line per parameter: `$1: value`, `$2: value`, etc.
/// - Input line at bottom showing current text with cursor
pub fn render_arg_input(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let arg_state = &state.frontend.arg_input;

    // Compute popup rect (centered, similar to picker).
    let popup_area = nullslop_selection_widget::compute_popup_rect(area);

    // Parse the command template to extract parameter info.
    let template = state
        .frontend
        .preferences
        .session_lifecycles
        .iter()
        .find(|l| l.name == arg_state.lifecycle_name)
        .and_then(|l| l.setup_command.as_ref())
        .map(|cmd| CommandTemplate::parse(cmd));

    let theme = &state.frontend.theme;

    let Some(template) = template else {
        // No template found — render minimal popup anyway.
        let block = Block::default()
            .title(" Session Lifecycle Args ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.focus_accent));
        frame.render_widget(block, popup_area);
        return;
    };

    let title = Line::from(Span::styled(
        " Session Lifecycle Args ",
        Style::default().fg(theme.focus_accent),
    ));

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.focus_accent));

    // Render the block first.
    frame.render_widget(block, popup_area);

    // Inner area for content (1 padding on each side).
    let inner = Rect {
        x: popup_area.x + 1,
        y: popup_area.y + 1,
        width: popup_area.width.saturating_sub(2),
        height: popup_area.height.saturating_sub(2),
    };

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // Split the user's input into tokens by whitespace.
    let input_args: Vec<&str> = arg_state.input.split_whitespace().collect();

    // Render lines one by one, tracking y position.
    let mut y_offset = inner.y;
    let max_y = inner.y + inner.height;

    // Line: command template.
    if y_offset < max_y {
        let template_display = template.display();
        let para = Paragraph::new(Line::from(Span::raw(format!(" {}", &template_display))));
        frame.render_widget(para, Rect::new(inner.x, y_offset, inner.width, 1));
        y_offset += 1;
    }

    // Blank line.
    if y_offset < max_y {
        y_offset += 1;
    }

    // Parameter lines.
    if template.has_splat() {
        // $@ / $*: show all args on one line.
        if y_offset < max_y {
            let value = arg_state.input.clone();
            let line_text =
                if value.is_empty() { "$@: ".to_owned() } else { format!("$@: {value}") };
            let para = Paragraph::new(Line::from(Span::raw(line_text)));
            frame.render_widget(para, Rect::new(inner.x, y_offset, inner.width, 1));
            y_offset += 1;
        }
    }

    for &param_num in template.params() {
        if y_offset >= max_y {
            break;
        }
        // param_num is 1-indexed; input_args is 0-indexed.
        let value = if param_num <= input_args.len() {
            input_args[param_num - 1]
        } else {
            ""
        };
        let line_text =
            if value.is_empty() { format!("${}: ", param_num) } else { format!("${}: {value}", param_num) };
        let para = Paragraph::new(Line::from(Span::raw(line_text)));
        frame.render_widget(para, Rect::new(inner.x, y_offset, inner.width, 1));
        y_offset += 1;
    }

    // Blank line before input.
    if y_offset < max_y {
        y_offset += 1;
    }

    // Last line: the input line with cursor.
    if y_offset < max_y {
        let input_line = Line::from(Span::raw(format!("> {}", arg_state.input)));
        let input_para = Paragraph::new(input_line);
        frame.render_widget(input_para, Rect::new(inner.x, y_offset, inner.width, 1));

        // Compute cursor x position: "> " (2) + grapheme count up to cursor_pos.
        let prefix_len = 2u16; // "> "
        let grapheme_count = arg_state.input[..arg_state.cursor_pos]
            .graphemes(true)
            .count();
        let cursor_x =
            (prefix_len + grapheme_count as u16).min(inner.width.saturating_sub(1));
        frame.set_cursor_position((inner.x.saturating_add(cursor_x), y_offset));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::app_state::ArgInputState;
    use crate::feat::preferences_actor::user_preferences::SessionLifecycle;
    use nullslop_testutil::setup_term;

    fn make_state_with_args(
        lifecycle_name: &str,
        setup_command: Option<&str>,
        input: &str,
        cursor_pos: usize,
    ) -> AppState {
        let mut state = AppState::default();
        state.frontend.arg_input = ArgInputState {
            lifecycle_name: lifecycle_name.to_owned(),
            template_display: String::new(),
            input: input.to_owned(),
            cursor_pos,
        };
        if let Some(cmd) = setup_command {
            state
                .frontend
                .preferences
                .session_lifecycles
                .push(SessionLifecycle {
                    name: lifecycle_name.to_owned(),
                    description: None,
                    setup_command: Some(cmd.to_owned()),
                    teardown_command: None,
                });
        }
        state
    }

    #[rstest::rstest]
    fn arg_input_popup_shows_title_in_border_no_template() {
        // Given a state with an unknown lifecycle (no template found).
        let state = make_state_with_args("unknown", None, "", 0);
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering the arg input popup.
        terminal
            .draw(|frame| {
                render_arg_input(frame, area, &state);
            })
            .unwrap();

        // Then the popup title appears in the top border.
        let buffer = terminal.backend().buffer().clone();
        let popup_area = nullslop_selection_widget::compute_popup_rect(area);
        let title_line_y = popup_area.y;

        let title_text = " Session Lifecycle Args ";
        let mut found_title = false;
        for x in popup_area.x..(popup_area.x + popup_area.width).min(buffer.area().width) {
            if let Some(cell) = buffer.cell((x, title_line_y)) {
                let cell_text: &str = cell.symbol();
                if matches!(cell_text, "┌" | "─" | "┐") {
                    continue;
                }
                if title_text.contains(cell_text) {
                    found_title = true;
                    break;
                }
            }
        }
        assert!(found_title, "title should appear in the top border");
    }

    #[rstest::rstest]
    fn arg_input_popup_shows_template_display() {
        // Given a state with a lifecycle that has $1 $2 params.
        let state = make_state_with_args("test", Some("script.sh $1 $2"), "", 0);
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                render_arg_input(frame, area, &state);
            })
            .unwrap();

        // Then the template display appears in the popup.
        let buffer = terminal.backend().buffer().clone();
        let popup_area = nullslop_selection_widget::compute_popup_rect(area);
        let inner_x = popup_area.x + 1;
        let template_y = popup_area.y + 1;

        if template_y < buffer.area().height {
            let display = " script.sh <1> <2>";
            for (i, expected_ch) in display.chars().enumerate() {
                let x = inner_x + i as u16;
                if x >= buffer.area().width {
                    break;
                }
                if let Some(cell) = buffer.cell((x, template_y)) {
                    assert_eq!(
                        cell.symbol(),
                        expected_ch.to_string(),
                        "template display mismatch at offset {i}"
                    );
                }
            }
        }
    }

    #[rstest::rstest]
    fn arg_input_popup_shows_param_labels() {
        // Given a state with $1 $2 params and typed input.
        let state = make_state_with_args("test", Some("script.sh $1 $2"), "foo bar", 7);
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                render_arg_input(frame, area, &state);
            })
            .unwrap();

        // Then parameter labels and values appear.
        // Layout: inner.y = popup.y+1 = template, popup.y+2 = blank, popup.y+3 = first param.
        let buffer = terminal.backend().buffer().clone();
        let popup_area = nullslop_selection_widget::compute_popup_rect(area);
        let inner_x = popup_area.x + 1;
        let param_y = popup_area.y + 3; // row 3: $1: foo

        if param_y < buffer.area().height {
            // Check "$1: foo" presence starting at inner_x.
            let expected = "$1: foo";
            for (i, expected_ch) in expected.chars().enumerate() {
                let x = inner_x + i as u16;
                if x >= buffer.area().width {
                    break;
                }
                if let Some(cell) = buffer.cell((x, param_y)) {
                    assert_eq!(
                        cell.symbol(),
                        expected_ch.to_string(),
                        "param label mismatch at offset {i}"
                    );
                }
            }

            // Row 4: $2: bar (second param).
            let param2_y = popup_area.y + 4;
            if param2_y < buffer.area().height {
                let expected2 = "$2: bar";
                for (i, expected_ch) in expected2.chars().enumerate() {
                    let x = inner_x + i as u16;
                    if x >= buffer.area().width {
                        break;
                    }
                    if let Some(cell) = buffer.cell((x, param2_y)) {
                        assert_eq!(
                            cell.symbol(),
                            expected_ch.to_string(),
                            "second param label mismatch at offset {i}"
                        );
                    }
                }
            }
        }
    }

    #[rstest::rstest]
    fn arg_input_popup_shows_input_line() {
        // Given a state with a lifecycle and some typed input.
        let state = make_state_with_args("test", Some("script.sh $1"), "hello world", 5);
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                render_arg_input(frame, area, &state);
            })
            .unwrap();

        // Then the input line ("> hello world") appears.
        let buffer = terminal.backend().buffer().clone();
        let popup_area = nullslop_selection_widget::compute_popup_rect(area);
        let inner_x = popup_area.x + 1;

        // Layout: row 1 = template, row 2 = blank, row 3 = $1: hello, row 4 = blank, row 5 = > hello world
        let input_y = popup_area.y + 5;

        if input_y < buffer.area().height {
            let expected = "> hello world";
            for (i, expected_ch) in expected.chars().enumerate() {
                let x = inner_x + i as u16;
                if x >= buffer.area().width {
                    break;
                }
                if let Some(cell) = buffer.cell((x, input_y)) {
                    assert_eq!(
                        cell.symbol(),
                        expected_ch.to_string(),
                        "input line mismatch at offset {i}"
                    );
                }
            }
        }
    }

    #[rstest::rstest]
    fn arg_input_popup_splat_shows_all_args() {
        // Given a state with a lifecycle using $@.
        let state = make_state_with_args("test", Some("script.sh $@"), "a b c", 5);
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                render_arg_input(frame, area, &state);
            })
            .unwrap();

        // Then the splat line shows all arguments.
        let buffer = terminal.backend().buffer().clone();
        let popup_area = nullslop_selection_widget::compute_popup_rect(area);
        let inner_x = popup_area.x + 1;

        // Layout: row 1 = template, row 2 = blank, row 3 = $@: a b c, ...
        let splat_y = popup_area.y + 3;

        if splat_y < buffer.area().height {
            let expected = "$@: a b c";
            for (i, expected_ch) in expected.chars().enumerate() {
                let x = inner_x + i as u16;
                if x >= buffer.area().width {
                    break;
                }
                if let Some(cell) = buffer.cell((x, splat_y)) {
                    assert_eq!(
                        cell.symbol(),
                        expected_ch.to_string(),
                        "splat line mismatch at offset {i}"
                    );
                }
            }
        }
    }

    #[rstest::rstest]
    fn arg_input_popup_handles_no_template() {
        // Given a state with an unknown lifecycle name (no matching template).
        let state = make_state_with_args("nonexistent", None, "foo", 3);
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering — should not panic.
        terminal
            .draw(|frame| {
                render_arg_input(frame, area, &state);
            })
            .unwrap();

        // Then the minimal popup still shows the title.
        let buffer = terminal.backend().buffer().clone();
        let popup_area = nullslop_selection_widget::compute_popup_rect(area);
        let title_line_y = popup_area.y;

        let mut found_title = false;
        for x in popup_area.x..(popup_area.x + popup_area.width).min(buffer.area().width) {
            if let Some(cell) = buffer.cell((x, title_line_y)) {
                let cell_text: &str = cell.symbol();
                if matches!(cell_text, "┌" | "─" | "┐") {
                    continue;
                }
                if " Session Lifecycle Args ".contains(cell_text) {
                    found_title = true;
                    break;
                }
            }
        }
        assert!(found_title, "minimal popup should show title");
    }
}
