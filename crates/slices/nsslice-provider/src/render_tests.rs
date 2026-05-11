//! Provider picker render tests.

use nullslop_component::AppState;
use nullslop_providers::{ProviderEntry, ProvidersConfig};
use nullslop_protocol::{Mode, PickerKind};
use nullslop_selection_widget::compute_popup_rect;
use nullslop_services::Services;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::loader::load_provider_picker_items;
use crate::render::render_provider_picker;

/// Creates a test terminal with the given dimensions.
fn setup_term(width: u16, height: u16) -> (Terminal<TestBackend>, Rect) {
    let backend = TestBackend::new(width, height);
    let terminal = Terminal::new(backend).unwrap();
    let area = Rect::new(0, 0, width, height);
    (terminal, area)
}

fn picker_state_with_ollama() -> (AppState, Services) {
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
    (AppState::default(), services)
}

/// Helper to load provider entries into the picker state.
fn load_picker_items(state: &mut AppState, services: &Services) {
    load_provider_picker_items(services, state);
}

#[rstest::rstest]
fn render_provider_picker_shows_telescope_layout() {
    // Given a terminal area and picker state with filter "ol".

    let (mut state, services) = picker_state_with_ollama();
    state.frontend.mode = Mode::Picker;
    state.frontend.active_picker_kind = Some(PickerKind::Provider);
    load_picker_items(&mut state, &services);
    state.provider.provider_picker.insert_char('o');
    state.provider.provider_picker.insert_char('l');

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
fn render_provider_picker_uses_dark_gray_border() {
    // Given a picker render.

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

    let (mut state, services) = picker_state_with_ollama();
    state.frontend.mode = Mode::Picker;
    state.provider.active_provider = "ollama/llama3".to_owned();
    state.frontend.active_picker_kind = Some(PickerKind::Provider);
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
