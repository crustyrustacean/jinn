//! Status bar — displays the active prompt strategy and current model.
//!
//! Shows `<strategy> | <model>` in a single row. The strategy name comes from
//! `PromptStrategyId`'s `Display` impl (e.g., "Passthrough", "Sliding Window").
//! The model shows `({provider})/{model}` when set, or "no model selected" otherwise.

use nullslop_component::AppState;
use nullslop_component_ui::UiElement;
use nullslop_providers::NO_PROVIDER_ID;
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::Paragraph;

/// A display element that shows the active strategy and provider/model in the status bar.
#[derive(Debug)]
pub struct StatusBarElement;

impl UiElement<AppState> for StatusBarElement {
    fn name(&self) -> String {
        "status-bar".to_owned()
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        let strategy = state.active_session().active_strategy();
        let pinned_count = state.active_session().pinned_entries().len();

        let left = if pinned_count > 0 {
            format!("({strategy}) \u{1f4cc}{pinned_count}")
        } else {
            format!("({strategy})")
        };

        let model = if state.provider.active_provider == NO_PROVIDER_ID {
            "no model selected".to_owned()
        } else if let Some((provider, model)) = state.provider.active_provider.split_once('/') {
            format!("({provider})/{model}")
        } else {
            state.provider.active_provider.clone()
        };

        let style = Style::default().fg(Color::DarkGray);

        let strategy_widget = Paragraph::new(left).style(style).alignment(Alignment::Left);
        frame.render_widget(strategy_widget, area);

        let model_widget = Paragraph::new(model)
            .style(style)
            .alignment(Alignment::Right);
        frame.render_widget(model_widget, area);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    fn setup_term(width: u16, height: u16) -> (Terminal<TestBackend>, Rect) {
        let backend = TestBackend::new(width, height);
        let terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, width, height);
        (terminal, area)
    }

    use super::*;
    use nullslop_component::{AppState, ProviderState};

    #[rstest::rstest]
    fn name_returns_status_bar() {
        let element = StatusBarElement;
        assert_eq!(element.name(), "status-bar");
    }

    #[rstest::rstest]
    fn render_shows_no_model_selected_when_unset() {
        let mut element = StatusBarElement;
        let state = AppState::default();
        let (mut terminal, area) = setup_term(50, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row: String = (0..50)
            .filter_map(|x| buffer.cell((x, 0)).map(ratatui::buffer::Cell::symbol))
            .collect();
        assert!(row.starts_with("(Passthrough)"));
        assert!(row.contains("no model selected"));
    }

    #[rstest::rstest]
    fn render_shows_provider_and_model() {
        let mut element = StatusBarElement;
        let state = AppState {
            provider: ProviderState {
                active_provider: "ollama/llama3".to_owned(),
                ..ProviderState::default()
            },
            ..AppState::default()
        };
        let (mut terminal, area) = setup_term(50, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row: String = (0..50)
            .filter_map(|x| buffer.cell((x, 0)).map(ratatui::buffer::Cell::symbol))
            .collect();
        assert!(row.starts_with("(Passthrough)"));
        assert!(row.contains("(ollama)/llama3"));
    }

    #[rstest::rstest]
    fn render_right_aligns_text() {
        let mut element = StatusBarElement;
        let state = AppState {
            provider: ProviderState {
                active_provider: "ollama/llama3".to_owned(),
                ..ProviderState::default()
            },
            ..AppState::default()
        };
        let (mut terminal, area) = setup_term(50, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let first = buffer.cell((0, 0)).expect("first cell");
        assert_eq!(first.symbol(), "(");
        let last = buffer.cell((49, 0)).expect("last cell");
        assert_eq!(last.symbol(), "3");
    }

    #[rstest::rstest]
    fn render_uses_gray_color() {
        let mut element = StatusBarElement;
        let state = AppState {
            provider: ProviderState {
                active_provider: "ollama/llama3".to_owned(),
                ..ProviderState::default()
            },
            ..AppState::default()
        };
        let (mut terminal, area) = setup_term(40, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text_cell = (0..40)
            .filter_map(|x| buffer.cell((x, 0)))
            .find(|c| c.symbol() != " ")
            .expect("should have text cell");
        assert_eq!(text_cell.fg, Color::DarkGray);
    }

    #[rstest::rstest]
    fn render_shows_provider_with_slash_in_model() {
        let mut element = StatusBarElement;
        let state = AppState {
            provider: ProviderState {
                active_provider: "openrouter/anthropic/claude-sonnet-4".to_owned(),
                ..ProviderState::default()
            },
            ..AppState::default()
        };
        let (mut terminal, area) = setup_term(80, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row: String = (0..80)
            .filter_map(|x| buffer.cell((x, 0)).map(ratatui::buffer::Cell::symbol))
            .collect();
        assert!(row.starts_with("(Passthrough)"));
        assert!(row.contains("(openrouter)/anthropic/claude-sonnet-4"));
    }

    #[rstest::rstest]
    fn render_shows_non_default_strategy() {
        let mut element = StatusBarElement;
        let mut state = AppState {
            provider: ProviderState {
                active_provider: "ollama/llama3".to_owned(),
                ..ProviderState::default()
            },
            ..AppState::default()
        };
        state
            .active_session_mut()
            .switch_strategy(nullslop_protocol::PromptStrategyId::sliding_window());
        let (mut terminal, area) = setup_term(50, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row: String = (0..50)
            .filter_map(|x| buffer.cell((x, 0)).map(ratatui::buffer::Cell::symbol))
            .collect();
        assert!(row.starts_with("(Sliding Window)"));
        assert!(row.contains("(ollama)/llama3"));
    }

    #[rstest::rstest]
    fn render_shows_pinned_count_when_entries_pinned() {
        let mut element = StatusBarElement;
        let mut state = AppState {
            provider: ProviderState {
                active_provider: "ollama/llama3".to_owned(),
                ..ProviderState::default()
            },
            ..AppState::default()
        };
        let idx = state
            .active_session_mut()
            .push_entry(nullslop_protocol::ChatEntry::user("hello"));
        let entry_id = state.active_session().history()[idx].id.clone();
        state
            .active_session_mut()
            .pin_entry(&entry_id, nullslop_protocol::PinPosition::Relative);
        let (mut terminal, area) = setup_term(60, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row: String = (0..60)
            .filter_map(|x| buffer.cell((x, 0)).map(ratatui::buffer::Cell::symbol))
            .collect();
        assert!(row.contains("\u{1f4cc}"));
        assert!(row.contains('1'));
    }

    #[rstest::rstest]
    fn render_hides_pinned_count_when_no_entries_pinned() {
        let mut element = StatusBarElement;
        let state = AppState {
            provider: ProviderState {
                active_provider: "ollama/llama3".to_owned(),
                ..ProviderState::default()
            },
            ..AppState::default()
        };
        let (mut terminal, area) = setup_term(60, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row: String = (0..60)
            .filter_map(|x| buffer.cell((x, 0)).map(ratatui::buffer::Cell::symbol))
            .collect();
        assert!(row.starts_with("(Passthrough) "));
        assert!(!row.contains("\u{1f4cc}"));
    }
}
