//! [`McpServersSection`] — the MCP servers sidebar section.
//!
//! Implements [`SidebarSection`] for displaying the active session's MCP
//! servers. Reads the global catalog from `frontend.preferences.mcp_servers`
//! and overlays the per-session enablement set + live connection status from
//! the active session. Each configured server renders one row with a visual
//! treatment matching its effective state:
//!
//! - **disabled** (not enabled for this session) — muted
//! - **starting** (enabled but no status yet, or status `Starting`) — yellow
//! - **running** (status `Running`) — green
//! - **dead** (status `Dead`) — red
//!
//! The section is read-only in Part 1: navigation works (j/k), but there are
//! no section-specific actions (enable/disable is done via the picker).

use crate::common::app_state::AppState;
use crate::common::render_ctx::RenderCtx;
use crate::feat::mcp_actor::protocol::McpConnectionStatus;
use crate::feat::ui::sidebar::section_trait::{
    EnterFrom, SectionNavResult, SidebarIntent, SidebarSection, SidebarSectionId,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// Solid full block used as the selection indicator (same as other sections).
const SELECTED_INDICATOR: &str = "\u{2588}";
/// One space used as the unselected border (same as other sections).
const UNSELECTED_BORDER: &str = " ";

/// MCP servers section cursor state — stored on `FrontendState`.
///
/// Tracks the selected index into the configured-servers list.
/// `None` means no cursor (section not focused).
#[derive(Debug, Clone, Default)]
pub struct McpServersSectionState {
    /// Index into the configured MCP servers list.
    pub selected_index: Option<usize>,
}

/// The effective visual state of a single server row.
///
/// Derived from enablement + live status. A server that is enabled but has
/// not yet reported a status is treated as [`Self::Starting`] — it occupies
/// the gap between "user toggled on" and "first `McpServerStatus` arrived".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerRowState {
    /// Not enabled for this session — muted rendering.
    Disabled,
    /// Enabled and coming up (or explicitly reporting `Starting`).
    Starting,
    /// Live connection established and tools registered.
    Running,
    /// Connection failed or was torn down.
    Dead,
}

impl ServerRowState {
    /// Combines enablement + the optional live status into a row state.
    fn derive(enabled: bool, status: Option<McpConnectionStatus>) -> Self {
        match (enabled, status) {
            (false, _) => Self::Disabled,
            (true, None | Some(McpConnectionStatus::Starting)) => Self::Starting,
            (true, Some(McpConnectionStatus::Running)) => Self::Running,
            (true, Some(McpConnectionStatus::Dead)) => Self::Dead,
        }
    }

    /// Returns the textual label shown after the server name.
    fn label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Dead => "dead",
        }
    }

    /// Returns the color used for the status label and indicator.
    fn color(self) -> Color {
        match self {
            Self::Disabled => Color::DarkGray,
            Self::Starting => Color::Yellow,
            Self::Running => Color::Green,
            Self::Dead => Color::Red,
        }
    }
}

/// Collects the configured server names in catalog order.
fn configured_server_names(state: &AppState) -> Vec<String> {
    state
        .frontend
        .preferences
        .mcp_servers
        .iter()
        .map(|s| s.name.clone())
        .collect()
}

/// Navigate within the MCP servers section.
///
/// Moves the cursor up/down through the configured-servers list. Exhausts at
/// the list boundaries so the sidebar can move focus to the neighbor section.
/// The section does NOT modify its cursor on exhaustion.
pub fn navigate(intent: &SidebarIntent, state: &mut AppState) -> SectionNavResult {
    let count = configured_server_names(state).len();
    if count == 0 {
        return SectionNavResult::Exhausted;
    }
    let max_index = count - 1;
    let current = state
        .frontend
        .mcp_servers_section
        .selected_index
        .unwrap_or(0);
    match intent {
        SidebarIntent::MoveDown => {
            if current >= max_index {
                SectionNavResult::Exhausted
            } else {
                state.frontend.mcp_servers_section.selected_index = Some(current + 1);
                SectionNavResult::Moved
            }
        }
        SidebarIntent::MoveUp => {
            if current == 0 {
                SectionNavResult::Exhausted
            } else {
                state.frontend.mcp_servers_section.selected_index = Some(current - 1);
                SectionNavResult::Moved
            }
        }
        SidebarIntent::Action(_) => SectionNavResult::Moved,
    }
}

/// Place the cursor on this section from a given direction.
///
/// Positions at the edge of the list: index 0 from top, last index from bottom.
pub fn receive_cursor(state: &mut AppState, enter_from: EnterFrom) {
    let count = configured_server_names(state).len();
    if count == 0 {
        return;
    }
    let index = match enter_from {
        EnterFrom::Top => 0,
        EnterFrom::Bottom => count - 1,
    };
    state.frontend.mcp_servers_section.selected_index = Some(index);
}

/// The MCP servers sidebar section.
///
/// Renders a header followed by one row per configured server, scoped to the
/// active session's enablement + status.
#[derive(Debug)]
pub struct McpServersSection;

impl SidebarSection for McpServersSection {
    fn id(&self) -> SidebarSectionId {
        SidebarSectionId::McpServers
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
        let state = ctx.state;
        let sidebar_focused = state.frontend.scope_stack.is_sidebar();
        let section_focused = sidebar_focused
            && matches!(
                state.frontend.scope_stack.sidebar_section(),
                Some(SidebarSectionId::McpServers)
            );

        let servers = &state.frontend.preferences.mcp_servers;
        let enabled = state.active_session().enabled_mcp_servers();
        let statuses = state.active_session().mcp_server_status();
        let cursor = state.frontend.mcp_servers_section.selected_index;
        let theme = &state.frontend.theme;

        let indicator_color = if sidebar_focused {
            theme.focus_accent
        } else {
            theme.border_unfocused
        };

        let lines = {
            let mut lines = Vec::new();
            // Header.
            lines.push(Line::from(vec![Span::styled(
                " MCP servers",
                Style::default()
                    .fg(theme.primary_text)
                    .add_modifier(Modifier::BOLD),
            )]));
            // Blank separator.
            lines.push(Line::from(""));

            if servers.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  none configured",
                    Style::default().fg(Color::DarkGray),
                )));
            } else {
                for (index, server) in servers.iter().enumerate() {
                    let is_selected = section_focused && cursor == Some(index);
                    let row_state = ServerRowState::derive(
                        enabled.contains(&server.name),
                        statuses.get(&server.name).copied(),
                    );

                    let indicator = if is_selected {
                        Span::styled(SELECTED_INDICATOR, Style::default().fg(indicator_color))
                    } else {
                        Span::raw(UNSELECTED_BORDER)
                    };

                    let name_style = if is_selected {
                        Style::default().add_modifier(Modifier::REVERSED)
                    } else {
                        Style::default()
                    };

                    lines.push(Line::from(vec![
                        indicator,
                        Span::styled(format!(" {}", server.name), name_style),
                        Span::raw(" "),
                        Span::styled(row_state.label(), Style::default().fg(row_state.color())),
                    ]));
                }
            }
            lines
        };

        let widget = Paragraph::new(lines).block(Block::default().borders(Borders::NONE));
        frame.render_widget(widget, area);
    }

    fn content_height(&self, ctx: &RenderCtx) -> u16 {
        // Collapsed to 0 when no servers are configured, matching the Pins/TaskList
        // pattern so an empty catalog wastes no sidebar space.
        let servers = &ctx.state.frontend.preferences.mcp_servers;
        if servers.is_empty() {
            return 0;
        }
        // header(1) + blank(1) + one row per server + trailing gap(1).
        let rows = u16::try_from(servers.len()).unwrap_or(u16::MAX);
        rows.saturating_add(3)
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]
    use super::McpServersSection;
    use crate::common::app_state::AppState;
    use crate::common::render_ctx::RenderCtx;
    use crate::feat::mcp::McpServerConfig;
    use crate::feat::mcp_actor::protocol::McpConnectionStatus;
    use crate::feat::ui::sidebar::section_trait::{SidebarSection, SidebarSectionId};
    use jinn_testutil::setup_term;

    fn server(name: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.to_owned(),
            command: "echo".to_owned(),
            args: vec![],
            ..Default::default()
        }
    }

    fn render_rows(state: &AppState, width: u16, height: u16) -> Vec<String> {
        let mut section = McpServersSection;
        let (mut terminal, area) = setup_term(width, height);
        terminal
            .draw(|frame| {
                let ctx = RenderCtx::new(state);
                section.render(frame, area, &ctx);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| {
                        buffer
                            .cell((x, y))
                            .map_or(" ", ratatui::buffer::Cell::symbol)
                    })
                    .collect()
            })
            .collect()
    }

    fn state_with_servers(servers: &[McpServerConfig]) -> AppState {
        let mut state = AppState::default();
        state.frontend.preferences.mcp_servers = servers.to_vec();
        state
    }

    #[rstest::rstest]
    fn section_id_is_mcp_servers() {
        // Given an McpServersSection.
        let section = McpServersSection;

        // When asking for its ID.
        // Then it returns McpServers.
        assert_eq!(section.id(), SidebarSectionId::McpServers);
    }

    #[rstest::rstest]
    fn render_shows_header() {
        // Given state with no configured servers.
        let state = state_with_servers(&[]);

        // When rendering.
        let rows = render_rows(&state, 30, 5);

        // Then the first row contains the MCP servers header.
        assert!(rows[0].contains("MCP servers"));
    }

    #[rstest::rstest]
    fn render_disabled_server_is_muted() {
        // Given a configured server not enabled for the session.
        let state = state_with_servers(&[server("excalimate")]);

        // When rendering.
        let rows = render_rows(&state, 40, 5);

        // Then the row shows the server name and the disabled label.
        let combined = rows.join("\n");
        assert!(combined.contains("excalimate"));
        assert!(combined.contains("disabled"));
    }

    #[rstest::rstest]
    fn render_enabled_no_status_shows_starting() {
        // Given an enabled server with no status event yet.
        let mut state = state_with_servers(&[server("excalimate")]);
        state.active_session_mut().enable_mcp_server("excalimate");

        // When rendering.
        let rows = render_rows(&state, 40, 5);

        // Then the row shows the starting label.
        let combined = rows.join("\n");
        assert!(
            combined.contains("starting"),
            "enabled-but-no-status should render as starting; got: {combined}"
        );
    }

    #[rstest::rstest]
    fn render_running_status_shows_running() {
        // Given an enabled server reporting Running.
        let mut state = state_with_servers(&[server("excalimate")]);
        state.active_session_mut().enable_mcp_server("excalimate");
        state
            .active_session_mut()
            .set_mcp_server_status("excalimate", McpConnectionStatus::Running);

        // When rendering.
        let rows = render_rows(&state, 40, 5);

        // Then the row shows the running label.
        let combined = rows.join("\n");
        assert!(combined.contains("running"));
    }

    #[rstest::rstest]
    fn render_dead_status_shows_dead() {
        // Given an enabled server reporting Dead.
        let mut state = state_with_servers(&[server("excalimate")]);
        state.active_session_mut().enable_mcp_server("excalimate");
        state
            .active_session_mut()
            .set_mcp_server_status("excalimate", McpConnectionStatus::Dead);

        // When rendering.
        let rows = render_rows(&state, 40, 5);

        // Then the row shows the dead label.
        let combined = rows.join("\n");
        assert!(combined.contains("dead"));
    }

    #[rstest::rstest]
    fn render_only_active_session_servers() {
        // Given two configured servers, with only alpha enabled for the active session.
        // Session B (not active) has beta enabled, but it must not leak into the render.
        use crate::protocol::SessionId;
        let mut state = state_with_servers(&[server("alpha"), server("beta")]);
        let session_b = SessionId::new();
        state
            .session
            .get_or_create(&session_b)
            .enable_mcp_server("beta");
        state.active_session_mut().enable_mcp_server("alpha");

        // When rendering the active session (A).
        let rows = render_rows(&state, 40, 6);

        // Then alpha shows enabled (starting) and beta shows disabled, because
        // beta's enablement lives on session B, not the active session A.
        let combined = rows.join("\n");
        assert!(combined.contains("alpha"));
        assert!(combined.contains("beta"));
        assert!(combined.contains("starting"));
        assert!(combined.contains("disabled"));
    }
}
