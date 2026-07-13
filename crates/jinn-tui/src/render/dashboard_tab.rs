//! Dashboard tab rendering — full-width service list using ratatui `Table`.
//!
//! Four real columns via `Constraint` widths:
//!   Name | Description | State | Notes
//!
//! Selection + scroll are driven by [`TableState`] synced from
//! [`DashboardState`] (selected index + scroll offset).

use jinn_domain::RenderCtx;
use jinn_domain::feat::dashboard::ActorLifecycle;
use jinn_domain::feat::theme::Theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, HighlightSpacing, Paragraph, Row, Table, TableState};

/// Renders the full dashboard view into `area` (the content rect of the tab).
/// Caller is responsible for clamping `scroll_offset` via
/// [`DashboardState::clamp_scroll`] before rendering; this function only reads.
pub fn render_dashboard(frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
    let state = ctx.state;
    let dashboard = &state.frontend.dashboard;
    let theme = &state.frontend.theme;

    let actors = dashboard.actors();
    if actors.is_empty() {
        render_empty(frame, area, theme);
        return;
    }

    let rows = build_rows(&actors, theme);
    let widths = [
        Constraint::Length(22),
        Constraint::Min(10),
        Constraint::Length(10),
        Constraint::Min(10),
    ];

    let header = Row::new(vec![
        Cell::from("Name"),
        Cell::from("Description"),
        Cell::from("State"),
        Cell::from("Notes"),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD));

    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(2)
        .row_highlight_style(Style::default().fg(theme.focus_accent))
        .highlight_symbol("▸ ")
        .highlight_spacing(HighlightSpacing::Always);

    let mut table_state = TableState::default();
    table_state.select(Some(dashboard.selected_index()));
    *table_state.offset_mut() = usize::from(dashboard.scroll_offset());

    frame.render_stateful_widget(table, area, &mut table_state);
}

/// Renders the empty-state placeholder.
fn render_empty(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    let para = Paragraph::new(Line::from(Span::styled(
        " No services registered.",
        Style::default().fg(theme.muted_text),
    )));
    frame.render_widget(para, area);
}

/// Builds the table rows from dashboard entries, applying per-lifecycle colors.
fn build_rows<'a>(
    actors: &[&'a jinn_domain::feat::dashboard::DashboardEntry],
    theme: &Theme,
) -> Vec<Row<'a>> {
    actors
        .iter()
        .map(|entry| {
            let name_cell =
                Cell::from(entry.name.as_str()).style(Style::default().fg(theme.primary_text));

            let desc_cell = Cell::from(entry.description.as_deref().unwrap_or(""))
                .style(Style::default().fg(theme.muted_text));

            let (state_str, state_color) = lifecycle_display(entry.lifecycle, theme);
            let state_cell = Cell::from(state_str).style(Style::default().fg(state_color));

            let status_str = entry.status_message.as_deref().unwrap_or("");
            let status_cell = Cell::from(status_str).style(Style::default().fg(theme.muted_text));

            Row::new(vec![name_cell, desc_cell, state_cell, status_cell])
        })
        .collect()
}

/// Returns the display string and color for a lifecycle variant.
fn lifecycle_display(lifecycle: ActorLifecycle, theme: &Theme) -> (&'static str, Color) {
    match lifecycle {
        ActorLifecycle::Starting => ("Starting", theme.warning),
        ActorLifecycle::Running => ("Running", theme.success),
        ActorLifecycle::Dead => ("Dead", theme.error_text),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use jinn_domain::feat::dashboard::DashboardState;
    use jinn_testutil::setup_term;

    async fn build_app() -> crate::TuiApp {
        crate::TuiApp::test_builder().build().await
    }

    fn write_dashboard(app: &crate::TuiApp, f: impl FnOnce(&mut DashboardState)) {
        f(&mut app.core.state.write().frontend.dashboard);
    }

    /// Collects the entire terminal buffer into a single string for substring
    /// assertions.
    fn buffer_string(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    #[tokio::test]
    async fn renders_actor_name_and_lifecycle() {
        // Given a dashboard with one running actor.
        let mut app = build_app().await;
        app.core
            .state
            .write()
            .frontend
            .scope_stack
            .swap_base(jinn_domain::FocusScope::Dashboard);
        write_dashboard(&app, |d| {
            d.mark_running("discord", Some("Discord bot".to_owned()));
        });
        let (mut terminal, _area) = setup_term(80, 24);

        // When rendering.
        terminal.draw(|frame| app.render(frame)).unwrap();

        // Then the buffer contains "discord" and "Running".
        let buf_str = buffer_string(&terminal);
        assert!(buf_str.contains("discord"), "dashboard should show name");
        assert!(
            buf_str.contains("Running"),
            "dashboard should show lifecycle"
        );
    }

    #[tokio::test]
    async fn renders_status_message_for_discord() {
        // Given a dashboard with discord in a connected state.
        let mut app = build_app().await;
        app.core
            .state
            .write()
            .frontend
            .scope_stack
            .swap_base(jinn_domain::FocusScope::Dashboard);
        write_dashboard(&app, |d| {
            d.mark_running("discord", None);
            d.set_status_message("discord", Some("Connected".to_owned()));
        });
        let (mut terminal, _area) = setup_term(80, 24);

        // When rendering.
        terminal.draw(|frame| app.render(frame)).unwrap();

        // Then the buffer contains "Connected".
        let buf_str = buffer_string(&terminal);
        assert!(
            buf_str.contains("Connected"),
            "dashboard should show status message"
        );
    }

    #[tokio::test]
    async fn renders_empty_placeholder_when_no_actors() {
        // Given a dashboard with no actors.
        let mut app = build_app().await;
        app.core
            .state
            .write()
            .frontend
            .scope_stack
            .swap_base(jinn_domain::FocusScope::Dashboard);
        let (mut terminal, _area) = setup_term(80, 24);

        // When rendering.
        terminal.draw(|frame| app.render(frame)).unwrap();

        // Then the buffer contains the placeholder.
        let buf_str = buffer_string(&terminal);
        assert!(
            buf_str.contains("No services"),
            "empty dashboard should show placeholder"
        );
    }

    #[tokio::test]
    async fn shows_selection_marker_on_selected_entry() {
        // Given a dashboard with two actors, second selected.
        let mut app = build_app().await;
        app.core
            .state
            .write()
            .frontend
            .scope_stack
            .swap_base(jinn_domain::FocusScope::Dashboard);
        write_dashboard(&app, |d| {
            d.mark_running("alpha", None);
            d.mark_running("beta", None);
            d.select_next(); // select beta (index 1)
        });
        let (mut terminal, _area) = setup_term(80, 24);

        // When rendering.
        terminal.draw(|frame| app.render(frame)).unwrap();

        // Then the buffer contains the selection marker ▸.
        let buf_str = buffer_string(&terminal);
        assert!(buf_str.contains('▸'), "selected entry should have marker");
    }

    #[tokio::test]
    async fn renders_no_em_dash_separator() {
        // Given a dashboard with an actor that has a description.
        let mut app = build_app().await;
        app.core
            .state
            .write()
            .frontend
            .scope_stack
            .swap_base(jinn_domain::FocusScope::Dashboard);
        write_dashboard(&app, |d| {
            d.mark_running("discord", Some("Discord bot".to_owned()));
        });
        let (mut terminal, _area) = setup_term(80, 24);

        // When rendering.
        terminal.draw(|frame| app.render(frame)).unwrap();

        // Then the buffer contains no em-dash characters.
        let buf_str = buffer_string(&terminal);
        assert!(
            !buf_str.contains('\u{2014}'),
            "dashboard should not contain em-dashes"
        );
    }

    #[test]
    fn clamp_scroll_keeps_selected_visible() {
        // Given a dashboard with 5 actors, selection at index 4, viewport 3.
        let mut state = DashboardState::new();
        for name in ["a", "b", "c", "d", "e"] {
            state.mark_running(name, None);
        }
        state.select_last(); // index 4
        assert_eq!(state.selected_index(), 4);

        // When clamping with viewport 3.
        state.clamp_scroll(3);

        // Then scroll_offset puts index 4 within the visible window.
        let visible_start = state.scroll_offset() as usize;
        let visible_end = visible_start + 3;
        assert!(
            (visible_start..visible_end).contains(&4),
            "selected index should be within visible window {visible_start}..{visible_end}"
        );
    }
}
