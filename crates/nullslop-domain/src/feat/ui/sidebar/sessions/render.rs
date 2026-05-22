//! [`SessionsSection`] — the open sessions sidebar section.
//!
//! Implements [`SidebarSection`] for listing all sessions currently loaded
//! into memory. The active session (currently displayed) is highlighted with
//! a `▸` prefix. Navigating with j/k immediately switches the active session.

pub mod entry_line;
pub mod scroll_tag;
pub mod truncate;

#[cfg(test)]
mod entry_line_tests;

use std::time::Instant;

use crate::common::app_state::AppState;
use crate::feat::ui::sidebar::section_trait::{SidebarSection, SidebarSectionId};
use crate::feat::ui::sidebar::sessions::state::sorted_open_sessions;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use throbber_widgets_tui::ThrobberState;

use super::{ANIMATION_INTERVAL, MAX_VISIBLE_SESSIONS};
use entry_line::assemble_entry_line;
use scroll_tag::render_scroll_tag;

/// The open sessions sidebar section.
///
/// Renders all sessions loaded into memory with the active session highlighted.
#[derive(Debug)]
pub struct SessionsSection {
    /// Animation state for the working indicator.
    throbber_state: ThrobberState,
    /// Timestamp of the last animation frame advance.
    last_animation_step: Instant,
}

impl Default for SessionsSection {
    fn default() -> Self {
        Self {
            throbber_state: ThrobberState::default(),
            last_animation_step: Instant::now(),
        }
    }
}

impl SessionsSection {
    /// Creates a new sessions section.
    pub fn new() -> Self {
        Self::default()
    }

    /// Advances the animation frame if enough time has elapsed.
    fn maybe_advance_animation(&mut self) {
        if self.last_animation_step.elapsed() >= ANIMATION_INTERVAL {
            self.throbber_state.calc_next();
            self.last_animation_step = Instant::now();
        }
    }
}

impl SidebarSection for SessionsSection {
    fn id(&self) -> SidebarSectionId {
        SidebarSectionId::Sessions
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        let sessions = sorted_open_sessions(state);
        let theme = &state.frontend.theme;
        let sidebar_focused = state.frontend.scope_stack.is_sidebar();
        let section_focused = sidebar_focused
            && matches!(
                state.frontend.scope_stack.sidebar_section(),
                Some(SidebarSectionId::Sessions)
            );

        let selected_index = state.frontend.sessions_section.selected_index;
        let scroll_offset = state.frontend.sessions_section.scroll_offset;

        let mut lines = Vec::new();

        if sessions.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                " No open sessions",
                Style::default().fg(theme.muted_text),
            )]));
        } else {
            let visible_count = MAX_VISIBLE_SESSIONS.min(sessions.len());
            let start = scroll_offset.min(sessions.len());
            let end = (start + visible_count).min(sessions.len());

            for (visual_i, entry) in sessions[start..end].iter().enumerate() {
                let i = start + visual_i;
                let is_selected = section_focused && selected_index == Some(i);
                let max_title_len = area.width.saturating_sub(5) as usize;
                lines.push(assemble_entry_line(
                    entry,
                    is_selected,
                    max_title_len,
                    &self.throbber_state,
                    theme,
                ));
            }

            // Advance animation only when enough time has elapsed.
            self.maybe_advance_animation();

            // Scroll indicators.
            let lines_above = scroll_offset;
            let lines_below = sessions
                .len()
                .saturating_sub(scroll_offset)
                .saturating_sub(visible_count);

            if lines_above > 0 || lines_below > 0 {
                let indicator_style = Style::default().fg(Color::Black).bg(theme.age_fresh);

                if lines_above > 0 {
                    render_scroll_tag(frame, area, "\u{2191}", area.y, indicator_style);
                }

                if lines_below > 0 {
                    render_scroll_tag(
                        frame,
                        area,
                        "\u{2193}",
                        area.y + visible_count as u16 - 1,
                        indicator_style,
                    );
                }
            }
        }

        // Footer: ╰─── Sessions ───╯ (with highlighted S)
        let label = " Sessions ";
        let width = area.width as usize;
        let label_len = label.len();
        let dash_budget = width.saturating_sub(2).saturating_sub(label_len);
        let left_dashes = dash_budget / 2;
        let right_dashes = dash_budget - left_dashes;
        let before_s = format!("\u{2570}{}\u{0020}", "\u{2500}".repeat(left_dashes),);
        let after_s = format!("essions {}\u{256F}", "\u{2500}".repeat(right_dashes),);

        lines.push(Line::from(vec![
            Span::styled(before_s, Style::default().fg(theme.primary_text)),
            Span::styled("S".to_owned(), Style::default().fg(theme.accent_action)),
            Span::styled(after_s, Style::default().fg(theme.primary_text)),
        ]));

        let widget = Paragraph::new(lines).block(Block::default().borders(Borders::NONE));
        frame.render_widget(widget, area);
    }

    fn content_height(&self, state: &AppState) -> u16 {
        // Count only Loaded sessions, matching sorted_open_sessions filter.
        let session_count = state
            .session
            .iter()
            .filter(|(_, s)| {
                s.session_state() == crate::feat::session::chat_session::SessionState::Loaded
            })
            .count() as u16;
        let visible = session_count.min(MAX_VISIBLE_SESSIONS as u16);
        // entries(N).max(1) + footer(1)
        visible.max(1) + 1 // max(1) for the no-sessions placeholder line
    }
}
