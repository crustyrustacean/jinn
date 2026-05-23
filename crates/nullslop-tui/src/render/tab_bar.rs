//! Tab bar rendering — displays tab labels at the top of the main area.

use nullslop_domain::ActiveTab;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Block,
};

/// Renders the tab bar in the given area.
pub fn render_tab_bar(frame: &mut Frame<'_>, area: Rect, active_tab: ActiveTab) {
    let tabs = ActiveTab::all();

    let spans: Vec<Span> = tabs
        .iter()
        .flat_map(|tab| {
            let is_active = *tab == active_tab;
            let style = if is_active {
                Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default()
            };
            let label = if is_active {
                format!(" {} ", tab.label())
            } else {
                format!(" {} ", tab.label())
            };
            vec![Span::styled(label, style), Span::raw("│")]
        })
        .collect();

    let line = Line::from(spans);
    frame.render_widget(
        ratatui::widgets::Paragraph::new(line).block(Block::default()),
        area,
    );
}
