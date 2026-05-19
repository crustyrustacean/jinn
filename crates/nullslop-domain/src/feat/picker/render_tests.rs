#![allow(clippy::expect_used, clippy::indexing_slicing)]

//! Keymap and context strategy picker render tests.

use crate::common::app_state::AppState;
use crate::common::app_state::FocusScope;
use crate::common::services::Services;
use crate::feat::theme::default_theme;
use crate::protocol::{Intent, KeymapEntry, PickerKind};
use nullslop_selection_widget::compute_popup_rect;
use nullslop_testutil::setup_term;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::render::{render_context_strategy_picker, render_keymap_picker};
use super::strategy_entries;

// --- Context strategy picker rendering tests ---

/// Helper to create a state with strategy entries loaded.
fn strategy_picker_state() -> (AppState, Services) {
    let services = Services::new();
    let mut state = AppState::default();
    strategy_entries::load_strategy_picker_items(&services, &mut state);
    (state, services)
}

#[rstest::rstest]
fn render_context_strategy_picker_shows_telescope_layout() {
    // Given a terminal area and picker state with entries loaded.

    let (mut state, _services) = strategy_picker_state();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::ContextAssembly,
    });

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering the picker.
    terminal
        .draw(|frame| {
            let area = frame.area();
            render_context_strategy_picker(frame, area, &state);
        })
        .unwrap();

    // Then the popup shows telescope layout (filter marker and separator).
    let buffer = terminal.backend().buffer().clone();
    let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
    // Filter is on the first inner row: popup.y + 1
    let filter_y = popup.y + 1;
    let filter_cell = buffer.cell((popup.x + 1, filter_y)).expect("filter cell");
    assert_eq!(filter_cell.symbol(), ">");

    // Separator is on the second inner row.
    let sep_y = popup.y + 2;
    let sep_cell = buffer.cell((popup.x + 1, sep_y)).expect("sep cell");
    assert_eq!(sep_cell.symbol(), "\u{2500}");
}

#[rstest::rstest]
fn render_context_strategy_picker_shows_active_marker() {
    // Given a state with entries (default is passthrough active).

    let (mut state, _services) = strategy_picker_state();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::ContextAssembly,
    });

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering the picker.
    terminal
        .draw(|frame| {
            let area = frame.area();
            render_context_strategy_picker(frame, area, &state);
        })
        .unwrap();

    // Then the first result row starts with ">" (active marker) in green.
    let buffer = terminal.backend().buffer().clone();
    let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
    // Results start at popup.y + 3 (border + input + separator)
    let result_y = popup.y + 3;
    let marker_cell = buffer.cell((popup.x + 1, result_y)).expect("marker cell");
    assert_eq!(marker_cell.symbol(), ">");
    assert_eq!(marker_cell.fg, Color::Green);
}

#[rstest::rstest]
fn render_context_strategy_picker_shows_footer_with_current_strategy() {
    // Given a state with entries (default is passthrough active).

    let (mut state, _services) = strategy_picker_state();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::ContextAssembly,
    });

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering the picker.
    terminal
        .draw(|frame| {
            let area = frame.area();
            render_context_strategy_picker(frame, area, &state);
        })
        .unwrap();

    // Then the buffer contains "Current:" and "Passthrough" in the footer area.
    let buffer = terminal.backend().buffer().clone();
    let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
    // Footer is the last row of the inner area (before bottom border).
    let footer_y = popup.y + popup.height - 2;
    let row_text: String = (popup.x..popup.x + popup.width)
        .filter_map(|x| {
            buffer
                .cell((x, footer_y))
                .map(ratatui::buffer::Cell::symbol)
        })
        .collect();
    assert!(
        row_text.contains("Current:"),
        "footer should contain 'Current:' text, got: {row_text}"
    );
    assert!(
        row_text.contains("Passthrough"),
        "footer should contain 'Passthrough' text, got: {row_text}"
    );
}

// --- Keymap picker rendering tests ---

fn keymap_picker_state() -> AppState {
    let mut state = AppState::default();
    let entries = vec![
        KeymapEntry {
            key_sequence: "q".to_owned(),
            description: "quit".to_owned(),
            scope: "Normal".to_owned(),
            category: "General".to_owned(),
            command: Intent::Quit,
            search_text: "q quit".to_owned(),
            theme: default_theme(),
        },
        KeymapEntry {
            key_sequence: "gg".to_owned(),
            description: "scroll to top".to_owned(),
            scope: "Normal".to_owned(),
            category: "Navigation".to_owned(),
            command: Intent::ScrollToTop,
            search_text: "gg scroll to top".to_owned(),
            theme: default_theme(),
        },
        KeymapEntry {
            key_sequence: "<esc>".to_owned(),
            description: "set mode normal".to_owned(),
            scope: "Picker".to_owned(),
            category: "General".to_owned(),
            command: Intent::EnterNormalMode,
            search_text: "<esc> set mode normal".to_owned(),
            theme: default_theme(),
        },
    ];
    state.frontend.keymap_picker.set_items(entries);
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Keymap,
    });
    state
}

#[rstest::rstest]
fn render_keymap_picker_shows_telescope_layout() {
    // Given a terminal area with keymap picker state.

    let state = keymap_picker_state();

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering the picker.
    terminal
        .draw(|frame| {
            let area = frame.area();
            render_keymap_picker(frame, area, &state);
        })
        .unwrap();

    // Then the buffer contains the title "Keymaps".
    let buffer = terminal.backend().buffer().clone();
    let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));

    // Title is on the first row (top border area).
    let title_row: String = (popup.x..popup.x + popup.width)
        .filter_map(|x| buffer.cell((x, popup.y)).map(ratatui::buffer::Cell::symbol))
        .collect();
    assert!(
        title_row.contains("Keymaps"),
        "title should contain 'Keymaps', got: {title_row}"
    );

    // Border color is DarkGray.
    let border_cell = buffer.cell((popup.x, popup.y)).expect("border cell");
    assert_eq!(border_cell.fg, Color::DarkGray);

    // Filter prompt is present on the first inner row.
    let filter_y = popup.y + 1;
    let filter_cell = buffer.cell((popup.x + 1, filter_y)).expect("filter cell");
    assert_eq!(filter_cell.symbol(), ">");
}

#[rstest::rstest]
fn render_keymap_picker_footer_shows_current_scope() {
    // Given a keymap picker state with show_all = false and origin scope "Normal".

    let mut state = keymap_picker_state();
    state.frontend.keymap_picker_show_all = false;

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering the picker.
    terminal
        .draw(|frame| {
            let area = frame.area();
            render_keymap_picker(frame, area, &state);
        })
        .unwrap();

    // Then the footer contains "Scope: Normal" and "CTRL+A".
    let buffer = terminal.backend().buffer().clone();
    let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
    let footer_y = popup.y + popup.height - 2;
    let footer_row: String = (popup.x..popup.x + popup.width)
        .filter_map(|x| {
            buffer
                .cell((x, footer_y))
                .map(ratatui::buffer::Cell::symbol)
        })
        .collect();
    assert!(
        footer_row.contains("Scope: Normal"),
        "footer should contain 'Scope: Normal', got: {footer_row}"
    );
    assert!(
        footer_row.contains("CTRL+A"),
        "footer should contain 'CTRL+A', got: {footer_row}"
    );
}

#[rstest::rstest]
fn render_keymap_picker_footer_shows_all_scopes() {
    // Given a keymap picker state with show_all = true and origin scope "Normal".

    let mut state = keymap_picker_state();
    state.frontend.keymap_picker_show_all = true;

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering the picker.
    terminal
        .draw(|frame| {
            let area = frame.area();
            render_keymap_picker(frame, area, &state);
        })
        .unwrap();

    // Then the footer contains "All scopes" and "CTRL+A to show Normal".
    let buffer = terminal.backend().buffer().clone();
    let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
    let footer_y = popup.y + popup.height - 2;
    let footer_row: String = (popup.x..popup.x + popup.width)
        .filter_map(|x| {
            buffer
                .cell((x, footer_y))
                .map(ratatui::buffer::Cell::symbol)
        })
        .collect();
    assert!(
        footer_row.contains("All scopes"),
        "footer should contain 'All scopes', got: {footer_row}"
    );
    assert!(
        footer_row.contains("CTRL+A to show Normal"),
        "footer should contain 'CTRL+A to show Normal', got: {footer_row}"
    );
}
