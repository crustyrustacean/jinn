//! [`SessionsSection`] - the open sessions sidebar section.
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

use crate::common::render_ctx::RenderCtx;
use crate::feat::ui::sidebar::section_trait::{SidebarSection, SidebarSectionId};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use throbber_widgets_tui::ThrobberState;

use super::{ANIMATION_INTERVAL, MAX_VISIBLE_SESSIONS};
use crate::feat::ui::sidebar::sessions::archive_tree::ArchiveTreePrompt;
use crate::feat::ui::sidebar::sessions::state::sorted_open_sessions;
use entry_line::assemble_entry_line;
use scroll_tag::render_scroll_tag;
use unicode_width::UnicodeWidthStr;

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

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
        let state = ctx.state;
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

            for (visual_i, entry) in sessions.get(start..end).unwrap_or(&[]).iter().enumerate() {
                let i = start + visual_i;
                let is_selected = section_focused && selected_index == Some(i);
                let max_title_len = area.width.saturating_sub(4) as usize;
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
        let before_s = format!("\u{2570}{}\u{0020}", "\u{2500}".repeat(left_dashes));
        let after_s = format!("essions {}\u{256F}", "\u{2500}".repeat(right_dashes));

        let footer_color = if section_focused {
            theme.focus_accent
        } else {
            theme.border_unfocused
        };

        lines.push(Line::from(vec![
            Span::styled(before_s, Style::default().fg(footer_color)),
            Span::styled("S".to_owned(), Style::default().fg(theme.accent_action)),
            Span::styled(after_s, Style::default().fg(footer_color)),
        ]));

        let widget = Paragraph::new(lines).block(Block::default().borders(Borders::NONE));
        frame.render_widget(widget, area);

        // Close session confirmation prompt - overlay 1 row above the cursor.
        if state.frontend.close_session_prompt
            && section_focused
            && let Some(sel) = selected_index
        {
            render_close_session_prompt(frame, area, sel.saturating_sub(scroll_offset));
        }

        // Archive-tree confirmation prompt - overlay 1 row above the cursor.
        // Yellow confirms ("archive N sessions"); red blocks (a member is busy).
        if let Some(prompt) = &state.frontend.archive_tree_prompt
            && section_focused
            && let Some(sel) = selected_index
        {
            render_archive_tree_prompt(frame, area, prompt, sel.saturating_sub(scroll_offset));
        }
    }

    fn content_height(&self, ctx: &RenderCtx) -> u16 {
        let state = ctx.state;
        let entry_count = sorted_open_sessions(state).len() as u16;
        let visible = entry_count.min(MAX_VISIBLE_SESSIONS as u16);
        // entries(N).max(1) + footer(1)
        visible.max(1) + 1 // max(1) for the no-sessions placeholder line
    }
}

/// Renders the close-session confirmation prompt one row above the cursor.
fn render_close_session_prompt(frame: &mut Frame<'_>, area: Rect, visual_row: usize) {
    let cursor_y = area.y + visual_row as u16;
    let prompt_y = cursor_y.saturating_sub(1);
    let widget = Paragraph::new(Line::from(Span::styled(
        " Press x again to close ",
        Style::default().fg(Color::Black).bg(Color::Yellow),
    )));
    frame.render_widget(
        widget,
        Rect {
            x: area.x,
            y: prompt_y,
            width: area.width,
            height: 1,
        },
    );
}

/// Renders the archive-tree confirmation prompt one row above the cursor.
///
/// Yellow = armed confirm ("Press A again to archive N sessions"); red =
/// blocked (a member of the subtree is busy).
fn render_archive_tree_prompt(
    frame: &mut Frame<'_>,
    area: Rect,
    prompt: &ArchiveTreePrompt,
    visual_row: usize,
) {
    let (text, bg) = match prompt {
        ArchiveTreePrompt::Confirm { count } => (
            format!(
                " Press A again to archive {count} session{} ",
                if *count == 1 { "" } else { "s" }
            ),
            Color::Yellow,
        ),
        ArchiveTreePrompt::Busy => (
            " Cannot archive tree while a session is busy ".to_owned(),
            Color::Red,
        ),
    };
    let cursor_y = area.y + visual_row as u16;
    let prompt_y = cursor_y.saturating_sub(1);
    // Right-align the banner so the full message fits: pin its end to the
    // area's right edge, clamped into the area when the term is narrow.
    let text_width = text.width() as u16;
    let prompt_x = {
        let unclamped_end = area.x + area.width;
        let end = unclamped_end.min(area.x.saturating_add(text_width));
        end.saturating_sub(text_width)
    };
    let widget = Paragraph::new(Line::from(Span::styled(
        text,
        Style::default().fg(Color::Black).bg(bg),
    )));
    frame.render_widget(
        widget,
        Rect {
            x: prompt_x,
            y: prompt_y,
            width: text_width,
            height: 1,
        },
    );
}
