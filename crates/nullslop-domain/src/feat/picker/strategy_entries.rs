//! Strategy entries — loading, sorting, and formatting.
//!
//! Contains loader functions, sorting, and formatting utilities for the
//! strategy picker overlay. The [`StrategyEntry`] struct and [`PickerItem`]
//! implementation live in `nullslop-protocol`.

use crate::common::app_state::AppState;
use crate::common::services::Services;
use crate::feat::picker::style::promote_active_to_top;
use crate::feat::theme::Theme;
use crate::protocol::{PromptStrategyId, StrategyEntry};

/// Loads strategy entries from the strategy registry, marking the active one.
pub fn load_strategy_entries(
    services: &Services,
    active_strategy: &PromptStrategyId,
    theme: &Theme,
) -> Vec<StrategyEntry> {
    let strategies = services.strategy_registry.list();
    strategies
        .into_iter()
        .map(|info| {
            let is_active = &info.id == active_strategy;
            StrategyEntry {
                strategy_id: info.id,
                is_active,
                name: info.name,
                description: info.description,
                theme: theme.clone(),
            }
        })
        .collect()
}

/// Reorders strategy entries so the active strategy is promoted to the top
/// when the filter is empty. When the filter is non-empty, preserves fuzzy match order.
pub fn sorted_strategy_entries(entries: &[StrategyEntry], filter: &str) -> Vec<StrategyEntry> {
    let mut result = entries.to_vec();

    promote_active_to_top(&mut result, |e| e.is_active, filter);

    result
}

/// Loads strategy entries into the picker state, ready for display.
pub fn load_strategy_picker_items(services: &Services, state: &mut AppState) {
    let active_strategy = state.active_session().active_strategy().clone();
    let all = load_strategy_entries(services, &active_strategy, &state.frontend.theme);
    let entries = sorted_strategy_entries(&all, "");
    state.frontend.context_strategy_picker.set_items(entries);
}

/// Formats the footer line showing the current strategy.
pub fn format_strategy_footer(strategy_name: &str, theme: &Theme) -> ratatui::text::Line<'static> {
    use ratatui::style::Style;
    use ratatui::text::{Line, Span};

    let gray = Style::default().fg(theme.muted_text);
    Line::from(vec![
        Span::styled("Current: ".to_owned(), gray),
        Span::styled(
            strategy_name.to_owned(),
            Style::default().fg(theme.primary_text),
        ),
    ])
}
