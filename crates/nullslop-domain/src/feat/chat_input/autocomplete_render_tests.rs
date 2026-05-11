//! Autocomplete popup render tests.

use crate::feat::chat_input::AutocompleteMatch;
use crate::component::AppState;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use super::render_autocomplete_popup;

/// Creates a test terminal with the given dimensions.
fn setup_term(width: u16, height: u16) -> (Terminal<TestBackend>, Rect) {
    let backend = TestBackend::new(width, height);
    let terminal = Terminal::new(backend).unwrap();
    let area = Rect::new(0, 0, width, height);
    (terminal, area)
}

/// Helper to create an `AppState` with autocomplete active.
///
/// Sets the input buffer to the given text, activates autocomplete at `token_start`,
/// and populates matches.
fn state_with_autocomplete(
    buffer_text: &str,
    token_start: usize,
    matches: Vec<AutocompleteMatch>,
) -> AppState {
    let mut state = AppState::default();
    state
        .active_chat_input_mut()
        .replace_all(buffer_text.to_owned());
    // Position cursor after the buffer text.
    // Note: cursor must be at the end for autocomplete to be consistent.
    state
        .active_chat_input_mut()
        .activate_autocomplete(token_start, matches);
    state
}

/// Extract a line of text from a buffer at the given row.
fn buffer_line(buf: &ratatui::buffer::Buffer, y: u16, start_x: u16, max_len: u16) -> String {
    let mut s = String::new();
    for x in start_x..start_x + max_len {
        let cell = buf.cell((x, y));
        let sym = cell.map_or(" ", ratatui::buffer::Cell::symbol);
        if sym == " " && s.ends_with("  ") {
            break;
        }
        s.push_str(sym);
    }
    s.trim_end().to_owned()
}

#[rstest::rstest]
fn render_autocomplete_popup_shows_matches() {
    // Given an AppState with autocomplete active and 3 matches.

    let matches = vec![
        AutocompleteMatch {
            name: "code-review".to_owned(),
            description: "Perform code review".to_owned(),
        },
        AutocompleteMatch {
            name: "summarize".to_owned(),
            description: "Summarize text".to_owned(),
        },
        AutocompleteMatch {
            name: "test-gen".to_owned(),
            description: "Generate tests".to_owned(),
        },
    ];
    let state = state_with_autocomplete("$co", 0, matches);

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering the autocomplete popup with a known input area.
    let input_area = Rect::new(0, 20, 80, 4);
    terminal
        .draw(|frame| {
            render_autocomplete_popup(frame, input_area, &state);
        })
        .unwrap();

    // Then the popup shows all three matches.
    let buffer = terminal.backend().buffer().clone();
    let popup_top = 20 - 5; // 3 matches + 2 border rows = 5, popup sits above input_area
    // Check that match names appear in the popup content.
    let line1 = buffer_line(&buffer, popup_top + 1, 1, 60);
    let line2 = buffer_line(&buffer, popup_top + 2, 1, 60);
    let line3 = buffer_line(&buffer, popup_top + 3, 1, 60);
    assert!(
        line1.contains("code-review"),
        "first match should contain 'code-review', got: {line1}"
    );
    assert!(
        line2.contains("summarize"),
        "second match should contain 'summarize', got: {line2}"
    );
    assert!(
        line3.contains("test-gen"),
        "third match should contain 'test-gen', got: {line3}"
    );
}

#[rstest::rstest]
fn render_autocomplete_popup_highlights_selected() {
    // Given an AppState with 2 matches and the second (most-relevant) selected.

    let matches = vec![
        AutocompleteMatch {
            name: "alpha".to_owned(),
            description: String::new(),
        },
        AutocompleteMatch {
            name: "beta".to_owned(),
            description: String::new(),
        },
    ];
    let mut state = state_with_autocomplete("$", 0, matches);
    // Default selected_index is last (index 1 = "beta").
    // Move selection up to select index 0 ("alpha").
    state.active_chat_input_mut().autocomplete_move_up();

    let (mut terminal, _area) = setup_term(80, 24);
    let input_area = Rect::new(0, 20, 80, 4);

    terminal
        .draw(|frame| {
            render_autocomplete_popup(frame, input_area, &state);
        })
        .unwrap();

    // Then the selected row has Modifier::REVERSED.
    // Popup: anchor_x = 0 + 2 (prompt_indent) + 0 (token_col) = 2.
    // Popup at (2, 16, 20, 4). Content starts at x=3.
    // First match (index 0, selected) at y=17, second at y=18.
    let buffer = terminal.backend().buffer().clone();
    let selected_cell = buffer.cell((3, 17)).expect("selected cell");
    assert!(
        selected_cell.modifier.contains(Modifier::REVERSED),
        "selected cell should have REVERSED modifier"
    );

    // Second match (index 1) is NOT selected.
    let unselected_cell = buffer.cell((3, 18)).expect("unselected cell");
    assert!(
        !unselected_cell.modifier.contains(Modifier::REVERSED),
        "unselected cell should NOT have REVERSED modifier"
    );
}

#[rstest::rstest]
fn render_autocomplete_popup_shows_no_matches_message() {
    // Given an AppState with autocomplete active but 0 matches.

    let state = state_with_autocomplete("$xyz", 0, vec![]);

    let (mut terminal, _area) = setup_term(80, 24);
    let input_area = Rect::new(0, 20, 80, 4);

    terminal
        .draw(|frame| {
            render_autocomplete_popup(frame, input_area, &state);
        })
        .unwrap();

    // Then the popup shows "<no prompts found>".
    let buffer = terminal.backend().buffer().clone();
    let popup_top = 20 - 3; // 1 content + 2 borders
    let line = buffer_line(&buffer, popup_top + 1, 1, 60);
    assert!(
        line.contains("<no prompts found>"),
        "should show no matches message, got: {line}"
    );
}

#[rstest::rstest]
fn render_autocomplete_popup_positioned_above_input() {
    // Given a known input area at row 20.

    let matches = vec![AutocompleteMatch {
        name: "test".to_owned(),
        description: "A test".to_owned(),
    }];
    let state = state_with_autocomplete("$", 0, matches);

    let (mut terminal, _area) = setup_term(80, 24);
    let input_area = Rect::new(0, 20, 80, 4);

    terminal
        .draw(|frame| {
            render_autocomplete_popup(frame, input_area, &state);
        })
        .unwrap();

    // Then the popup's bottom edge touches input_area.y.
    // Popup: anchor_x = 0 + 2 + 0 = 2. Height = 1 + 2 = 3.
    // popup_y = 20 - 3 = 17. Bottom border at y = 19.
    let buffer = terminal.backend().buffer().clone();
    let border_cell = buffer.cell((2, 19)).expect("bottom border cell");
    assert_eq!(
        border_cell.fg,
        Color::DarkGray,
        "bottom border of popup should be at row 19, x=2 (popup anchor)"
    );
}

#[rstest::rstest]
fn render_autocomplete_popup_anchored_at_dollar() {
    // Given a buffer "foo $co" — the $ is at grapheme index 4.

    let matches = vec![AutocompleteMatch {
        name: "code".to_owned(),
        description: "Code stuff".to_owned(),
    }];
    let state = state_with_autocomplete("foo $co", 4, matches);

    let (mut terminal, _area) = setup_term(80, 24);
    // Input area starts at x=10 to see horizontal anchoring.
    let input_area = Rect::new(10, 20, 70, 4);

    terminal
        .draw(|frame| {
            render_autocomplete_popup(frame, input_area, &state);
        })
        .unwrap();

    // Then the popup's left edge is near the $ column.
    // $ is at grapheme index 4 in the buffer, col 4 on the first line.
    // Input inner starts at x=10, prompt_indent=2, so anchor_x = 10 + 2 + 4 = 16.
    let buffer = terminal.backend().buffer().clone();
    let popup_top = 20 - 3; // 1 match + 2 borders
    // Top-left corner of the popup should be at or near x=16.
    let corner_cell = buffer.cell((16, popup_top)).expect("popup corner");
    assert_eq!(
        corner_cell.fg,
        Color::DarkGray,
        "popup left border should be anchored at $ column (x=16)"
    );
}

#[rstest::rstest]
fn render_autocomplete_popup_width_based_on_content() {
    // Given matches with varying name lengths.

    let matches = vec![
        AutocompleteMatch {
            name: "short".to_owned(),
            description: "s".to_owned(),
        },
        AutocompleteMatch {
            name: "a-very-long-template-name".to_owned(),
            description: "A very long description indeed".to_owned(),
        },
    ];
    let state = state_with_autocomplete("$", 0, matches);

    let (mut terminal, _area) = setup_term(80, 24);
    let input_area = Rect::new(0, 20, 80, 4);

    terminal
        .draw(|frame| {
            render_autocomplete_popup(frame, input_area, &state);
        })
        .unwrap();

    // Then the popup width accommodates the longest line plus borders.
    let buffer = terminal.backend().buffer().clone();
    let popup_top = 20 - 4; // 2 matches + 2 borders
    // The longest line: "a-very-long-template-name — A very long description indeed"
    // Check that the longer match text is visible in the buffer.
    let long_line = buffer_line(&buffer, popup_top + 2, 1, 60);
    assert!(
        long_line.contains("a-very-long-template-name"),
        "long name should be visible, got: {long_line}"
    );
}

#[rstest::rstest]
fn render_autocomplete_popup_does_not_render_when_inactive() {
    // Given an AppState with autocomplete inactive.

    let state = AppState::default();

    let (mut terminal, _area) = setup_term(80, 24);
    let input_area = Rect::new(0, 20, 80, 4);

    terminal
        .draw(|frame| {
            render_autocomplete_popup(frame, input_area, &state);
        })
        .unwrap();

    // Then no popup renders — the buffer should remain empty (default space chars).
    let buffer = terminal.backend().buffer().clone();
    // Check an area above the input where the popup would be.
    let cell = buffer.cell((0, 15)).expect("cell should exist");
    assert_eq!(
        cell.symbol(),
        " ",
        "no popup content should appear when autocomplete is inactive"
    );
}
