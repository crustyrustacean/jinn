//! Status bar - displays the session CWD, active prompt strategy, and current model.
//!
//! Shows the session's working directory on line 1, and status information
//! on line 2: strategy, pinned count, token stats, turn count, and model.
//! The model shows `({provider})/{model}` when set, or "no model selected" otherwise.

use crate::common::app_state::AppState;
use crate::common::ui_element::UiElement;
use crate::feat::provider_infra::NO_PROVIDER_ID;
use crate::feat::session::{TokenStats, aggregate_tree_stats};
use crate::feat::ui::status_bar::turn_counter;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// A display element that shows the active strategy and provider/model in the status bar.
#[derive(Debug)]
pub struct StatusBarElement;

/// Shorten a path for display: replace home directory prefix with `~`.
fn shorten_path(path: &std::path::Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(relative) = path.strip_prefix(&home)
    {
        let display = relative.display().to_string();
        if display.is_empty() {
            return "~".to_owned();
        }
        return format!("~/{display}");
    }
    path.display().to_string()
}

/// Format a token count in human-readable form with one decimal place.
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

/// Format a token budget as whole numbers only (e.g. `150k`, `1M`, `999`).
fn format_budget(count: usize) -> String {
    if count >= 1_000_000 {
        format!("{}M", count / 1_000_000)
    } else if count >= 1_000 {
        format!("{}k", count / 1_000)
    } else {
        count.to_string()
    }
}

/// Look up the context_length for the active model from the model cache.
///
/// Returns `Some(context_length)` if the cache is populated and the active model
/// is found with a non-None context_length. Returns `None` otherwise.
fn resolve_context_limit(
    model_cache: Option<&crate::feat::provider_infra::ModelCache>,
    active_model: &str,
) -> Option<u32> {
    model_cache.and_then(|cache| {
        let provider_name = active_model.split('/').next()?;
        let models = cache.entries.get(provider_name)?;
        let model_suffix = &active_model[(provider_name.len() + 1)..];
        models
            .iter()
            .find(|m| m.id == model_suffix)
            .and_then(|m| m.context_length)
    })
}

impl UiElement<AppState> for StatusBarElement {
    fn name(&self) -> String {
        "status-bar".to_owned()
    }

    #[allow(clippy::cast_precision_loss, clippy::too_many_lines)]
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        // Split area into cwd line + info line.
        let [cwd_area, info_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);

        // --- Line 1: CWD ---
        let cwd = state.active_session().cwd();
        let cwd_display = shorten_path(cwd);
        let style = Style::default().fg(state.frontend.theme.muted_text);
        let cwd_widget = Paragraph::new(Line::from(Span::styled(cwd_display, style)))
            .style(style)
            .alignment(Alignment::Left);
        frame.render_widget(cwd_widget, cwd_area);

        // --- Line 1 right: Tree aggregate (only when tree has >1 session) ---
        let tree = aggregate_tree_stats(
            state.session.sessions(),
            state.session.frozen_nodes(),
            state.session.active_session_id(),
        );
        if tree.session_count > 1 {
            let up_arrow = '\u{2191}';
            let down_arrow = '\u{2193}';
            let turn_symbol = '\u{21BB}';
            let session_symbol = '\u{29C9}';
            let tree_prefix = '\u{1F333}';
            let tree_display = format!(
                "{tree_prefix} {up_arrow}{} {down_arrow}{} ${:.5} {turn_symbol}{turns} {session_symbol}{count}",
                format_tokens(tree.total_sent),
                format_tokens(tree.total_received),
                tree.total_cost,
                turns = tree.total_turns,
                count = tree.session_count,
            );
            let tree_widget = Paragraph::new(Line::from(Span::styled(tree_display, style)))
                .alignment(Alignment::Right);
            frame.render_widget(tree_widget, cwd_area);
        }

        // --- Line 2: Existing info ---
        let active_model = state.active_session().profile().model.clone();

        // Compute token stats for the active session only (no descendants).
        let active_session = state.active_session();
        let token_stats = TokenStats::from_ledger(active_session.token_ledger());
        let total_cost = TokenStats::total_cost(active_session.token_ledger());
        let up_arrow = '\u{2191}';
        let down_arrow = '\u{2193}';
        let mut token_info = format!(
            "{up_arrow}{} {down_arrow}{}",
            format_tokens(token_stats.total_sent),
            format_tokens(token_stats.total_received),
        );

        let ctx_size = state.active_session().context_size();
        let ctx_limit = resolve_context_limit(state.provider.model_cache.as_ref(), &active_model);

        let context_display = match (ctx_size, ctx_limit) {
            (Some(used), Some(max)) => {
                let ctx_used = u64::from(used);
                let max_u64 = u64::from(max);
                let pct = if max_u64 > 0 {
                    format!("{:.1}%", (ctx_used as f64 / max_u64 as f64) * 100.0)
                } else {
                    "0.0%".to_owned()
                };
                format!("{}/{}", pct, format_budget(max as usize))
            }
            (None, Some(max)) => {
                format!("0.0%/{}", format_budget(max as usize))
            }
            (Some(used), None) => {
                format!("{}/???", format_tokens(u64::from(used)))
            }
            (None, None) => "0/???".to_owned(),
        };
        token_info = format!("{token_info} {context_display}");

        let model = if active_model == NO_PROVIDER_ID {
            "no model selected".to_owned()
        } else if let Some((provider, model)) = active_model.split_once('/') {
            format!("({provider})/{model}")
        } else {
            active_model.clone()
        };

        let left_side = {
            // Build left side: cost + turn count.
            let turn_count = turn_counter::compute_turn_count(state.active_session().history());
            let turn_symbol = '\u{21BB}';
            let left_spans: Vec<Span> = vec![
                Span::styled(token_info, style),
                Span::styled(format!(" ${:.5}", total_cost.abs()), style),
                Span::styled(format!(" {turn_symbol}{turn_count}"), style),
            ];
            Paragraph::new(Line::from(left_spans))
                .style(style)
                .alignment(Alignment::Left)
        };
        frame.render_widget(left_side, info_area);

        let right_spans = vec![Span::styled(model, style)];
        let right_line = Line::from(right_spans);
        let model_widget = Paragraph::new(right_line).alignment(Alignment::Right);
        frame.render_widget(model_widget, info_area);
    }
}
