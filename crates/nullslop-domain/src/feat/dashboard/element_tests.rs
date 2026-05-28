#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

use crate::common::app_state::AppState;
use crate::common::ui_element::UiElement;
use crate::feat::dashboard::element::DashboardElement;
use nullslop_testutil::setup_term;
use ratatui::style::Color;

fn render_rows(
    element: &mut DashboardElement,
    state: &AppState,
    width: u16,
    height: u16,
) -> Vec<String> {
    let (mut terminal, area) = setup_term(width, height);
    terminal
        .draw(|frame| {
            element.render(frame, area, state);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| {
                    buffer
                        .cell((x, y))
                        .map_or(" ", ratatui::buffer::Cell::symbol)
                })
                .collect()
        })
        .collect()
}

#[rstest::rstest]
fn name_returns_dashboard() {
    // Given a DashboardElement.
    let element = DashboardElement;

    // When querying the name.
    let name = element.name();

    // Then it is "dashboard".
    assert_eq!(name, "dashboard");
}

#[rstest::rstest]
fn render_empty_shows_no_actors() {
    // Given a DashboardElement with no actors.
    let mut element = DashboardElement;
    let state = AppState::default();

    // When rendering.
    let rows = render_rows(&mut element, &state, 40, 10);

    // Then "No actors registered." appears.
    assert!(rows[0].contains("No actors registered."));
}

#[rstest::rstest]
fn actor_name_and_status_appear_on_first_line() {
    // Given a DashboardElement with an actor in Starting status.
    let mut element = DashboardElement;
    let state = {
        let mut s = AppState::default();
        s.frontend
            .dashboard
            .mark_starting("echo", Some("Echoes messages back".to_owned()));
        s
    };

    // When rendering.
    let rows = render_rows(&mut element, &state, 40, 10);

    // Then the actor name and status appear on the first line.
    assert!(rows[0].contains("echo"));
    assert!(rows[0].contains("Starting"));
}

#[rstest::rstest]
fn actor_description_appears_on_next_line() {
    // Given a DashboardElement with an actor in Starting status.
    let mut element = DashboardElement;
    let state = {
        let mut s = AppState::default();
        s.frontend
            .dashboard
            .mark_starting("echo", Some("Echoes messages back".to_owned()));
        s
    };

    // When rendering.
    let rows = render_rows(&mut element, &state, 40, 10);

    // And the description appears on the next line in light gray.
    assert!(rows[1].contains("Echoes messages back"));
}

#[rstest::rstest]
fn render_actor_with_running_status() {
    // Given a DashboardElement with an actor in Running status.
    let mut element = DashboardElement;
    let state = {
        let mut s = AppState::default();
        s.frontend
            .dashboard
            .mark_starting("echo", Some("Echoes messages back".to_owned()));
        s.frontend.dashboard.mark_running("echo", None);
        s
    };

    // When rendering.
    let rows = render_rows(&mut element, &state, 40, 10);

    // Then the actor name and Running status appear.
    assert!(rows[0].contains("echo"));
    assert!(rows[0].contains("Running"));
}

#[rstest::rstest]
fn render_actor_without_description() {
    // Given a DashboardElement with an actor that has no description.
    let mut element = DashboardElement;
    let state = {
        let mut s = AppState::default();
        s.frontend.dashboard.mark_starting("actor-a", None);
        s
    };

    // When rendering.
    let rows = render_rows(&mut element, &state, 40, 10);

    // Then the actor name and status appear with no description line.
    assert!(rows[0].contains("actor-a"));
    assert!(rows[0].contains("Starting"));
}

#[rstest::rstest]
fn blank_line_between_actors() {
    // Given two actors.
    let mut element = DashboardElement;
    let state = {
        let mut s = AppState::default();
        s.frontend
            .dashboard
            .mark_starting("echo", Some("Echo".to_owned()));
        s.frontend
            .dashboard
            .mark_starting("llm", Some("LLM".to_owned()));
        s
    };

    let rows = render_rows(&mut element, &state, 40, 10);
    // There should be a blank line between actors
    assert!(rows[2].trim().is_empty());
}

#[rstest::rstest]
fn second_actor_renders() {
    // Given two actors.
    let mut element = DashboardElement;
    let state = {
        let mut s = AppState::default();
        s.frontend
            .dashboard
            .mark_starting("echo", Some("Echo".to_owned()));
        s.frontend
            .dashboard
            .mark_starting("llm", Some("LLM".to_owned()));
        s
    };

    let rows = render_rows(&mut element, &state, 40, 10);
    assert!(rows[3].contains("llm"));
}

#[rstest::rstest]
fn selected_entry_name_has_yellow_marker() {
    // Given two actors with the first selected (default index 0).
    let mut element = DashboardElement;
    let state = {
        let mut s = AppState::default();
        s.frontend
            .dashboard
            .mark_starting("echo", Some("Echoes messages back".to_owned()));
        s.frontend
            .dashboard
            .mark_starting("llm", Some("LLM streaming".to_owned()));
        s
    };

    let (mut terminal, area) = setup_term(40, 10);
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the first entry has a yellow full block at columns 0-1.
    let buffer = terminal.backend().buffer();
    let cell0 = buffer.cell((0, 0)).expect("cell 0,0");
    let cell1 = buffer.cell((1, 0)).expect("cell 1,0");
    assert_eq!(cell0.symbol(), "\u{2588}");
    assert_eq!(cell0.fg, Color::Yellow);
    assert_eq!(cell1.symbol(), "\u{2588}");
    assert_eq!(cell1.fg, Color::Yellow);
}

#[rstest::rstest]
fn selected_entry_description_has_yellow_marker() {
    // Given two actors with the first selected (default index 0).
    let mut element = DashboardElement;
    let state = {
        let mut s = AppState::default();
        s.frontend
            .dashboard
            .mark_starting("echo", Some("Echoes messages back".to_owned()));
        s.frontend
            .dashboard
            .mark_starting("llm", Some("LLM streaming".to_owned()));
        s
    };

    let (mut terminal, area) = setup_term(40, 10);
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // And the description line also has the yellow block.
    let buffer = terminal.backend().buffer();
    let desc_cell0 = buffer.cell((0, 1)).expect("cell 0,1");
    assert_eq!(desc_cell0.symbol(), "\u{2588}");
    assert_eq!(desc_cell0.fg, Color::Yellow);
}

#[rstest::rstest]
fn render_unselected_entry_shows_spaces() {
    // Given two actors with the first selected (default index 0).
    let mut element = DashboardElement;
    let state = {
        let mut s = AppState::default();
        s.frontend
            .dashboard
            .mark_starting("echo", Some("Echoes messages back".to_owned()));
        s.frontend
            .dashboard
            .mark_starting("llm", Some("LLM streaming".to_owned()));
        s
    };

    // When rendering.
    let (mut terminal, area) = setup_term(40, 10);
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the second entry (row 3) has spaces at columns 0-1.
    let buffer = terminal.backend().buffer();
    let cell0 = buffer.cell((0, 3)).expect("cell 0,3");
    let cell1 = buffer.cell((1, 3)).expect("cell 1,3");
    assert_eq!(cell0.symbol(), " ");
    assert_eq!(cell1.symbol(), " ");
}

#[rstest::rstest]
fn unselected_entry_has_spaces() {
    // Given two actors with the second selected.
    let mut element = DashboardElement;
    let state = {
        let mut s = AppState::default();
        s.frontend
            .dashboard
            .mark_starting("echo", Some("Echoes messages back".to_owned()));
        s.frontend
            .dashboard
            .mark_starting("llm", Some("LLM streaming".to_owned()));
        s.frontend.dashboard.select_next();
        s
    };

    let (mut terminal, area) = setup_term(40, 10);
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // Then the first entry has spaces (unselected).
    let buffer = terminal.backend().buffer();
    let first_cell = buffer.cell((0, 0)).expect("cell 0,0");
    assert_eq!(first_cell.symbol(), " ");
}

#[rstest::rstest]
fn selected_entry_has_yellow_block() {
    // Given two actors with the second selected.
    let mut element = DashboardElement;
    let state = {
        let mut s = AppState::default();
        s.frontend
            .dashboard
            .mark_starting("echo", Some("Echoes messages back".to_owned()));
        s.frontend
            .dashboard
            .mark_starting("llm", Some("LLM streaming".to_owned()));
        s.frontend.dashboard.select_next();
        s
    };

    let (mut terminal, area) = setup_term(40, 10);
    terminal
        .draw(|frame| {
            element.render(frame, area, &state);
        })
        .unwrap();

    // And the second entry (row 3) has the yellow block (selected).
    let buffer = terminal.backend().buffer();
    let second_cell = buffer.cell((0, 3)).expect("cell 0,3");
    assert_eq!(second_cell.symbol(), "\u{2588}");
    assert_eq!(second_cell.fg, Color::Yellow);
}

#[rstest::rstest]
fn dashboard_element_is_selectable() {
    // Given a DashboardElement.
    let element = DashboardElement;

    // When calling is_selectable.
    let selectable: &dyn UiElement<AppState> = &element;

    // Then it returns true.
    assert!(selectable.is_selectable());
}
