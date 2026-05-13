//! Status bar — displays the active prompt strategy and current model.
//!
//! Shows `<strategy> | <model>` in a single row. The strategy name comes from
//! `PromptStrategyId`'s `Display` impl (e.g., "Passthrough", "Sliding Window").
//! The model shows `({provider})/{model}` when set, or "no model selected" otherwise.

use crate::common::app_state::AppState;
use crate::common::ui_element::UiElement;
use crate::feat::provider_infra::NO_PROVIDER_ID;
use crate::feat::session::aggregate_session_stats;
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// A display element that shows the active strategy and provider/model in the status bar.
#[derive(Debug)]
pub struct StatusBarElement;

/// Format a token count in human-readable form.
#[allow(clippy::cast_precision_loss)]
fn format_tokens(count: u64) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

impl UiElement<AppState> for StatusBarElement {
    fn name(&self) -> String {
        "status-bar".to_owned()
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        let strategy = state.active_session().active_strategy();
        let pinned_count = state.active_session().pinned_entries().len();

        // Compute aggregated token stats for the active session.
        let agg = aggregate_session_stats(&state.session.sessions, &state.session.active_session);
        let up_arrow = '\u{2191}';
        let down_arrow = '\u{2193}';
        let mut token_info = format!(
            "{up_arrow}{} {down_arrow}{}",
            format_tokens(agg.total_sent()),
            format_tokens(agg.total_received()),
        );
        if let Some(ctx_size) = state.active_session().context_size() {
            token_info = format!("{} ctx:{}", token_info, format_tokens(u64::from(ctx_size)));
        }

        let left = if pinned_count > 0 {
            format!("({strategy}) \u{1f4cc}{pinned_count} {token_info}")
        } else {
            format!("({strategy}) {token_info}")
        };

        let active_model = state.active_session().profile().model.clone();
        let model = if active_model == NO_PROVIDER_ID {
            "no model selected".to_owned()
        } else if let Some((provider, model)) = active_model.split_once('/') {
            format!("({provider})/{model}")
        } else {
            active_model.clone()
        };

        let style = Style::default().fg(Color::DarkGray);

        let strategy_widget = Paragraph::new(left).style(style).alignment(Alignment::Left);
        frame.render_widget(strategy_widget, area);

        let notification = state.frontend.active_status_notification();
        let right_line = if let Some(msg) = notification {
            Line::from(vec![
                Span::styled(msg, Style::default().fg(Color::Green)),
                Span::styled(format!("  {model}"), style),
            ])
        } else {
            Line::from(Span::styled(model, style))
        };
        let model_widget = Paragraph::new(right_line).alignment(Alignment::Right);
        frame.render_widget(model_widget, area);
    }
}

#[cfg(test)]
mod tests {
    use nullslop_testutil::{buffer_row, setup_term};

    use super::*;
    use crate::common::app_state::{AppState, StatusNotification};

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
        let row = buffer_row(&buffer, 0, 50);
        assert!(row.starts_with("(Passthrough)"));
        assert!(row.contains("no model selected"));
    }

    #[rstest::rstest]
    fn render_shows_provider_and_model() {
        let mut element = StatusBarElement;
        let mut state = AppState::default();
        state
            .active_session_mut()
            .set_model("ollama/llama3".to_owned());
        let (mut terminal, area) = setup_term(50, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = buffer_row(&buffer, 0, 50);
        assert!(row.starts_with("(Passthrough)"));
        assert!(row.contains("(ollama)/llama3"));
    }

    #[rstest::rstest]
    fn render_right_aligns_text() {
        let mut element = StatusBarElement;
        let mut state = AppState::default();
        state
            .active_session_mut()
            .set_model("ollama/llama3".to_owned());
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
        let mut state = AppState::default();
        state
            .active_session_mut()
            .set_model("ollama/llama3".to_owned());
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
        let mut state = AppState::default();
        state
            .active_session_mut()
            .set_model("openrouter/anthropic/claude-sonnet-4".to_owned());
        let (mut terminal, area) = setup_term(80, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = buffer_row(&buffer, 0, 80);
        assert!(row.starts_with("(Passthrough)"));
        assert!(row.contains("(openrouter)/anthropic/claude-sonnet-4"));
    }

    #[rstest::rstest]
    fn render_shows_non_default_strategy() {
        let mut element = StatusBarElement;
        let mut state = AppState::default();
        state
            .active_session_mut()
            .set_model("ollama/llama3".to_owned());
        state
            .active_session_mut()
            .switch_strategy(crate::protocol::PromptStrategyId::sliding_window());
        let (mut terminal, area) = setup_term(50, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = buffer_row(&buffer, 0, 50);
        assert!(row.starts_with("(Sliding Window)"));
        assert!(row.contains("(ollama)/llama3"));
    }

    #[rstest::rstest]
    fn render_shows_pinned_count_when_entries_pinned() {
        let mut element = StatusBarElement;
        let mut state = AppState::default();
        state
            .active_session_mut()
            .set_model("ollama/llama3".to_owned());
        let idx = state
            .active_session_mut()
            .push_entry(crate::protocol::ChatEntry::user("hello"));
        let entry_id = state.active_session().history()[idx].id.clone();
        state
            .active_session_mut()
            .pin_entry(&entry_id, crate::protocol::PinPosition::Relative);
        let (mut terminal, area) = setup_term(60, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = buffer_row(&buffer, 0, 60);
        assert!(row.contains("\u{1f4cc}"));
        assert!(row.contains('1'));
    }

    #[rstest::rstest]
    fn render_hides_pinned_count_when_no_entries_pinned() {
        let mut element = StatusBarElement;
        let mut state = AppState::default();
        state
            .active_session_mut()
            .set_model("ollama/llama3".to_owned());
        let (mut terminal, area) = setup_term(60, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = buffer_row(&buffer, 0, 60);
        assert!(row.starts_with("(Passthrough) "));
        assert!(!row.contains("\u{1f4cc}"));
    }

    #[rstest::rstest]
    fn render_shows_notification_when_active() {
        // Given a state with a notification and a model.
        let mut element = StatusBarElement;
        let mut state = AppState::default();
        state
            .active_session_mut()
            .set_model("ollama/llama3".to_owned());
        state
            .frontend
            .set_status_notification("Copied to clipboard");
        let (mut terminal, area) = setup_term(80, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = buffer_row(&buffer, 0, 80);
        // Then the notification text appears in the right portion.
        assert!(row.contains("Copied to clipboard"));
        // And the model is still shown.
        assert!(row.contains("(ollama)/llama3"));
    }

    #[rstest::rstest]
    fn render_notification_uses_green_color() {
        // Given a state with an active notification.
        let mut element = StatusBarElement;
        let mut state = AppState::default();
        state
            .active_session_mut()
            .set_model("ollama/llama3".to_owned());
        state.frontend.set_status_notification("Copied!");
        let (mut terminal, area) = setup_term(80, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();

        // Find a cell in the notification text ("Copied!") that has green fg.
        // The notification is right-aligned, so scan right side for 'C'.
        let green_cell = (0..80)
            .filter_map(|x| buffer.cell((x, 0)))
            .find(|c| c.symbol() == "C" && c.fg == Color::Green);
        assert!(green_cell.is_some(), "notification text should be green");
    }

    #[rstest::rstest]
    fn render_no_notification_shows_model_only() {
        // Given a state with no notification.
        let mut element = StatusBarElement;
        let mut state = AppState::default();
        state
            .active_session_mut()
            .set_model("ollama/llama3".to_owned());
        let (mut terminal, area) = setup_term(80, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = buffer_row(&buffer, 0, 80);
        // Then only the model is shown on the right.
        assert!(row.contains("(ollama)/llama3"));
        assert!(!row.contains("Copied"));
    }

    #[rstest::rstest]
    fn render_expired_notification_not_shown() {
        // Given a state with an expired notification.
        let mut element = StatusBarElement;
        let mut state = AppState::default();
        state
            .active_session_mut()
            .set_model("ollama/llama3".to_owned());
        state.frontend.status_notification = Some(StatusNotification {
            message: "old msg".to_owned(),
            created_at: std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(5))
                .unwrap(),
        });
        let (mut terminal, area) = setup_term(80, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = buffer_row(&buffer, 0, 80);
        // Then the notification is not shown.
        assert!(!row.contains("old msg"));
        // And the model is still shown normally.
        assert!(row.contains("(ollama)/llama3"));
    }

    // --- Token display tests ---

    #[rstest::rstest]
    fn render_shows_token_counts_with_zero_values() {
        // Given a state with no token records.
        let mut element = StatusBarElement;
        let state = AppState::default();
        let (mut terminal, area) = setup_term(80, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = buffer_row(&buffer, 0, 80);
        // Then the status bar shows zero token counts.
        assert!(row.contains("\u{2191}0 \u{2193}0"));
    }

    #[rstest::rstest]
    fn render_shows_token_counts_with_values() {
        // Given a session with token records.
        use crate::feat::session::token_stats::TokenRecord;
        let mut element = StatusBarElement;
        let mut state = AppState::default();
        state
            .active_session_mut()
            .set_model("ollama/llama3".to_owned());
        state.active_session_mut().push_token_record(TokenRecord {
            timestamp: jiff::Timestamp::now(),
            tokens_sent: 1500,
            tokens_received: 750,
        });
        let (mut terminal, area) = setup_term(80, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = buffer_row(&buffer, 0, 80);
        // Then the status bar shows token counts.
        assert!(row.contains("1.5k"));
        assert!(row.contains("750"));
    }

    #[rstest::rstest]
    fn render_shows_context_size_when_cached() {
        // Given a session with a cached context size.
        use crate::feat::session::token_stats::TokenRecord;
        let mut element = StatusBarElement;
        let mut state = AppState::default();
        state
            .active_session_mut()
            .set_model("ollama/llama3".to_owned());
        state.active_session_mut().push_token_record(TokenRecord {
            timestamp: jiff::Timestamp::now(),
            tokens_sent: 5000,
            tokens_received: 0,
        });
        state.active_session_mut().set_context_size(5000);
        let (mut terminal, area) = setup_term(80, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = buffer_row(&buffer, 0, 80);
        // Then the status bar shows ctx:5.0k.
        assert!(row.contains("ctx:5.0k"));
    }

    #[rstest::rstest]
    fn render_hides_context_size_when_not_cached() {
        // Given a session with no cached context size.
        let mut element = StatusBarElement;
        let state = AppState::default();
        let (mut terminal, area) = setup_term(80, 1);
        terminal
            .draw(|frame| {
                element.render(frame, area, &state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = buffer_row(&buffer, 0, 80);
        // Then ctx: is not shown.
        assert!(!row.contains("ctx:"));
        // But token counts are still shown.
        assert!(row.contains("0 0") || row.contains("\u{2191}0 \u{2193}0"));
    }
}
