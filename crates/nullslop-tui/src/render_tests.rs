use super::*;
use crate::scope::Scope;
use crate::selection::{SelectableRects, SelectionState};
use nullslop_protocol::Intent;
use nullslop_selection_widget::compute_popup_rect;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui_spatial_splits::SplitManager;

/// Creates a minimal `TuiApp` for render testing.
fn render_test_app() -> crate::TuiApp {
    crate::TuiApp::test_builder().build()
}

/// Creates a test terminal with the given dimensions.
fn setup_term(width: u16, height: u16) -> (Terminal<TestBackend>, Rect) {
    let backend = TestBackend::new(width, height);
    let terminal = Terminal::new(backend).unwrap();
    let area = Rect::new(0, 0, width, height);
    (terminal, area)
}



#[rstest::rstest]
fn app_layout_meets_min_size() {
    // Given a 40x14 area.
    let area = Rect::new(0, 0, 40, 14);

    // When checking meets_min_size.
    let result = AppLayout::meets_min_size(area);

    // Then it returns true.
    assert!(result);
}

#[rstest::rstest]
fn app_layout_too_small() {
    // Given a 10x5 area.
    let area = Rect::new(0, 0, 10, 5);

    // When checking meets_min_size.
    let result = AppLayout::meets_min_size(area);

    // Then it returns false.
    assert!(!result);
}

#[rstest::rstest]
fn init_tab_manager_has_two_tabs() {
    // Given a default tab manager.
    let mgr = init_tab_manager();

    // When checking tab count.
    // Then there are 2 tabs and the first is active.
    assert_eq!(mgr.tab_count(), 2);
    assert!(mgr.active_tab().is_some());
    assert_eq!(mgr.active_tab().unwrap().name, "Chat");
}

#[rstest::rstest]
fn app_layout_includes_indicator_row() {
    // Given a 40x14 area.
    let area = Rect::new(0, 0, 40, 14);
    let layout = AppLayout::new(area, 1, 0);

    // Then the indicator row has height 1 and is between content and counter.
    assert_eq!(layout.indicator.height, 1);
    assert!(layout.indicator.y > layout.content.y);
    assert!(layout.indicator.y < layout.counter.y);
}

#[rstest::rstest]
fn app_layout_queue_area_has_dynamic_height() {
    // Given a 40x20 area with 3 queued messages.
    let area = Rect::new(0, 0, 40, 20);
    let layout = AppLayout::new(area, 1, 3);

    // Then the queue area has height 3 and sits between indicator and counter.
    assert_eq!(layout.queue.height, 3);
    assert!(layout.queue.y > layout.indicator.y);
    assert!(layout.queue.y < layout.counter.y);
}

#[rstest::rstest]
fn app_layout_queue_area_zero_height_when_empty() {
    // Given a 40x14 area with no queued messages.
    let area = Rect::new(0, 0, 40, 14);
    let layout = AppLayout::new(area, 1, 0);

    // Then the queue area has height 0.
    assert_eq!(layout.queue.height, 0);
}

#[rstest::rstest]
fn app_layout_includes_status_bar() {
    // Given a 40x14 area.
    let area = Rect::new(0, 0, 40, 14);
    let layout = AppLayout::new(area, 1, 0);

    // Then the status bar has height 1 and is at the bottom.
    assert_eq!(layout.status_bar.height, 1);
    assert!(layout.status_bar.y > layout.input.y);
    assert_eq!(layout.status_bar.y + layout.status_bar.height, area.height);
}

// --- Provider picker rendering tests ---

fn picker_state_with_ollama() -> (nullslop_component::AppState, nullslop_services::Services) {
    use nullslop_providers::{ProviderEntry, ProvidersConfig};
    let config = ProvidersConfig {
        providers: vec![ProviderEntry {
            name: "ollama".to_owned(),
            backend: "ollama".to_owned(),
            models: vec!["llama3".to_owned()],
            base_url: Some("http://localhost:11434".to_owned()),
            api_key_env: None,
            requires_key: false,
        }],
        aliases: vec![],
        default_provider: None,
    };
    let services = nullslop_services::test_services::TestServices::builder()
        .with_providers(config)
        .build();
    (nullslop_component::AppState::default(), services)
}

/// Helper to load provider entries into the picker state.
fn load_picker_items(
    state: &mut nullslop_component::AppState,
    services: &nullslop_services::Services,
) {
    nullslop_component::provider_picker::load_provider_picker_items(services, state);
}

#[rstest::rstest]
fn render_provider_picker_shows_telescope_layout() {
    // Given a terminal area and picker state with filter "ol".
    use nullslop_selection_widget::compute_popup_rect;

    let (mut state, services) = picker_state_with_ollama();
    state.mode = Mode::Picker;
    state.active_picker_kind = Some(PickerKind::Provider);
    load_picker_items(&mut state, &services);
    state.provider_picker.insert_char('o');
    state.provider_picker.insert_char('l');

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering the picker.
    terminal
        .draw(|frame| {
            let area = frame.area();
            render_provider_picker(frame, area, &state);
        })
        .unwrap();

    // Then the popup contains the filter text with "> ol".
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
fn larger_terminal_gets_taller_popup() {
    // Given two terminal sizes.
    use nullslop_selection_widget::compute_popup_rect;

    let small_area = Rect::new(0, 0, 80, 24);
    let large_area = Rect::new(0, 0, 80, 42);

    // When computing popup rects.
    let small_popup = compute_popup_rect(small_area);
    let large_popup = compute_popup_rect(large_area);

    // Then the larger terminal gets a taller popup.
    assert!(large_popup.height > small_popup.height);
}

#[rstest::rstest]
fn small_terminal_uses_75_percent_height() {
    // Given two terminal sizes.
    use nullslop_selection_widget::compute_popup_rect;

    let small_area = Rect::new(0, 0, 80, 24);
    let large_area = Rect::new(0, 0, 80, 42);

    // When computing popup rects.
    let small_popup = compute_popup_rect(small_area);
    let _large_popup = compute_popup_rect(large_area);

    // Then the small terminal popup uses 75% of height + 4 rows of chrome.
    // floor(24 * 0.75) = 18, min(18 + 4, 24) = 22.
    assert_eq!(small_popup.height, 22);
}

#[rstest::rstest]
fn render_provider_picker_uses_dark_gray_border() {
    // Given a picker render.
    use nullslop_selection_widget::compute_popup_rect;

    let (mut state, services) = picker_state_with_ollama();
    load_picker_items(&mut state, &services);

    let (mut terminal, _area) = setup_term(80, 24);

    terminal
        .draw(|frame| {
            let area = frame.area();
            render_provider_picker(frame, area, &state);
        })
        .unwrap();

    // Then the border color is DarkGray, not Yellow.
    let buffer = terminal.backend().buffer().clone();
    let popup = compute_popup_rect(Rect::new(0, 0, 80, 24));
    let border_cell = buffer.cell((popup.x, popup.y)).expect("border cell");
    assert_eq!(border_cell.fg, Color::DarkGray);
}

#[rstest::rstest]
fn render_provider_picker_shows_active_model_marker() {
    // Given a state with active_provider set to "ollama/llama3" and items loaded.
    use nullslop_selection_widget::compute_popup_rect;

    let (mut state, services) = picker_state_with_ollama();
    state.mode = Mode::Picker;
    state.active_provider = "ollama/llama3".to_owned();
    state.active_picker_kind = Some(PickerKind::Provider);
    load_picker_items(&mut state, &services);

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering the picker.
    terminal
        .draw(|frame| {
            let area = frame.area();
            render_provider_picker(frame, area, &state);
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

// --- Context strategy picker rendering tests ---

/// Helper to create a state with strategy entries loaded.
fn strategy_picker_state() -> (nullslop_component::AppState, nullslop_services::Services) {
    let services = nullslop_services::Services::new();
    let mut state = nullslop_component::AppState::default();
    nullslop_component::context_strategy_picker::entries::load_strategy_picker_items(
        &services, &mut state,
    );
    (state, services)
}

#[rstest::rstest]
fn render_context_strategy_picker_shows_telescope_layout() {
    // Given a terminal area and picker state with entries loaded.
    use nullslop_selection_widget::compute_popup_rect;

    let (mut state, _services) = strategy_picker_state();
    state.mode = Mode::Picker;
    state.active_picker_kind = Some(PickerKind::ContextAssembly);

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
    use nullslop_selection_widget::compute_popup_rect;

    let (mut state, _services) = strategy_picker_state();
    state.mode = Mode::Picker;
    state.active_picker_kind = Some(PickerKind::ContextAssembly);

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
    use nullslop_selection_widget::compute_popup_rect;

    let (mut state, _services) = strategy_picker_state();
    state.mode = Mode::Picker;
    state.active_picker_kind = Some(PickerKind::ContextAssembly);

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

fn keymap_picker_state() -> nullslop_component::AppState {
    use nullslop_component::keymap_picker::KeymapEntry;

    let mut state = nullslop_component::AppState::default();
    let entries = vec![
        KeymapEntry {
            key_sequence: "q".to_owned(),
            description: "quit".to_owned(),
            scope: "Normal".to_owned(),
            category: "General".to_owned(),
            command: Intent::Quit,
            search_text: "q quit".to_owned(),
        },
        KeymapEntry {
            key_sequence: "gg".to_owned(),
            description: "scroll to top".to_owned(),
            scope: "Normal".to_owned(),
            category: "Navigation".to_owned(),
            command: Intent::ScrollToTop,
            search_text: "gg scroll to top".to_owned(),
        },
        KeymapEntry {
            key_sequence: "<esc>".to_owned(),
            description: "set mode normal".to_owned(),
            scope: "Picker".to_owned(),
            category: "General".to_owned(),
            command: Intent::SetMode { mode: Mode::Normal },
            search_text: "<esc> set mode normal".to_owned(),
        },
    ];
    state.keymap_picker.set_items(entries);
    state.mode = Mode::Picker;
    state.active_picker_kind = Some(PickerKind::Keymap);
    state.keymap_picker_origin_scope = Some("Normal".to_owned());
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
    state.keymap_picker_show_all = false;

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
    state.keymap_picker_show_all = true;

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

// --- Selection highlight tests ---

#[rstest::rstest]
fn cell_inside_selection_is_inverted() {
    // Given a buffer with distinctively colored cells and an active selection.
    let area = Rect::new(0, 0, 20, 10);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    // Paint a cell inside the selection with known colors.
    buf.cell_mut((3, 3)).unwrap().set_fg(Color::Yellow);
    buf.cell_mut((3, 3)).unwrap().set_bg(Color::Blue);
    // Paint a cell outside the selection with known colors.
    buf.cell_mut((15, 8)).unwrap().set_fg(Color::Red);
    buf.cell_mut((15, 8)).unwrap().set_bg(Color::Green);

    // And an app with an Active selection covering (2,2) to (5,4).
    let mut app = render_test_app();
    app.selection = SelectionState::Active {
        anchor: (2, 2),
        focus: (5, 4),
        bounds: area,
    };

    // When applying selection highlight.
    apply_selection_highlight(&app, &mut buf);

    // Then cell (3, 3) inside the selection has swapped fg/bg.
    let inside = buf.cell((3, 3)).expect("cell inside selection");
    assert_eq!(inside.fg, Color::Blue); // was bg
    assert_eq!(inside.bg, Color::Yellow); // was fg
}

#[rstest::rstest]
fn cell_outside_selection_is_unchanged() {
    // Given a buffer with distinctively colored cells and an active selection.
    let area = Rect::new(0, 0, 20, 10);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    // Paint a cell inside the selection with known colors.
    buf.cell_mut((3, 3)).unwrap().set_fg(Color::Yellow);
    buf.cell_mut((3, 3)).unwrap().set_bg(Color::Blue);
    // Paint a cell outside the selection with known colors.
    buf.cell_mut((15, 8)).unwrap().set_fg(Color::Red);
    buf.cell_mut((15, 8)).unwrap().set_bg(Color::Green);

    // And an app with an Active selection covering (2,2) to (5,4).
    let mut app = render_test_app();
    app.selection = SelectionState::Active {
        anchor: (2, 2),
        focus: (5, 4),
        bounds: area,
    };

    // When applying selection highlight.
    apply_selection_highlight(&app, &mut buf);

    // Then cell (15, 8) outside the selection is unchanged.
    let outside = buf.cell((15, 8)).expect("cell outside selection");
    assert_eq!(outside.fg, Color::Red);
    assert_eq!(outside.bg, Color::Green);
}

#[rstest::rstest]
fn cell_inside_clamped_selection_is_inverted() {
    // Given a buffer covering a large area and a selection where the raw anchor
    // extends beyond the selection's constraining bounds.
    let full_area = Rect::new(0, 0, 30, 30);
    let mut buf = ratatui::buffer::Buffer::empty(full_area);
    // Paint cell inside bounds (will be in clamped selection).
    buf.cell_mut((7, 7)).unwrap().set_fg(Color::Cyan);
    buf.cell_mut((7, 7)).unwrap().set_bg(Color::Magenta);
    // Paint cell at raw anchor position (0, 0) — outside bounds.
    buf.cell_mut((0, 0)).unwrap().set_fg(Color::White);
    buf.cell_mut((0, 0)).unwrap().set_bg(Color::Black);

    // And an Active selection with anchor outside bounds.
    // bounds=(5,5,10,10) means valid range is (5,5)-(14,14).
    // anchor=(0,0) is outside bounds, focus=(8,8) is inside.
    // selection_rect() should clamp to (5,5)-(8,8).
    let bounds = Rect::new(5, 5, 10, 10);
    let mut app = render_test_app();
    app.selection = SelectionState::Active {
        anchor: (0, 0),
        focus: (8, 8),
        bounds,
    };

    // When applying selection highlight.
    apply_selection_highlight(&app, &mut buf);

    // Then cell (7, 7) inside the clamped selection is inverted.
    let inside = buf.cell((7, 7)).expect("cell inside clamped selection");
    assert_eq!(inside.fg, Color::Magenta); // was bg
    assert_eq!(inside.bg, Color::Cyan); // was fg
}

#[rstest::rstest]
fn cell_at_raw_anchor_not_inverted() {
    // Given a buffer covering a large area and a selection where the raw anchor
    // extends beyond the selection's constraining bounds.
    let full_area = Rect::new(0, 0, 30, 30);
    let mut buf = ratatui::buffer::Buffer::empty(full_area);
    // Paint cell inside bounds (will be in clamped selection).
    buf.cell_mut((7, 7)).unwrap().set_fg(Color::Cyan);
    buf.cell_mut((7, 7)).unwrap().set_bg(Color::Magenta);
    // Paint cell at raw anchor position (0, 0) — outside bounds.
    buf.cell_mut((0, 0)).unwrap().set_fg(Color::White);
    buf.cell_mut((0, 0)).unwrap().set_bg(Color::Black);

    // And an Active selection with anchor outside bounds.
    // bounds=(5,5,10,10) means valid range is (5,5)-(14,14).
    // anchor=(0,0) is outside bounds, focus=(8,8) is inside.
    // selection_rect() should clamp to (5,5)-(8,8).
    let bounds = Rect::new(5, 5, 10, 10);
    let mut app = render_test_app();
    app.selection = SelectionState::Active {
        anchor: (0, 0),
        focus: (8, 8),
        bounds,
    };

    // When applying selection highlight.
    apply_selection_highlight(&app, &mut buf);

    // Then cell (0, 0) at the raw anchor position is NOT inverted.
    let outside = buf.cell((0, 0)).expect("cell at raw anchor");
    assert_eq!(outside.fg, Color::White); // unchanged
    assert_eq!(outside.bg, Color::Black); // unchanged
}

#[rstest::rstest]
fn selection_highlight_does_nothing_when_idle() {
    // Given a buffer with distinctly colored cells and an Idle selection.
    let area = Rect::new(0, 0, 20, 10);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    buf.cell_mut((5, 5)).unwrap().set_fg(Color::Yellow);
    buf.cell_mut((5, 5)).unwrap().set_bg(Color::Blue);

    // And an app with an Idle selection.
    let mut app = render_test_app();
    app.selection = SelectionState::Idle;

    // When applying selection highlight.
    apply_selection_highlight(&app, &mut buf);

    // Then no cells are inverted — colors remain unchanged.
    let cell = buf.cell((5, 5)).expect("colored cell");
    assert_eq!(cell.fg, Color::Yellow); // unchanged
    assert_eq!(cell.bg, Color::Blue); // unchanged
}

#[rstest::rstest]
fn reset_bg_cell_gets_explicit_colors() {
    // Given a buffer where cells have matching fg and bg (e.g. both Reset,
    // as with user messages rendered with Style::default().bold()).
    let area = Rect::new(0, 0, 20, 10);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    // User-message-style cell: fg = Reset, bg = Reset (bold modifier).
    buf.cell_mut((3, 3))
        .unwrap()
        .set_style(Style::default().add_modifier(Modifier::BOLD));
    // Adjacent cell with distinct colors (assistant-style).
    buf.cell_mut((4, 3)).unwrap().set_fg(Color::Cyan);

    // And an Active selection covering both cells.
    let mut app = render_test_app();
    app.selection = SelectionState::Active {
        anchor: (2, 2),
        focus: (5, 4),
        bounds: area,
    };

    // When applying selection highlight.
    apply_selection_highlight(&app, &mut buf);

    // Then the Reset/Reset cell gets explicit highlight colors.
    let reset_cell = buf.cell((3, 3)).expect("reset cell");
    assert_eq!(reset_cell.fg, Color::Black);
    assert_eq!(reset_cell.bg, Color::White);
}

#[rstest::rstest]
fn distinct_color_cell_gets_swapped() {
    // Given a buffer where cells have matching fg and bg (e.g. both Reset,
    // as with user messages rendered with Style::default().bold()).
    let area = Rect::new(0, 0, 20, 10);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    // User-message-style cell: fg = Reset, bg = Reset (bold modifier).
    buf.cell_mut((3, 3))
        .unwrap()
        .set_style(Style::default().add_modifier(Modifier::BOLD));
    // Adjacent cell with distinct colors (assistant-style).
    buf.cell_mut((4, 3)).unwrap().set_fg(Color::Cyan);

    // And an Active selection covering both cells.
    let mut app = render_test_app();
    app.selection = SelectionState::Active {
        anchor: (2, 2),
        focus: (5, 4),
        bounds: area,
    };

    // When applying selection highlight.
    apply_selection_highlight(&app, &mut buf);

    // Then the distinct-colors cell gets swapped fg/bg.
    let cyan_cell = buf.cell((4, 3)).expect("cyan cell");
    assert_eq!(cyan_cell.fg, Color::Reset); // was bg
    assert_eq!(cyan_cell.bg, Color::Cyan); // was fg
}

// --- Clipboard flush tests ---

#[rstest::rstest]
fn clipboard_copy_clears_pending_flag_on_idle_selection() {
    // Given an app with pending_clipboard set but Idle selection.
    let mut app = render_test_app();
    app.selection = SelectionState::Idle;
    app.pending_clipboard = true;

    let area = Rect::new(0, 0, 20, 5);
    let buf = ratatui::buffer::Buffer::empty(area);

    // When flushing the pending clipboard.
    flush_pending_clipboard(&mut app, &buf);

    // Then the pending flag is cleared (even though there was nothing to copy).
    assert!(!app.pending_clipboard);
}

#[rstest::rstest]
fn clipboard_copy_skips_empty_selection() {
    // Given an app with pending_clipboard and an Active selection over empty cells.
    let area = Rect::new(0, 0, 20, 5);
    let buf = ratatui::buffer::Buffer::empty(area);

    let mut app = render_test_app();
    app.selection = SelectionState::Active {
        anchor: (0, 0),
        focus: (3, 0),
        bounds: area,
    };
    app.pending_clipboard = true;

    // When flushing the pending clipboard.
    flush_pending_clipboard(&mut app, &buf);

    // Then the pending flag is cleared.
    assert!(!app.pending_clipboard);
}

#[rstest::rstest]
fn clipboard_clears_pending_flag_immediately() {
    // Given a buffer with known text and an active selection.
    let area = Rect::new(0, 0, 20, 5);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    // Write "Hello" on row 2.
    for (i, ch) in "Hello".chars().enumerate() {
        buf.cell_mut((2 + i as u16, 2))
            .unwrap()
            .set_symbol(&ch.to_string());
    }

    let mut app = render_test_app();
    app.selection = SelectionState::Active {
        anchor: (2, 2),
        focus: (6, 2),
        bounds: area,
    };
    app.pending_clipboard = true;

    // When flushing the pending clipboard.
    flush_pending_clipboard(&mut app, &buf);

    // Then the pending flag is cleared immediately.
    assert!(!app.pending_clipboard);
}

#[rstest::rstest]
#[ignore = "requires clipboard access (run with --ignored)"]
fn clipboard_contains_selected_text() {
    // Given a buffer with known text and an active selection.
    let area = Rect::new(0, 0, 20, 5);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    // Write "Hello" on row 2.
    for (i, ch) in "Hello".chars().enumerate() {
        buf.cell_mut((2 + i as u16, 2))
            .unwrap()
            .set_symbol(&ch.to_string());
    }

    let mut app = render_test_app();
    app.selection = SelectionState::Active {
        anchor: (2, 2),
        focus: (6, 2),
        bounds: area,
    };
    app.pending_clipboard = true;

    // When flushing the pending clipboard.
    flush_pending_clipboard(&mut app, &buf);

    // Then after the clipboard thread completes, the clipboard contains
    // the selected text.
    std::thread::sleep(std::time::Duration::from_millis(500));
    let mut clipboard = arboard::Clipboard::new().expect("clipboard access");
    let content = clipboard.get_text().expect("read clipboard");
    assert_eq!(content, "Hello");
}

// --- Element-driven selectable rect tests ---

#[rstest::rstest]
fn render_registers_content_rect_for_selectable_chat_log() {
    // Given a TuiApp rendered in Chat tab with a 80x24 terminal.

    let mut app = render_test_app();
    // Default tab is Chat.

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering.
    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .unwrap();

    // Then the content area rect is registered as selectable.
    // Chat log is selectable, so layout.content should be in selectable_rects.
    let layout = AppLayout::new(frame_area(80, 24), 1, 0);
    let found = app
        .selectable_rects
        .find_for_position(layout.content.x + 1, layout.content.y + 1);
    assert!(
        found.is_some(),
        "chat log content rect should be selectable"
    );
    assert_eq!(found.unwrap(), layout.content);
}

#[rstest::rstest]
fn picker_popup_rect_is_selectable() {
    // Given a TuiApp rendered with Mode::Picker.

    let mut app = render_test_app();
    // Switch to Picker mode with an active provider picker.
    app.core.state.write().mode = nullslop_protocol::Mode::Picker;
    app.core.state.write().active_picker_kind = Some(nullslop_protocol::PickerKind::Provider);

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering.
    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .unwrap();

    // Then the picker popup rect is registered as selectable.
    let popup_rect = compute_popup_rect(Rect::new(0, 0, 80, 24));
    // Query position (popup.x + 1, 0) — inside popup, but above the content area (y=1)
    // so the smallest matching rect is the picker popup, not the content.
    let found = app.selectable_rects.find_for_position(popup_rect.x + 1, 0);
    assert!(found.is_some(), "picker popup rect should be selectable");
    assert_eq!(found.unwrap(), popup_rect);
}

#[rstest::rstest]
fn content_area_rect_is_selectable() {
    // Given a TuiApp rendered with Mode::Picker.

    let mut app = render_test_app();
    // Switch to Picker mode with an active provider picker.
    app.core.state.write().mode = nullslop_protocol::Mode::Picker;
    app.core.state.write().active_picker_kind = Some(nullslop_protocol::PickerKind::Provider);

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering.
    terminal
        .draw(|frame| {
            app.render(frame);
        })
        .unwrap();

    // Then the content area rect is also still selectable (chat-log is selectable).
    let layout = AppLayout::new(frame_area(80, 24), 1, 0);
    let content_found = app
        .selectable_rects
        .find_for_position(layout.content.x + 1, layout.content.y + 1);
    assert!(
        content_found.is_some(),
        "content rect should also be selectable alongside picker"
    );
}

// --- Autocomplete popup rendering tests ---

/// Helper to create an `AppState` with autocomplete active.
///
/// Sets the input buffer to the given text, activates autocomplete at `token_start`,
/// and populates matches.
fn state_with_autocomplete(
    buffer_text: &str,
    token_start: usize,
    matches: Vec<nullslop_component::chat_input_box::state::AutocompleteMatch>,
) -> nullslop_component::AppState {
    let mut state = nullslop_component::AppState::default();
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
    use nullslop_component::chat_input_box::state::AutocompleteMatch;

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
    use nullslop_component::chat_input_box::state::AutocompleteMatch;

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
    use nullslop_component::chat_input_box::state::AutocompleteMatch;

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
    use nullslop_component::chat_input_box::state::AutocompleteMatch;

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
    use nullslop_component::chat_input_box::state::AutocompleteMatch;

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

    let state = nullslop_component::AppState::default();

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

/// Helper to create a Rect matching the terminal dimensions.
fn frame_area(w: u16, h: u16) -> Rect {
    Rect::new(0, 0, w, h)
}
