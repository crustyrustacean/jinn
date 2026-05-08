//! Status bar — displays the active prompt strategy and current model.
//!
//! Shows `<strategy> | <model>` in a single row. The strategy name comes from
//! [`PromptStrategyId`]'s `Display` impl (e.g., "Passthrough", "Sliding Window").
//! The model shows `({provider})/{model}` when set, or "no model selected" otherwise.

use crate::AppState;
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

        let model = if state.active_provider == NO_PROVIDER_ID {
            "no model selected".to_owned()
        } else if let Some((provider, model)) = state.active_provider.split_once('/') {
            format!("({provider})/{model}")
        } else {
            state.active_provider.clone()
        };

        let style = Style::default().fg(Color::DarkGray);

        let strategy_widget =
            Paragraph::new(left).style(style).alignment(Alignment::Left);
        frame.render_widget(strategy_widget, area);

        let model_widget = Paragraph::new(model).style(style).alignment(Alignment::Right);
        frame.render_widget(model_widget, area);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;

    use super::*;
    use crate::AppState;

    #[test]
    fn name_returns_status_bar() {
        // Given a StatusBarElement.
        let element = StatusBarElement;

        // When querying the name.
        let name = element.name();

        // Then it is "status-bar".
        assert_eq!(name, "status-bar");
    }

    #[test]
    fn render_shows_no_model_selected_when_unset() {
        // Given a StatusBarElement with default (no provider) state.
        let mut element = StatusBarElement;
        let state = AppState::default();

        let backend = TestBackend::new(50, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 50, 1);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the left side shows "(Passthrough)" and the right side shows "no model selected".
        let buffer = terminal.backend().buffer().clone();
        let row: String = (0..50)
            .filter_map(|x| buffer.cell((x, 0)).map(ratatui::buffer::Cell::symbol))
            .collect();
        assert!(row.starts_with("(Passthrough)"), "should show '(Passthrough)' on the left, got: {row}");
        assert!(row.contains("no model selected"), "should show 'no model selected' on the right, got: {row}");
    }

    #[test]
    fn render_shows_provider_and_model() {
        // Given a StatusBarElement with active_provider = "ollama/llama3".
        let mut element = StatusBarElement;
        let state = AppState { active_provider: "ollama/llama3".to_owned(), ..AppState::default() };

        let backend = TestBackend::new(50, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 50, 1);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the left side shows "(Passthrough)" and the right side shows "(ollama)/llama3".
        let buffer = terminal.backend().buffer().clone();
        let row: String = (0..50)
            .filter_map(|x| buffer.cell((x, 0)).map(ratatui::buffer::Cell::symbol))
            .collect();
        assert!(row.starts_with("(Passthrough)"), "should show '(Passthrough)' on the left, got: {row}");
        assert!(row.contains("(ollama)/llama3"), "should show '(ollama)/llama3' on the right, got: {row}");
    }

    #[test]
    fn render_right_aligns_text() {
        // Given a StatusBarElement with a short provider string in a wide area.
        let mut element = StatusBarElement;
        let state = AppState { active_provider: "ollama/llama3".to_owned(), ..AppState::default() };

        let backend = TestBackend::new(50, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 50, 1);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the model is right-aligned and the strategy is left-aligned.
        let buffer = terminal.backend().buffer().clone();
        // Strategy "(Passthrough)" is left-aligned — starts at column 0.
        let first = buffer.cell((0, 0)).expect("first cell");
        assert_eq!(first.symbol(), "(", "strategy should start at column 0");
        // Model "(ollama)/llama3" is right-aligned — last char at column 49.
        let last = buffer.cell((49, 0)).expect("last cell");
        assert_eq!(last.symbol(), "3", "model should end at column 49");
    }

    #[test]
    fn render_uses_gray_color() {
        // Given a StatusBarElement with a provider set.
        let mut element = StatusBarElement;
        let state = AppState { active_provider: "ollama/llama3".to_owned(), ..AppState::default() };

        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 40, 1);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the text color is Gray.
        let buffer = terminal.backend().buffer().clone();
        // Find a non-space cell.
        let text_cell = (0..40)
            .filter_map(|x| buffer.cell((x, 0)))
            .find(|c| c.symbol() != " ")
            .expect("should have text cell");
        assert_eq!(text_cell.fg, Color::DarkGray, "text should be dark gray");
    }

    #[test]
    fn render_shows_provider_with_slash_in_model() {
        // Given a StatusBarElement with active_provider = "openrouter/anthropic/claude-sonnet-4".
        let mut element = StatusBarElement;
        let state = AppState { active_provider: "openrouter/anthropic/claude-sonnet-4".to_owned(), ..AppState::default() };

        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 80, 1);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the left side shows "(Passthrough)" and the right side shows "(openrouter)/anthropic/claude-sonnet-4".
        let buffer = terminal.backend().buffer().clone();
        let row: String = (0..80)
            .filter_map(|x| buffer.cell((x, 0)).map(ratatui::buffer::Cell::symbol))
            .collect();
        assert!(row.starts_with("(Passthrough)"), "should show '(Passthrough)' on the left, got: {row}");
        assert!(row.contains("(openrouter)/anthropic/claude-sonnet-4"), "should show '(openrouter)/anthropic/claude-sonnet-4' on the right, got: {row}");
    }

    #[test]
    fn render_shows_non_default_strategy() {
        // Given a session with sliding_window strategy and an active provider.
        let mut element = StatusBarElement;
        let mut state = AppState { active_provider: "ollama/llama3".to_owned(), ..AppState::default() };
        state.active_session_mut().switch_strategy(nullslop_protocol::PromptStrategyId::sliding_window());

        let backend = TestBackend::new(50, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 50, 1);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the left side shows "(Sliding Window)" and the right side shows "(ollama)/llama3".
        let buffer = terminal.backend().buffer().clone();
        let row: String = (0..50)
            .filter_map(|x| buffer.cell((x, 0)).map(ratatui::buffer::Cell::symbol))
            .collect();
        assert!(row.starts_with("(Sliding Window)"), "should show '(Sliding Window)' on the left, got: {row}");
        assert!(row.contains("(ollama)/llama3"), "should show '(ollama)/llama3' on the right, got: {row}");
    }

    #[test]
    fn render_strategy_surrounded_by_parens() {
        // Given default state with no provider.
        let mut element = StatusBarElement;
        let state = AppState::default();

        let backend = TestBackend::new(50, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 50, 1);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the strategy name is surrounded by parens on the left side.
        let buffer = terminal.backend().buffer().clone();
        let first = buffer.cell((0, 0)).expect("first cell");
        assert_eq!(first.symbol(), "(", "strategy should start with '('");
        // "(Passthrough)" is 13 chars.
        let closing = buffer.cell((12, 0)).expect("closing paren cell");
        assert_eq!(closing.symbol(), ")", "strategy should end with ')'");
    }

    #[test]
    fn render_shows_pinned_count_when_entries_pinned() {
        // Given a session with a pinned entry.
        let mut element = StatusBarElement;
        let mut state = AppState { active_provider: "ollama/llama3".to_owned(), ..AppState::default() };
        let idx = state.active_session_mut().push_entry(
            nullslop_protocol::ChatEntry::user("hello"),
        );
        let entry_id = state.active_session().history()[idx].id.clone();
        state.active_session_mut().pin_entry(&entry_id, nullslop_protocol::PinPosition::Relative);

        let backend = TestBackend::new(60, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 60, 1);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the left side contains the pin emoji and count.
        let buffer = terminal.backend().buffer().clone();
        let row: String = (0..60)
            .filter_map(|x| buffer.cell((x, 0)).map(ratatui::buffer::Cell::symbol))
            .collect();
        assert!(row.contains("\u{1f4cc}"), "should show pin emoji, got: {row}");
        assert!(row.contains("1"), "should show pin count, got: {row}");
    }

    #[test]
    fn render_hides_pinned_count_when_no_entries_pinned() {
        // Given a session with no pinned entries.
        let mut element = StatusBarElement;
        let state = AppState { active_provider: "ollama/llama3".to_owned(), ..AppState::default() };

        let backend = TestBackend::new(60, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 60, 1);

        // When rendering.
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();

        // Then the left side shows just the strategy, no pin count.
        let buffer = terminal.backend().buffer().clone();
        let row: String = (0..60)
            .filter_map(|x| buffer.cell((x, 0)).map(ratatui::buffer::Cell::symbol))
            .collect();
        assert!(row.starts_with("(Passthrough) "), "should show '(Passthrough)' with no pin count, got: {row}");
        assert!(!row.contains("\u{1f4cc}"), "should not show pin emoji when no entries pinned");
    }
}
