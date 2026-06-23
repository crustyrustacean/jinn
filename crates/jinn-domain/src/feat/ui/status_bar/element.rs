//! Status bar - displays the session CWD, active prompt strategy, and current model.
//!
//! Shows the session's working directory on line 1, and status information
//! on line 2: strategy, pinned count, token stats, turn count, and model.
//! The model shows `({provider})/{model}` when set, or "no model selected" otherwise.

use crate::common::path_display::shorten_path;
use crate::common::render_ctx::RenderCtx;
use crate::common::ui_element::UiElement;
use crate::feat::session::{TokenStats, aggregate_tree_stats};
use crate::feat::ui::status_bar::turn_counter;
use crate::resolve_effort;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// A display element that shows the active strategy and provider/model in the status bar.
#[derive(Debug)]
pub struct StatusBarElement;

/// Format a token count in human-readable form with one decimal place.
#[expect(
    clippy::cast_precision_loss,
    reason = "loss is negligible for UI calculations"
)]
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
    let cache = model_cache?;
    let provider_name = active_model.split('/').next()?;
    let models = cache.entries.get(provider_name)?;
    let model_suffix = active_model.get((provider_name.len() + 1)..)?;
    models
        .iter()
        .find(|m| m.id == model_suffix)
        .and_then(|m| m.context_length)
}

impl UiElement for StatusBarElement {
    fn name(&self) -> String {
        "status-bar".to_owned()
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
        let state = ctx.state;
        // Split area into cwd line + info line.
        let [cwd_area, info_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);

        let style = Style::default().fg(state.frontend.theme.muted_text);

        render_cwd_line(frame, cwd_area, state, style);
        render_tree_aggregate(frame, cwd_area, state, style);
        render_token_info_line(frame, info_area, state, style);
    }
}

/// Renders the CWD line (left-aligned) for the active session.
fn render_cwd_line(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &crate::common::app_state::AppState,
    style: Style,
) {
    let cwd = state.active_session().cwd();
    let cwd_display = shorten_path(cwd);
    let cwd_widget = Paragraph::new(Line::from(Span::styled(cwd_display, style)))
        .style(style)
        .alignment(Alignment::Left);
    frame.render_widget(cwd_widget, area);
}

/// Renders the tree aggregate (right-aligned on the CWD line) when the tree has >1 session.
fn render_tree_aggregate(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &crate::common::app_state::AppState,
    style: Style,
) {
    let tree = aggregate_tree_stats(
        state.session.sessions(),
        state.session.frozen_nodes(),
        state.session.active_session_id(),
    );
    if tree.session_count <= 1 {
        return;
    }
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
    let tree_widget =
        Paragraph::new(Line::from(Span::styled(tree_display, style))).alignment(Alignment::Right);
    frame.render_widget(tree_widget, area);
}

/// Renders the token/model info line (left: token stats + cost + turns; right: model).
fn render_token_info_line(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &crate::common::app_state::AppState,
    style: Style,
) {
    let active_model = state.active_session().profile().model.clone();
    let token_info = build_token_info_string(state, &active_model);

    let model = build_model_string(state, &active_model);
    let total_cost = TokenStats::total_cost(state.active_session().token_ledger());

    let left_side = {
        let turn_count = turn_counter::compute_turn_count(
            state.active_session().history(),
            state.active_session().fork_ordinal(),
        );
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
    frame.render_widget(left_side, area);

    let right_spans = vec![Span::styled(model, style)];
    let model_widget = Paragraph::new(Line::from(right_spans)).alignment(Alignment::Right);
    frame.render_widget(model_widget, area);
}

/// Builds the left-side token info string: sent/received counts + context budget.
fn build_token_info_string(
    state: &crate::common::app_state::AppState,
    active_model: &crate::feat::session::model_selection::ModelSelection,
) -> String {
    let active_session = state.active_session();
    let token_stats = TokenStats::from_ledger(active_session.token_ledger());
    let up_arrow = '\u{2191}';
    let down_arrow = '\u{2193}';
    let token_info = format!(
        "{up_arrow}{} {down_arrow}{}",
        format_tokens(token_stats.total_sent),
        format_tokens(token_stats.total_received),
    );

    let ctx_size = state.active_session().context_size();
    let ctx_limit = resolve_context_limit(
        state.provider.model_cache.as_ref(),
        active_model.display_str(),
    );
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
        (None, Some(max)) => format!("0.0%/{}", format_budget(max as usize)),
        (Some(used), None) => format!("{}/???", format_tokens(u64::from(used))),
        (None, None) => "0/???".to_owned(),
    };
    format!("{token_info} {context_display}")
}

/// Builds the right-side model display string: resolved name + reasoning effort.
fn build_model_string(
    state: &crate::common::app_state::AppState,
    active_model: &crate::feat::session::model_selection::ModelSelection,
) -> String {
    let model = {
        // For an Alloy, the bar surfaces the last-rotated member (which one
        // actually answered) from the token ledger, falling back to the first
        // member before any dispatch. For a Single selection the model _is_ the
        // source of truth — never read the historical ledger, which would show
        // a stale model after switching providers via the picker.
        //
        // `model_used` is cloned to an owned `Option<String>` so the resolved
        // `&str` borrows only `active_model` and a local, avoiding a lifetime
        // clash between the ledger borrow and the `active_model` borrow.
        let last_dispatched = active_model.as_alloy().and_then(|_| {
            state
                .active_session()
                .token_ledger()
                .last()
                .and_then(|r| r.model_used.clone())
        });
        let resolved = last_dispatched
            .as_deref()
            .unwrap_or_else(|| active_model.display_str());

        if active_model.is_no_provider() {
            "no model selected".to_owned()
        } else if let Some((provider, m)) = resolved.split_once('/') {
            format!("({provider})/{m}")
        } else {
            resolved.to_owned()
        }
    };

    let model = match active_model.as_alloy() {
        Some(alloy) => format!("[alloy {}] {model}", alloy.models.len()),
        None => model,
    };

    // Append the resolved reasoning effort as `[<mode>]`.
    // Omitted entirely when no effort is resolved (no `[none]` noise).
    let resolved_effort = resolve_effort(state.active_session().profile().reasoning_effort);
    match resolved_effort {
        Some(effort) => format!("{model} [{}]", effort.as_str()),
        None => model,
    }
}
