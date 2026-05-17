//! Session lifecycle picker and arg input popup rendering.

use crate::common::app_state::AppState;
use crate::feat::session_lifecycle::command_template::{CommandTemplate, Param};
use nullslop_selection_widget::SelectionWidget;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
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
/// - Command template line (e.g., `./script.sh <1> <2>` or `./script.sh <branch> <target>`)
/// - One line per parameter: `$1: value`, `<branch>: value`, etc.
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
        frame.render_widget(Clear, popup_area);
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

    // Clear background before rendering the popup borders.
    frame.render_widget(Clear, popup_area);
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

    // Parameter lines — one per unique parameter in order of first appearance.
    for (idx, param) in template.params().iter().enumerate() {
        if y_offset >= max_y {
            break;
        }
        let line_text = match param {
            Param::Named(_) | Param::Positional(_) => {
                let value = if idx < input_args.len() {
                    input_args[idx]
                } else {
                    ""
                };
                let label = match param {
                    Param::Named(name) => format!("<{name}>"),
                    Param::Positional(n) => format!("${}", n),
                    Param::Splat => unreachable!(),
                };
                if value.is_empty() {
                    format!("{label}: ")
                } else {
                    format!("{label}: {value}")
                }
            }
            Param::Splat => {
                // Splat shows all remaining args after positional slots.
                let remaining: Vec<&str> = input_args[idx..].to_vec();
                if remaining.is_empty() {
                    "$@: ".to_owned()
                } else {
                    format!("$@: {}", remaining.join(" "))
                }
            }
        };
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
        let buffer = terminal.backend().buffer().clone();
        let popup_area = nullslop_selection_widget::compute_popup_rect(area);
        let inner_x = popup_area.x + 1;
        let param_y = popup_area.y + 3; // row 3: $1: foo

        if param_y < buffer.area().height {
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
    fn arg_input_popup_shows_named_params() {
        // Given a state with <branch> <target> named params and typed input.
        let state =
            make_state_with_args("test", Some("script.sh <branch> <target>"), "my-feature workdir", 0);
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                render_arg_input(frame, area, &state);
            })
            .unwrap();

        // Then the named parameter labels appear with values.
        let buffer = terminal.backend().buffer().clone();
        let popup_area = nullslop_selection_widget::compute_popup_rect(area);
        let inner_x = popup_area.x + 1;

        // Row 3: <branch>: my-feature
        let param_y = popup_area.y + 3;
        if param_y < buffer.area().height {
            let expected = "<branch>: my-feature";
            for (i, expected_ch) in expected.chars().enumerate() {
                let x = inner_x + i as u16;
                if x >= buffer.area().width {
                    break;
                }
                if let Some(cell) = buffer.cell((x, param_y)) {
                    assert_eq!(
                        cell.symbol(),
                        expected_ch.to_string(),
                        "named param label mismatch at offset {i}"
                    );
                }
            }
        }

        // Row 4: <target>: workdir
        let param2_y = popup_area.y + 4;
        if param2_y < buffer.area().height {
            let expected2 = "<target>: workdir";
            for (i, expected_ch) in expected2.chars().enumerate() {
                let x = inner_x + i as u16;
                if x >= buffer.area().width {
                    break;
                }
                if let Some(cell) = buffer.cell((x, param2_y)) {
                    assert_eq!(
                        cell.symbol(),
                        expected_ch.to_string(),
                        "second named param mismatch at offset {i}"
                    );
                }
            }
        }
    }

    #[rstest::rstest]
    fn arg_input_popup_shows_mixed_named_and_positional() {
        // Given a state with mixed <branch> $1 params.
        let state = make_state_with_args("test", Some("script.sh <branch> $1"), "a b", 0);
        let (mut terminal, area) = setup_term(80, 24);

        // When rendering.
        terminal
            .draw(|frame| {
                render_arg_input(frame, area, &state);
            })
            .unwrap();

        // Then both param types appear with their values.
        let buffer = terminal.backend().buffer().clone();
        let popup_area = nullslop_selection_widget::compute_popup_rect(area);
        let inner_x = popup_area.x + 1;

        // Row 3: <branch>: a
        let param_y = popup_area.y + 3;
        if param_y < buffer.area().height {
            let expected = "<branch>: a";
            for (i, expected_ch) in expected.chars().enumerate() {
                let x = inner_x + i as u16;
                if x >= buffer.area().width {
                    break;
                }
                if let Some(cell) = buffer.cell((x, param_y)) {
                    assert_eq!(
                        cell.symbol(),
                        expected_ch.to_string(),
                        "mixed first param mismatch at offset {i}"
                    );
                }
            }
        }

        // Row 4: $1: b
        let param2_y = popup_area.y + 4;
        if param2_y < buffer.area().height {
            let expected2 = "$1: b";
            for (i, expected_ch) in expected2.chars().enumerate() {
                let x = inner_x + i as u16;
                if x >= buffer.area().width {
                    break;
                }
                if let Some(cell) = buffer.cell((x, param2_y)) {
                    assert_eq!(
                        cell.symbol(),
                        expected_ch.to_string(),
                        "mixed second param mismatch at offset {i}"
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

    #[rstest::rstest]
    fn arg_input_popup_background_cleared() {
        // Given a state with text in the chat area.
        let state = AppState::default();
        let (mut terminal, area) = setup_term(80, 24);

        // First draw something in the background so we can detect if Clear works.
        terminal
            .draw(|frame| {
                // Fill the terminal with characters *before* the popup area.
                let fill = Paragraph::new(Line::from(Span::raw("XXXXX")));
                frame.render_widget(fill, Rect::new(0, 0, 80, 24));
            })
            .unwrap();

        // Then draw the arg input popup on top.
        terminal
            .draw(|frame| {
                render_arg_input(frame, area, &state);
            })
            .unwrap();

        // Then the background character cells inside the popup area should be empty
        // (cleared by Clear widget), not "X".
        let buffer = terminal.backend().buffer().clone();
        let popup_area = nullslop_selection_widget::compute_popup_rect(area);

        // Check a few interior cells (not border) for cleared content.
        let inner_center = (
            popup_area.x + popup_area.width / 2,
            popup_area.y + popup_area.height / 2,
        );
        if let Some(cell) = buffer.cell(inner_center) {
            assert_ne!(
                cell.symbol(),
                "X",
                "background should be cleared inside popup"
            );
        }
    }
}
