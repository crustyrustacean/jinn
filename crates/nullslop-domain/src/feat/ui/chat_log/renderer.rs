//! Renders the conversation history.
//!
//! Each entry in the chat log is displayed with a distinct visual style so the user
//! can tell them apart at a glance:
//!
//! - **User messages** appear as white text on a dark gray background block.
//! - **System messages** appear muted in dark gray.
//! - **Actor messages** appear highlighted with the actor's name and content.
//! - **Assistant messages** appear in white with no background.
//! - **Tool calls** appear as dark text on a dark green background block.
//! - **Tool results** appear as dark text on a dark green (success) or dark red
//!   (failure) background block.
//!
//! A 2-column gutter on the left shows a dark gray background by default,
//! and turns yellow when the cursor selects an entry. Pinned entries show
//! a 📌 emoji in the gutter. When a pinned entry is selected, the gutter
//! background changes to the focus accent color (yellow by default) so the
//! pin highlight is unmistakable.
//!
//! The gutter is rendered as a separate column from the content so that
//! line wrapping does not break the gutter display.
//!
//! Text wraps within the available space.

use crate::common::app_state::AppState;
use crate::common::ui_element::UiElement;
use crate::protocol::ChatEntryKind;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::line_count_cache::EntryLineCache;
use super::shared::{GUTTER_WIDTH, RenderContext};
use super::{
    actor, assistant, compaction, error_entry, info, skill, system, table, thinking, tool_call,
    tool_result, user,
};

/// Default number of lines to show for tool entries (calls and results) before truncating.
const DEFAULT_TOOL_ENTRY_MAX_LINES: u16 = 6;
// alternatives: |❚┃╏⣿𜺏░▒▓
const GUTTER_STR: &str = "𜺏 ";

/// Display element for the full conversation history.
#[derive(Debug)]
pub struct ChatLogElement {
    pub(crate) line_cache: EntryLineCache,
}

impl Default for ChatLogElement {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatLogElement {
    /// Create a new chat log element with a fresh line count cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            line_cache: EntryLineCache::new(),
        }
    }
}

impl UiElement<AppState> for ChatLogElement {
    fn name(&self) -> String {
        "chat-log".to_owned()
    }

    fn is_selectable(&self) -> bool {
        true
    }

    #[allow(clippy::too_many_lines)]
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        // Show loading indicator when a session is being loaded.
        if state.session.is_loading() {
            let loading = Paragraph::new("Loading session...")
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::default().fg(state.frontend.theme.muted_text))
                .block(Block::default().borders(Borders::NONE));
            frame.render_widget(loading, area);
            return;
        }

        let selected_idx = state.active_session().selected_entry_index();
        let history = state.active_session().history();

        // Split area into gutter and content columns.
        let gutter_area = Rect {
            x: area.x,
            y: area.y,
            width: GUTTER_WIDTH,
            height: area.height,
        };
        let content_area = Rect {
            x: area.x + GUTTER_WIDTH,
            y: area.y,
            width: area.width.saturating_sub(GUTTER_WIDTH),
            height: area.height,
        };

        let content_width = content_area.width;

        // Build lines while tracking per-entry wrapped line ranges.
        // entry_line_ranges[i] = (start_wrapped_line, end_wrapped_line) in wrapped coords.
        // --- PASS 1: Compute entry_line_ranges from cache (no rendering). ---
        //
        // Walk all entries. On cache hit, use the cached wrapped count.
        // On cache miss, render the entry to compute its count and store
        // the lines for potential reuse in pass 2.
        let mut entry_line_ranges: Vec<(u16, u16)> = Vec::with_capacity(history.len());
        let mut wrapped_cursor: u16 = 0;
        // Store rendered lines from cache misses for reuse in pass 2.
        let mut miss_lines: std::collections::HashMap<usize, Vec<Line<'static>>> =
            std::collections::HashMap::new();

        for (i, entry) in history.iter().enumerate() {
            let is_expanded = state.active_session().is_entry_expanded(&entry.id);

            if let Some(cached_count) = self.line_cache.get(entry, is_expanded, content_width) {
                // Cache hit — use the cached wrapped count directly.
                let start = wrapped_cursor;
                let end = wrapped_cursor + cached_count;
                entry_line_ranges.push((start, end));
                wrapped_cursor = end;
            } else {
                // Cache miss — render to compute count.
                let is_selected = selected_idx == Some(i);
                let max_lines = state
                    .frontend
                    .preferences
                    .tool_entry_max_lines
                    .unwrap_or(DEFAULT_TOOL_ENTRY_MAX_LINES);
                let ctx = RenderContext {
                    content_width,
                    _is_selected: is_selected,
                    is_expanded,
                    tool_entry_max_lines: max_lines,
                    theme: state.frontend.theme.clone(),
                };
                let lines = entry_to_lines(entry, &ctx);
                let wrapped_count: u16 = if content_width == 0 {
                    lines.len() as u16
                } else {
                    Paragraph::new(lines.clone())
                        .wrap(Wrap { trim: false })
                        .line_count(content_width) as u16
                };
                self.line_cache
                    .insert(entry, is_expanded, content_width, wrapped_count);

                let start = wrapped_cursor;
                let end = wrapped_cursor + wrapped_count;
                entry_line_ranges.push((start, end));
                wrapped_cursor = end;

                // Store lines for reuse if this entry is visible.
                miss_lines.insert(i, lines);
            }
        }

        let total_wrapped = wrapped_cursor;

        // --- Scroll math (same as before, but without content_lines). ---
        let blank_count = area.height.saturating_sub(total_wrapped) as usize;
        let total_display = total_wrapped + blank_count as u16;
        let max_offset = total_display.saturating_sub(area.height);

        state.active_session().set_last_max_offset(max_offset);
        state
            .active_session()
            .set_entry_line_ranges(entry_line_ranges.clone());
        state.active_session().set_viewport_height(area.height);
        state.active_session().set_blank_count(blank_count as u16);

        let scroll_offset = state.active_session().scroll_offset();
        let resolved = scroll_offset.unwrap_or(max_offset);
        let mut clamped = resolved.min(max_offset);

        // Scroll-to-selected: adjust clamped offset to keep selected entry visible.
        if let Some(sel_idx) = selected_idx
            && let Some(&(start, end)) = entry_line_ranges.get(sel_idx)
        {
            let abs_start = start + blank_count as u16;
            let abs_end = end + blank_count as u16;
            let entry_height = abs_end.saturating_sub(abs_start);
            let viewport_top = clamped;
            let viewport_bottom = clamped.saturating_add(area.height);

            if entry_height <= area.height {
                if abs_start < viewport_top {
                    clamped = abs_start;
                } else if abs_end > viewport_bottom {
                    clamped = abs_end.saturating_sub(area.height);
                }
            } else if abs_start >= viewport_bottom {
                clamped = abs_start;
            } else if abs_end <= viewport_top {
                clamped = abs_end.saturating_sub(area.height);
            }
        }

        // --- PASS 2: Render only visible entries. ---
        //
        // Determine which entries overlap the viewport and render their lines.
        // The Paragraph is built from only visible entries, with a local scroll
        // offset to account for lines above the viewport.
        let viewport_top = clamped;
        let viewport_bottom = clamped.saturating_add(area.height);

        // Determine gutter focus state.
        let chat_log_active = matches!(
            state.frontend.scope_stack.current(),
            crate::common::app_state::FocusScope::Normal
        );
        let (gutter_active_color, gutter_inactive_color) = {
            let theme = &state.frontend.theme;
            (theme.focus_accent, theme.border_unfocused)
        };

        // Find visible entry indices.
        let mut visible_indices: Vec<usize> = Vec::new();
        for (i, &(start, end)) in entry_line_ranges.iter().enumerate() {
            let abs_start = start + blank_count as u16;
            let abs_end = end + blank_count as u16;
            if abs_end > viewport_top && abs_start < viewport_bottom {
                visible_indices.push(i);
            }
        }

        let mut content_lines: Vec<Line<'static>> = Vec::new();
        let mut gutter_lines: Vec<Line<'static>> = Vec::new();
        let mut lines_before_viewport: u16 = 0;

        // Blank lines above content.
        if blank_count > 0 && viewport_top < blank_count as u16 {
            // Include blank lines up to the content start.
            for _ in 0..blank_count {
                content_lines.push(Line::from(""));
                gutter_lines.push(Line::from(Span::styled(
                    GUTTER_STR.to_string(),
                    Style::default().fg(state.frontend.theme.border_unfocused),
                )));
            }
            // Blank lines that are above the viewport contribute to the scroll offset.
            lines_before_viewport = viewport_top;
        }

        // Render visible entries.
        for &i in &visible_indices {
            let entry = &history[i];
            let is_selected = selected_idx == Some(i);
            let is_expanded = state.active_session().is_entry_expanded(&entry.id);
            let max_lines = state
                .frontend
                .preferences
                .tool_entry_max_lines
                .unwrap_or(DEFAULT_TOOL_ENTRY_MAX_LINES);

            let (entry_start, _entry_end) = entry_line_ranges[i];
            let abs_entry_start = entry_start + blank_count as u16;

            // Get content lines — reuse from cache miss or render fresh.
            let entry_content_lines = if let Some(lines) = miss_lines.remove(&i) {
                lines
            } else {
                let ctx = RenderContext {
                    content_width,
                    _is_selected: is_selected,
                    is_expanded,
                    tool_entry_max_lines: max_lines,
                    theme: state.frontend.theme.clone(),
                };
                entry_to_lines(entry, &ctx)
            };

            // Build gutter lines.
            let is_pinned = entry.pin_position.is_some();
            let gutter_style = if is_selected && chat_log_active {
                Style::default().fg(gutter_active_color)
            } else if is_selected {
                Style::default().fg(gutter_inactive_color)
            } else {
                Style::default().fg(state.frontend.theme.border_unfocused)
            };
            let gutter_content = if is_pinned { "📌" } else { GUTTER_STR };

            let pin_highlight_style = if is_selected && is_pinned && chat_log_active {
                Style::default()
                    .fg(state.frontend.theme.gutter_bg)
                    .bg(gutter_active_color)
            } else if is_selected && is_pinned {
                Style::default()
                    .fg(state.frontend.theme.gutter_bg)
                    .bg(gutter_inactive_color)
            } else {
                Style::default()
            };

            let entry_wrapped: u16 = if content_width == 0 {
                entry_content_lines.len() as u16
            } else {
                Paragraph::new(entry_content_lines.clone())
                    .wrap(Wrap { trim: false })
                    .line_count(content_width) as u16
            };

            let mut entry_gutter_lines = Vec::new();
            let blank_gutter = Span::styled(GUTTER_STR.to_string(), gutter_style);
            for (j, _) in entry_content_lines.iter().enumerate() {
                let span = if j == 0 && is_pinned {
                    Span::styled(gutter_content.to_owned(), pin_highlight_style)
                } else if j == 0 {
                    Span::styled(gutter_content.to_owned(), gutter_style)
                } else {
                    blank_gutter.clone()
                };
                entry_gutter_lines.push(Line::from(span));
            }

            let logical_count = entry_content_lines.len() as u16;
            if entry_wrapped > logical_count {
                let extra = entry_wrapped - logical_count;
                for _ in 0..extra {
                    entry_gutter_lines.push(Line::from(Span::styled(
                        GUTTER_STR.to_string(),
                        gutter_style,
                    )));
                }
            }

            // Track lines above viewport for scroll calculation.
            if abs_entry_start < viewport_top {
                lines_before_viewport += viewport_top.saturating_sub(abs_entry_start);
            }

            content_lines.extend(entry_content_lines);
            gutter_lines.extend(entry_gutter_lines);
        }

        let paragraph_scroll = lines_before_viewport;

        // Render gutter column.
        let gutter_widget = Paragraph::new(gutter_lines)
            .block(Block::default().borders(Borders::NONE))
            .scroll((paragraph_scroll, 0));
        frame.render_widget(gutter_widget, gutter_area);

        // Render content column.
        let chat_widget = Paragraph::new(content_lines)
            .block(Block::default().borders(Borders::NONE))
            .wrap(Wrap { trim: false })
            .scroll((paragraph_scroll, 0));
        frame.render_widget(chat_widget, content_area);

        // Render a scroll indicator when the user has scrolled up from the bottom.
        if clamped < max_offset {
            let hidden = max_offset - clamped;
            let label = format!(" ↑ {hidden} lines above ");
            let label_len = label.len();
            let indicator = Paragraph::new(Line::from(Span::styled(
                label,
                Style::default()
                    .fg(state.frontend.theme.muted_text)
                    .bg(state.frontend.theme.scroll_indicator_bg),
            )));
            // Render in the bottom-right corner of the chat area.
            let indicator_width = u16::try_from(label_len)
                .unwrap_or(area.width)
                .min(area.width);
            let indicator_area = Rect {
                x: area.x + area.width.saturating_sub(indicator_width),
                y: area.y + area.height.saturating_sub(1),
                width: indicator_width,
                height: 1,
            };
            frame.render_widget(indicator, indicator_area);
        }
    }
}

/// Convert a chat entry into one or more visual lines, splitting on `\n`.
///
/// Each entry type is delegated to its own submodule. Lines returned here are
/// content-width only — the gutter is rendered as a separate column.
fn entry_to_lines(entry: &crate::protocol::ChatEntry, ctx: &RenderContext) -> Vec<Line<'static>> {
    match &entry.kind {
        ChatEntryKind::User { display, .. } => user::to_lines(display, ctx),
        ChatEntryKind::System(text) => system::to_lines(text, ctx),
        ChatEntryKind::Error(text) => error_entry::to_lines(text, ctx),
        ChatEntryKind::Actor { source, text } => actor::to_lines(source, text, ctx),
        ChatEntryKind::Assistant(text) => assistant::to_lines(text, ctx),
        ChatEntryKind::ToolCall {
            name, arguments, ..
        } => tool_call::to_lines(name, arguments, ctx),
        ChatEntryKind::ToolResult {
            name,
            content,
            status,
            truncation,
            ..
        } => tool_result::to_lines(name, content, *status, truncation.as_ref(), ctx),
        ChatEntryKind::Table(data) => table::to_lines(data, ctx),
        ChatEntryKind::Thinking(text) => thinking::to_lines(text, ctx),
        ChatEntryKind::Skill { name, content, .. } => skill::to_lines(name, content, ctx),
        ChatEntryKind::Info(lines) => info::to_lines(lines, ctx),
        ChatEntryKind::Compaction {
            summary,
            entries_compacted,
            tokens_before,
            model_used,
        } => compaction::to_lines(summary, *entries_compacted, *tokens_before, model_used, ctx),
    }
}
