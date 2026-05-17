//! Status bar — displays the active prompt strategy and current model.
//!
//! Shows `<strategy> | <model>` in a single row. The strategy name comes from
//! `PromptStrategyId`'s `Display` impl (e.g., "Passthrough", "Sliding Window").
//! The model shows `({provider})/{model}` when set, or "no model selected" otherwise.

use crate::common::app_state::AppState;
use crate::common::ui_element::UiElement;
use crate::feat::provider_infra::NO_PROVIDER_ID;
use crate::feat::session::aggregate_session_stats;
use crate::feat::ui::status_bar::turn_counter;
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
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

        let style = Style::default().fg(state.frontend.theme.muted_text);

        // Build left side: strategy info + turn count.
        let turn_count = turn_counter::compute_turn_count(state.active_session().history());
        let left_spans: Vec<Span> = vec![
            Span::styled(left, style),
            Span::styled(format!(" Turns: {turn_count}"), style),
        ];

        let strategy_widget = Paragraph::new(Line::from(left_spans))
            .style(style)
            .alignment(Alignment::Left);
        frame.render_widget(strategy_widget, area);

        let notification = state.frontend.active_status_notification();
        let right_spans = if let Some(msg) = notification {
            vec![
                Span::styled(msg, Style::default().fg(state.frontend.theme.success)),
                Span::styled(format!("  {model}"), style),
            ]
        } else {
            vec![Span::styled(model, style)]
        };
        let right_line = Line::from(right_spans);
        let model_widget = Paragraph::new(right_line).alignment(Alignment::Right);
        frame.render_widget(model_widget, area);
    }
}
