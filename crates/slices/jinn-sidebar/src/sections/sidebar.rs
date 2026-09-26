//! [`Sidebar`] - the sidebar container that manages section registration,
//! focus delegation, section-crossing navigation, and rendering.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Block;

use super::layout;
use super::section_trait::{
    EnterFrom, SectionNavResult, SidebarIntent, SidebarSection, SidebarSectionId,
};
use super::{mcp_servers_section, persona_section, pins, sessions, task_list_section};
use jinn_domain::common::app_state::AppState;
use jinn_domain::common::render_ctx::RenderCtx;
use jinn_mcp_msg::config::McpServersConfig;
/// The sidebar container.
///
/// Holds registered sections in order, manages focus, and handles
/// section-crossing navigation for `j`/`k` intents.
#[derive(Debug)]
pub struct Sidebar {
    sections: Vec<Box<dyn SidebarSection>>,
}

impl Sidebar {
    /// Creates a new empty sidebar.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sections: Vec::new(),
        }
    }

    /// Registers a section with the sidebar.
    ///
    /// Sections are rendered and navigated in registration order.
    pub fn register(&mut self, section: Box<dyn SidebarSection>) {
        self.sections.push(section);
    }

    /// Returns the number of registered sections.
    #[must_use]
    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// Renders all sections within the given area.
    ///
    /// Sections form one document whose rows are windowed by a single offset
    /// that keeps the focused section's cursor visible, so a section taller
    /// than the column scrolls instead of being silently dropped. Each section
    /// receives the slice of the column its span occupies, plus the number of
    /// its own leading rows scrolled above the column.
    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
        // Clear sidebar area with dark gray background.
        let background =
            Block::default().style(Style::default().bg(ctx.state.frontend.theme.gutter_bg));
        frame.render_widget(background, area);

        let document = {
            let ids: Vec<_> = self.sections.iter().map(|section| section.id()).collect();
            layout::with_cursor(layout::document_for(ctx.state, ctx.config, &ids), ctx.state)
        };
        // The offset normally follows the focused section's cursor. While the
        // chat pane holds focus there is no sidebar section, so the document
        // has no cursor to centre on — fall back to the offset last derived
        // while the sidebar was focused, so the column does not jump when
        // focus comes and goes. The write-back happens in the pre-render
        // pass (`write_scroll_offset`), keeping `render` read-only.
        let offset = layout::scroll_offset(ctx.state, ctx.config, area.height);
        // When the document is shorter than the column, leave the unused rows
        // *between* the last two sections rather than pushing the whole
        // document down: the leading sections stay at the top of the column,
        // the trailing block sits at the bottom, and the gap separates them.
        // Once the document overflows, the slack is zero and the single scroll
        // offset takes over.
        let slack = document.bottom_slack(area.height);
        let last_index = document.spans.len().saturating_sub(1);

        for (index, (span, section)) in document
            .spans
            .iter()
            .zip(self.sections.iter_mut())
            .enumerate()
        {
            let push_down = if index == last_index { slack } else { 0 };
            let Some((section_area, skip_rows)) =
                layout::visible_rect(area, *span, offset, push_down)
            else {
                continue;
            };
            section.render(frame, section_area, skip_rows, ctx);
        }

        layout::render_scroll_indicators(
            frame,
            area,
            offset,
            document.total_rows,
            &ctx.state.frontend.theme,
        );
    }
}

impl Default for Sidebar {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Sidebar topology - section-crossing navigation
// ---------------------------------------------------------------------------

/// Navigate the sidebar, handling section-crossing when a section exhausts its entries.
///
/// This is the single navigation entry point called by the IntentHandler.
/// Sections report `Exhausted` when they run out of entries; this function
/// decides whether to switch to an adjacent section or keep the cursor where it is.
pub fn navigate_sidebar(
    direction: &SidebarIntent,
    state: &mut AppState,
    config: &jinn_slices::ConfigLayer,
) {
    let focused = state
        .frontend
        .sidebar_section()
        .unwrap_or(jinn_sidebar_msg::SidebarSectionId::Persona);
    let result = dispatch_navigate(focused, direction, state, config);

    if result == SectionNavResult::Exhausted {
        let neighbor = match direction {
            SidebarIntent::MoveDown => next_section(focused),
            SidebarIntent::MoveUp => prev_section(focused),
            SidebarIntent::Action(_) => return,
        };

        // Scan past consecutive empty sections.
        let mut candidate = neighbor;
        while let Some(target) = candidate {
            if section_has_content(target, state, config) {
                // Restore history position when leaving Pins.
                if focused == jinn_sidebar_msg::SidebarSectionId::Pins {
                    state.active_session_mut().restore_history_position();
                }
                clear_cursor(focused, state);
                state.frontend.scope_set_sidebar_section(target);
                let enter_from = match direction {
                    SidebarIntent::MoveDown => EnterFrom::Top,
                    SidebarIntent::MoveUp => EnterFrom::Bottom,
                    SidebarIntent::Action(_) => return,
                };
                receive_cursor(target, enter_from, state, config);
                return;
            }
            candidate = match direction {
                SidebarIntent::MoveDown => next_section(target),
                SidebarIntent::MoveUp => prev_section(target),
                SidebarIntent::Action(_) => return,
            };
        }
    }
}

fn dispatch_navigate(
    section: SidebarSectionId,
    intent: &SidebarIntent,
    state: &mut AppState,
    config: &jinn_slices::ConfigLayer,
) -> SectionNavResult {
    match section {
        jinn_sidebar_msg::SidebarSectionId::Persona => persona_section::navigate(intent, state),
        jinn_sidebar_msg::SidebarSectionId::Pins => pins::navigate(intent, state),
        jinn_sidebar_msg::SidebarSectionId::TaskList => task_list_section::navigate(intent, state),
        jinn_sidebar_msg::SidebarSectionId::McpServers => {
            mcp_servers_section::navigate(intent, state, config)
        }
        jinn_sidebar_msg::SidebarSectionId::Sessions => sessions::navigate(intent, state),
    }
}

fn next_section(id: SidebarSectionId) -> Option<SidebarSectionId> {
    match id {
        jinn_sidebar_msg::SidebarSectionId::Persona => {
            Some(jinn_sidebar_msg::SidebarSectionId::Pins)
        }
        jinn_sidebar_msg::SidebarSectionId::Pins => {
            Some(jinn_sidebar_msg::SidebarSectionId::TaskList)
        }
        jinn_sidebar_msg::SidebarSectionId::TaskList => {
            Some(jinn_sidebar_msg::SidebarSectionId::McpServers)
        }
        jinn_sidebar_msg::SidebarSectionId::McpServers => {
            Some(jinn_sidebar_msg::SidebarSectionId::Sessions)
        }
        jinn_sidebar_msg::SidebarSectionId::Sessions => None,
    }
}

fn prev_section(id: SidebarSectionId) -> Option<SidebarSectionId> {
    match id {
        jinn_sidebar_msg::SidebarSectionId::Persona => None,
        jinn_sidebar_msg::SidebarSectionId::Pins => {
            Some(jinn_sidebar_msg::SidebarSectionId::Persona)
        }
        jinn_sidebar_msg::SidebarSectionId::TaskList => {
            Some(jinn_sidebar_msg::SidebarSectionId::Pins)
        }
        jinn_sidebar_msg::SidebarSectionId::McpServers => {
            Some(jinn_sidebar_msg::SidebarSectionId::TaskList)
        }
        jinn_sidebar_msg::SidebarSectionId::Sessions => {
            Some(jinn_sidebar_msg::SidebarSectionId::McpServers)
        }
    }
}

fn section_has_content(
    id: SidebarSectionId,
    state: &AppState,
    config: &jinn_slices::ConfigLayer,
) -> bool {
    match id {
        jinn_sidebar_msg::SidebarSectionId::Persona => true,
        jinn_sidebar_msg::SidebarSectionId::Pins => !state.sorted_pinned_ids().is_empty(),
        jinn_sidebar_msg::SidebarSectionId::TaskList => {
            !state.active_session().task_list().is_empty()
        }
        jinn_sidebar_msg::SidebarSectionId::McpServers => {
            let enabled = state.active_session().enabled_mcp_servers();
            config
                .get::<McpServersConfig>()
                .unwrap_or_default()
                .iter()
                .any(|(name, _)| enabled.contains(name.as_str()))
        }
        jinn_sidebar_msg::SidebarSectionId::Sessions => !state.session.is_empty(),
    }
}

pub(crate) fn clear_cursor(id: SidebarSectionId, state: &mut AppState) {
    state.frontend.update_sections(|s| match id {
        jinn_sidebar_msg::SidebarSectionId::Persona => s.persona.cursor = None,
        jinn_sidebar_msg::SidebarSectionId::Pins => s.pins.clear_selection(),
        jinn_sidebar_msg::SidebarSectionId::TaskList => s.task_list.selected_phase_index = None,
        jinn_sidebar_msg::SidebarSectionId::McpServers => s.mcp_servers.selected_index = None,
        jinn_sidebar_msg::SidebarSectionId::Sessions => {
            s.sessions.selected_index = None;
        }
    });
}

fn receive_cursor(
    id: SidebarSectionId,
    enter_from: EnterFrom,
    state: &mut AppState,
    config: &jinn_slices::ConfigLayer,
) {
    match id {
        jinn_sidebar_msg::SidebarSectionId::Persona => {
            persona_section::receive_cursor(state, enter_from)
        }
        jinn_sidebar_msg::SidebarSectionId::Pins => pins::receive_cursor(state, enter_from),
        jinn_sidebar_msg::SidebarSectionId::TaskList => {
            task_list_section::receive_cursor(state, enter_from)
        }
        jinn_sidebar_msg::SidebarSectionId::McpServers => {
            mcp_servers_section::receive_cursor(state, enter_from, config)
        }
        jinn_sidebar_msg::SidebarSectionId::Sessions => sessions::receive_cursor(state, enter_from),
    }
}

/// Check if a section has a retained cursor.
fn section_has_cursor(id: SidebarSectionId, state: &AppState) -> bool {
    state.frontend.with_sections(
        |s| match id {
            jinn_sidebar_msg::SidebarSectionId::Persona => s.persona.cursor.is_some(),
            jinn_sidebar_msg::SidebarSectionId::Pins => s.pins.selected_id().is_some(),
            jinn_sidebar_msg::SidebarSectionId::TaskList => {
                s.task_list.selected_phase_index.is_some()
            }
            jinn_sidebar_msg::SidebarSectionId::McpServers => {
                s.mcp_servers.selected_index.is_some()
            }
            jinn_sidebar_msg::SidebarSectionId::Sessions => s.sessions.selected_index.is_some(),
        },
        || false,
    )
}

/// Jump directly to the next/previous sidebar section without clearing cursors.
///
/// Uses existing [`next_section`]/[`prev_section`] helpers, skipping empty sections.
/// Retains the leaving section's cursor position. If the target section has no
/// cursor (never visited), calls [`receive_cursor`] as fallback.
/// If the target has a retained cursor, ensures scroll offset is valid.
#[expect(
    clippy::else_if_without_else,
    reason = "no-op on fallthrough is intentional"
)]
pub fn jump_to_section(
    direction: &SidebarIntent,
    state: &mut AppState,
    config: &jinn_slices::ConfigLayer,
) {
    let focused = state
        .frontend
        .sidebar_section()
        .unwrap_or(jinn_sidebar_msg::SidebarSectionId::Persona);
    let neighbor_fn: fn(SidebarSectionId) -> Option<SidebarSectionId> = match direction {
        SidebarIntent::MoveDown => next_section,
        SidebarIntent::MoveUp => prev_section,
        SidebarIntent::Action(_) => return,
    };

    // Find the next non-empty section.
    let mut candidate = neighbor_fn(focused);
    while let Some(target) = candidate {
        if section_has_content(target, state, config) {
            // Restore history position when leaving Pins.
            if focused == jinn_sidebar_msg::SidebarSectionId::Pins
                && target != jinn_sidebar_msg::SidebarSectionId::Pins
            {
                state.active_session_mut().restore_history_position();
            }

            state.frontend.scope_set_sidebar_section(target);

            // Save history position when entering Pins without receive_cursor.
            if target == jinn_sidebar_msg::SidebarSectionId::Pins
                && !state.active_session().has_saved_history_position()
            {
                state.active_session_mut().save_history_position();
            }

            // If target has no cursor, call receive_cursor as fallback.
            if !section_has_cursor(target, state) {
                let enter_from = match direction {
                    SidebarIntent::MoveDown => EnterFrom::Top,
                    SidebarIntent::MoveUp => EnterFrom::Bottom,
                    SidebarIntent::Action(_) => return,
                };
                receive_cursor(target, enter_from, state, config);
            } else if target == jinn_sidebar_msg::SidebarSectionId::Pins {
                // Pins has a retained cursor - sync chat log to show it.
                pins::pins_section::sync_chat_log_cursor(state);
            }
            return;
        }
        candidate = neighbor_fn(target);
    }
}
