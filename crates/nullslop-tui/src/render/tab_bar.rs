//! Tab bar rendering and initialization.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui_tabs::{TabManager, TabsBar, TabsStyle};

/// Build the default tab manager with Chat and Dashboard tabs.
pub fn init_tab_manager() -> TabManager {
    let mut mgr = TabManager::new();
    mgr.add_tab("Chat");
    mgr.add_tab("Dashboard");
    mgr
}

/// Renders the tab bar.
pub(super) fn render_tab_bar(
    frame: &mut Frame<'_>,
    area: Rect,
    manager: &TabManager,
    tab_active_fg: Color,
    tab_active_bg: Color,
    tab_inactive_fg: Color,
) {
    let tabs = manager.tabs();
    let active_id = manager.active_id();
    let bar = TabsBar::new(tabs, active_id).tabs_style(TabsStyle {
        active: Style::default().fg(tab_active_fg).bg(tab_active_bg),
        inactive: Style::default().fg(tab_inactive_fg),
        ..TabsStyle::default()
    });
    frame.render_widget(bar, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    fn init_tab_manager_has_two_tabs() {
        // Given a default tab manager.
        let mgr = init_tab_manager();

        // When checking tab count.
        // Then there are 2 tabs and the first is active.
        assert_eq!(mgr.tab_count(), 2);
        assert!(mgr.active_tab().is_some());
        assert_eq!(mgr.active_tab().unwrap().name, "Chat");
    }
}
