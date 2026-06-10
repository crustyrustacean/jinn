#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

//! Provider picker render tests.

use crate::common::app_state::{AppState, FocusScope};
use crate::common::render_ctx::RenderCtx;
use crate::common::services::Services;
use crate::feat::provider_infra::{ProviderEntry, ProvidersConfig};
use crate::protocol::PickerKind;
use jinn_selection_widget::compute_popup_rect;
use jinn_testutil::setup_term;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::loader::load_provider_picker_items;
use super::render::render_provider_picker;

fn picker_state_with_ollama() -> (AppState, Services) {
    let config = ProvidersConfig {
        providers: vec![ProviderEntry {
            name: "ollama".to_owned(),
            backend: "ollama".to_owned(),
            models: vec!["llama3".to_owned()],
            base_url: Some("http://localhost:11434".to_owned()),
            api_key_env: None,
            requires_key: false,
            extra_body: None,
            context_length: None,
        }],
        aliases: vec![],
        default_provider: None,
    };
    let services = crate::common::services::test_services::TestServices::builder()
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
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Provider,
    });
    load_picker_items(&mut state, &services);
    state.provider.provider_picker.insert_char('o');
    state.provider.provider_picker.insert_char('l');

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering the picker.
    terminal
        .draw(|frame| {
            let area = frame.area();
            let ctx = RenderCtx::new(&state);
            render_provider_picker(frame, area, &ctx);
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
            let ctx = RenderCtx::new(&state);
            render_provider_picker(frame, area, &ctx);
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
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Provider,
    });
    state
        .active_session_mut()
        .set_model("ollama/llama3".to_owned());
    load_picker_items(&mut state, &services);

    let (mut terminal, _area) = setup_term(80, 24);

    // When rendering the picker.
    terminal
        .draw(|frame| {
            let area = frame.area();
            let ctx = RenderCtx::new(&state);
            render_provider_picker(frame, area, &ctx);
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
