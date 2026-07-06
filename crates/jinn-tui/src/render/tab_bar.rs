//! Tab bar — top-level strip showing `[ Chat ] [ Dashboard ]`.

use jinn_domain::RenderCtx;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// The tab labels shown left-to-right.
///
/// Order must stay in sync with how `is_dashboard_active` is derived from
/// the current `FocusScope`.
const TAB_LABELS: [&str; 2] = ["Chat", "Dashboard"];

/// Returns `true` when the dashboard tab is the active one.
fn is_dashboard_active(ctx: &RenderCtx) -> bool {
    matches!(
        ctx.state.frontend.scope_stack.base(),
        jinn_domain::FocusScope::Dashboard
    )
}

/// Renders the tab bar into `area`.
pub fn render_tab_bar(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    let theme = &ctx.state.frontend.theme;
    let dashboard_active = is_dashboard_active(ctx);

    let mut spans = Vec::new();
    for (idx, &label) in TAB_LABELS.iter().enumerate() {
        let is_active = if dashboard_active { idx == 1 } else { idx == 0 };

        let style = if is_active {
            Style::default()
                .fg(theme.tab_active_fg)
                .bg(theme.tab_active_bg)
        } else {
            Style::default().fg(theme.tab_inactive_fg)
        };

        spans.push(Span::styled(format!(" {label} "), style));
        if idx + 1 < TAB_LABELS.len() {
            spans.push(Span::raw(" "));
        }
    }

    let line = Line::from(spans);
    let para = Paragraph::new(line);
    frame.render_widget(para, area);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use jinn_domain::FocusScope;
    use jinn_testutil::setup_term;
    use ratatui::style::Color;

    async fn build_app_with_scope(scope: FocusScope) -> crate::TuiApp {
        let app = crate::TuiApp::test_builder().build().await;
        app.core.state.write().frontend.scope_stack.swap_base(scope);
        app
    }

    #[tokio::test]
    async fn chat_tab_is_highlighted_in_normal_scope() {
        // Given an app in Normal (chat) scope.
        let mut app = build_app_with_scope(FocusScope::Normal).await;
        let (mut terminal, _area) = setup_term(80, 24);

        // When rendering.
        terminal.draw(|frame| app.render(frame)).unwrap();

        // Then the chat tab cell has an active background (non-Reset).
        let layout = crate::render::app_layout::AppLayout::new(
            ratatui::layout::Rect::new(0, 0, 80, 24),
            1,
            12,
            30,
        );
        let buffer = terminal.backend().buffer();
        let cell = buffer
            .cell((layout.tab_bar.x + 1, layout.tab_bar.y))
            .expect("chat tab cell");
        assert_ne!(
            cell.bg,
            Color::Reset,
            "chat tab should be highlighted in Normal scope"
        );
    }

    #[tokio::test]
    async fn dashboard_tab_is_highlighted_in_dashboard_scope() {
        // Given an app in Dashboard scope.
        let mut app = build_app_with_scope(FocusScope::Dashboard).await;
        let (mut terminal, _area) = setup_term(80, 24);

        // When rendering.
        terminal.draw(|frame| app.render(frame)).unwrap();

        // Then the dashboard tab cell has an active background (non-Reset).
        let layout = crate::render::app_layout::AppLayout::new(
            ratatui::layout::Rect::new(0, 0, 80, 24),
            1,
            12,
            30,
        );
        let buffer = terminal.backend().buffer();
        // " Chat " (6 cols) + separator space (1) = 7 cols offset.
        let dash_x = layout.tab_bar.x + 1 + " Chat ".len() as u16 + 1;
        let cell = buffer
            .cell((dash_x, layout.tab_bar.y))
            .expect("dashboard tab cell");
        assert_ne!(
            cell.bg,
            Color::Reset,
            "dashboard tab should be highlighted in Dashboard scope"
        );
    }

    #[tokio::test]
    async fn dashboard_tab_stays_highlighted_when_quake_bar_open() {
        let mut app = build_app_with_scope(FocusScope::Dashboard).await;
        app.core
            .state
            .write()
            .frontend
            .scope_stack
            .push(FocusScope::QuakeBar);
        let (mut terminal, _area) = setup_term(80, 24);

        // When rendering.
        terminal.draw(|frame| app.render(frame)).unwrap();

        // Then the dashboard tab is still highlighted (uses base scope, not top).
        let layout = crate::render::app_layout::AppLayout::new(
            ratatui::layout::Rect::new(0, 0, 80, 24),
            1,
            12,
            30,
        );
        let buffer = terminal.backend().buffer();
        let dash_x = layout.tab_bar.x + 1 + " Chat ".len() as u16 + 1;
        let chat_cell = buffer
            .cell((layout.tab_bar.x + 1, layout.tab_bar.y))
            .expect("chat tab cell");
        let dash_cell = buffer
            .cell((dash_x, layout.tab_bar.y))
            .expect("dashboard tab cell");
        assert_eq!(
            chat_cell.bg,
            Color::Reset,
            "chat tab should NOT be highlighted when base is Dashboard"
        );
        assert_ne!(
            dash_cell.bg,
            Color::Reset,
            "dashboard tab should stay highlighted when quake bar is open"
        );
    }
}
