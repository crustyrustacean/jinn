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

use std::collections::HashMap;

use crate::common::app_state::AppState;
use crate::common::ui_element::UiElement;
use crate::feat::session::tool_result_status::ToolResultStatus;
use crate::feat::theme::Theme;
use crate::protocol::{ChatEntry, ChatEntryKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::line_count_cache::EntryLineCache;
use super::shared::{GUTTER_WIDTH, RenderContext};
use super::{
    actor, assistant, compaction, error_entry, skill, system, table, thinking, tool_call,
    tool_result, transient, user,
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

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        if state.session.is_loading() {
            render_loading(frame, area, &state.frontend.theme);
            return;
        }

        let mut render = HistoryRender::new(state, area);
        render.build_tool_result_map();
        render.compute_line_ranges(&mut self.line_cache);
        render.compute_scroll();

        {
            let session = state.active_session();
            session.set_last_max_offset(render.scroll.max_offset);
            session.set_entry_line_ranges(render.entry_line_ranges.clone());
            session.set_viewport_height(area.height);
            session.set_blank_count(render.scroll.blank_count as u16);
        }

        render.find_visible_indices();
        render.build_blank_lines();
        render.render_visible_entries();
        render.paint(frame);
    }
}

// ---------------------------------------------------------------------------
// Loading indicator
// ---------------------------------------------------------------------------

/// Render a centered "Loading session..." message while a session loads.
fn render_loading(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    let loading = Paragraph::new("Loading session...")
        .alignment(ratatui::layout::Alignment::Center)
        .style(Style::default().fg(theme.muted_text))
        .block(Block::default().borders(Borders::NONE));
    frame.render_widget(loading, area);
}

// ---------------------------------------------------------------------------
// Scroll state
// ---------------------------------------------------------------------------

/// Accumulated scroll computation results.
struct ScrollState {
    blank_count: usize,
    max_offset: u16,
    clamped: u16,
}

// ---------------------------------------------------------------------------
// History render pipeline
// ---------------------------------------------------------------------------

/// Accumulates state across the two-pass render pipeline.
///
/// The render pipeline is:
/// 1. `build_tool_result_map` — pair tool calls with their result status
/// 2. `compute_line_ranges` — cache-aware entry line counting (pass 1)
/// 3. `compute_scroll` — blank count, max offset, clamp, scroll-to-selected
/// 4. `find_visible_indices` — determine which entries overlap the viewport
/// 5. `build_blank_lines` — push blank spacer lines above content
/// 6. `render_visible_entries` — build content + gutter lines for visible entries (pass 2)
/// 7. `paint` — render the final paragraph widgets to the frame
struct HistoryRender<'a> {
    // Inputs (set once at construction)
    history: &'a [ChatEntry],
    selected_idx: Option<usize>,
    state: &'a AppState,
    content_width: u16,
    theme: Theme,
    area: Rect,
    gutter_area: Rect,
    content_area: Rect,

    // Built by pipeline steps
    tool_result_statuses: HashMap<String, ToolResultStatus>,
    entry_line_ranges: Vec<(u16, u16)>,
    miss_lines: HashMap<usize, Vec<Line<'static>>>,
    total_wrapped: u16,
    scroll: ScrollState,
    visible_indices: Vec<usize>,
    content_lines: Vec<Line<'static>>,
    gutter_lines: Vec<Line<'static>>,
    lines_before_viewport: u16,
}

impl<'a> HistoryRender<'a> {
    fn new(state: &'a AppState, area: Rect) -> Self {
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
        Self {
            history: state.active_session().history(),
            selected_idx: state.active_session().selected_entry_index(),
            state,
            content_width: content_area.width,
            theme: state.frontend.theme.clone(),
            area,
            gutter_area,
            content_area,
            tool_result_statuses: HashMap::new(),
            entry_line_ranges: Vec::new(),
            miss_lines: HashMap::new(),
            total_wrapped: 0,
            scroll: ScrollState {
                blank_count: 0,
                max_offset: 0,
                clamped: 0,
            },
            visible_indices: Vec::new(),
            content_lines: Vec::new(),
            gutter_lines: Vec::new(),
            lines_before_viewport: 0,
        }
    }

    // -----------------------------------------------------------------------
    // Step 1: Build tool result status map
    // -----------------------------------------------------------------------

    /// Pair tool call IDs with their result status for background coloring.
    fn build_tool_result_map(&mut self) {
        self.tool_result_statuses = self
            .history
            .iter()
            .filter_map(|entry| match &entry.kind {
                ChatEntryKind::ToolResult { id, status, .. } => Some((id.clone(), *status)),
                _ => None,
            })
            .collect();
    }

    // -----------------------------------------------------------------------
    // Step 2: Pass 1 — compute entry line ranges
    // -----------------------------------------------------------------------

    /// Walk all entries, compute wrapped line counts (using cache where possible),
    /// and record the (start, end) wrapped-line range for each entry.
    fn compute_line_ranges(&mut self, cache: &mut EntryLineCache) {
        let mut wrapped_cursor: u16 = 0;

        for (i, entry) in self.history.iter().enumerate() {
            let is_expanded = self.state.active_session().is_entry_expanded(&entry.id);

            if let Some(cached_count) = cache.get(entry, is_expanded, self.content_width) {
                let start = wrapped_cursor;
                let end = wrapped_cursor + cached_count;
                self.entry_line_ranges.push((start, end));
                wrapped_cursor = end;
            } else {
                let is_selected = self.selected_idx == Some(i);
                let max_lines = self
                    .state
                    .frontend
                    .preferences
                    .tool_entry_max_lines
                    .unwrap_or(DEFAULT_TOOL_ENTRY_MAX_LINES);
                let paired_status = self.paired_status_for_entry(entry);
                let ctx = RenderContext {
                    content_width: self.content_width,
                    _is_selected: is_selected,
                    is_expanded,
                    tool_entry_max_lines: max_lines,
                    theme: self.theme.clone(),
                    paired_status,
                };
                let lines = entry_to_lines(entry, &ctx);
                let wrapped_count: u16 = if self.content_width == 0 {
                    lines.len() as u16
                } else {
                    Paragraph::new(lines.clone())
                        .wrap(Wrap { trim: false })
                        .line_count(self.content_width) as u16
                };
                cache.insert(entry, is_expanded, self.content_width, wrapped_count);

                let start = wrapped_cursor;
                let end = wrapped_cursor + wrapped_count;
                self.entry_line_ranges.push((start, end));
                wrapped_cursor = end;

                self.miss_lines.insert(i, lines);
            }
        }

        self.total_wrapped = wrapped_cursor;
    }

    /// Look up the paired tool result status for an entry (if applicable).
    fn paired_status_for_entry(&self, entry: &ChatEntry) -> Option<ToolResultStatus> {
        match &entry.kind {
            ChatEntryKind::ToolCall { id, .. } => self.tool_result_statuses.get(id).copied(),
            ChatEntryKind::ToolResult { status, .. } => Some(*status),
            _ => None,
        }
    }

    // -----------------------------------------------------------------------
    // Step 3: Scroll math
    // -----------------------------------------------------------------------

    /// Compute blank count, max offset, resolve scroll offset, and clamp.
    /// Adjusts clamped offset for scroll-to-selected.
    fn compute_scroll(&mut self) {
        let blank_count = self.area.height.saturating_sub(self.total_wrapped) as usize;
        let total_display = self.total_wrapped + blank_count as u16;
        let max_offset = total_display.saturating_sub(self.area.height);

        let scroll_offset = self.state.active_session().scroll_offset();
        let resolved = scroll_offset.unwrap_or(max_offset);
        let mut clamped = resolved.min(max_offset);

        // Scroll-to-selected: adjust clamped offset to keep selected entry visible.
        if let Some(sel_idx) = self.selected_idx
            && let Some(&(start, end)) = self.entry_line_ranges.get(sel_idx)
        {
            let abs_start = start + blank_count as u16;
            let abs_end = end + blank_count as u16;
            let entry_height = abs_end.saturating_sub(abs_start);
            let viewport_top = clamped;
            let viewport_bottom = clamped.saturating_add(self.area.height);

            if entry_height <= self.area.height {
                if abs_start < viewport_top {
                    clamped = abs_start;
                } else if abs_end > viewport_bottom {
                    clamped = abs_end.saturating_sub(self.area.height);
                }
            } else if abs_start >= viewport_bottom {
                clamped = abs_start;
            } else if abs_end <= viewport_top {
                clamped = abs_end.saturating_sub(self.area.height);
            }
        }

        self.scroll = ScrollState {
            blank_count,
            max_offset,
            clamped,
        };
    }

    // -----------------------------------------------------------------------
    // Step 4: Find visible entries
    // -----------------------------------------------------------------------

    /// Determine which entries overlap the current viewport.
    fn find_visible_indices(&mut self) {
        let viewport_top = self.scroll.clamped;
        let viewport_bottom = self.scroll.clamped.saturating_add(self.area.height);

        self.visible_indices = self
            .entry_line_ranges
            .iter()
            .enumerate()
            .filter_map(|(i, &(start, end))| {
                let abs_start = start + self.scroll.blank_count as u16;
                let abs_end = end + self.scroll.blank_count as u16;
                if abs_end > viewport_top && abs_start < viewport_bottom {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();
    }

    // -----------------------------------------------------------------------
    // Step 5: Blank lines above content
    // -----------------------------------------------------------------------

    /// Push blank spacer lines above the content when history is shorter than viewport.
    fn build_blank_lines(&mut self) {
        let blank_count = self.scroll.blank_count;
        let viewport_top = self.scroll.clamped;

        if blank_count > 0 && viewport_top < blank_count as u16 {
            for _ in 0..blank_count {
                self.content_lines.push(Line::from(""));
                self.gutter_lines.push(Line::from(Span::styled(
                    GUTTER_STR.to_string(),
                    Style::default().fg(self.theme.border_unfocused),
                )));
            }
            self.lines_before_viewport = viewport_top;
        }
    }

    // -----------------------------------------------------------------------
    // Step 6: Pass 2 — render visible entries
    // -----------------------------------------------------------------------

    /// Build content and gutter lines for all visible entries.
    fn render_visible_entries(&mut self) {
        let viewport_top = self.scroll.clamped;
        let chat_log_active = matches!(
            self.state.frontend.scope_stack.current(),
            crate::common::app_state::FocusScope::Normal
        );
        let (gutter_active_color, gutter_inactive_color) =
            { (self.theme.focus_accent, self.theme.border_unfocused) };

        for &i in &self.visible_indices {
            let entry = &self.history[i];
            let is_selected = self.selected_idx == Some(i);
            let is_expanded = self.state.active_session().is_entry_expanded(&entry.id);
            let max_lines = self
                .state
                .frontend
                .preferences
                .tool_entry_max_lines
                .unwrap_or(DEFAULT_TOOL_ENTRY_MAX_LINES);

            let (entry_start, _entry_end) = self.entry_line_ranges[i];
            let abs_entry_start = entry_start + self.scroll.blank_count as u16;

            // Get content lines — reuse from cache miss or render fresh.
            let entry_content_lines = if let Some(lines) = self.miss_lines.remove(&i) {
                lines
            } else {
                let paired_status = self.paired_status_for_entry(entry);
                let ctx = RenderContext {
                    content_width: self.content_width,
                    _is_selected: is_selected,
                    is_expanded,
                    tool_entry_max_lines: max_lines,
                    theme: self.theme.clone(),
                    paired_status,
                };
                entry_to_lines(entry, &ctx)
            };

            // Build gutter lines for this entry.
            let entry_gutter_lines = self.build_entry_gutter_lines(
                &entry_content_lines,
                entry,
                is_selected,
                chat_log_active,
                gutter_active_color,
                gutter_inactive_color,
            );

            // Track lines above viewport for scroll calculation.
            if abs_entry_start < viewport_top {
                self.lines_before_viewport += viewport_top.saturating_sub(abs_entry_start);
            }

            self.content_lines.extend(entry_content_lines);
            self.gutter_lines.extend(entry_gutter_lines);
        }
    }

    /// Build gutter lines for a single entry, handling pin icon, selected style,
    /// pin highlight, and wrap-overflow padding.
    fn build_entry_gutter_lines(
        &self,
        entry_content_lines: &[Line<'static>],
        entry: &ChatEntry,
        is_selected: bool,
        chat_log_active: bool,
        gutter_active_color: ratatui::style::Color,
        gutter_inactive_color: ratatui::style::Color,
    ) -> Vec<Line<'static>> {
        let is_pinned = entry.pin_position.is_some();
        let gutter_style = if is_selected && chat_log_active {
            Style::default().fg(gutter_active_color)
        } else if is_selected {
            Style::default().fg(gutter_inactive_color)
        } else {
            Style::default().fg(self.theme.border_unfocused)
        };
        let gutter_content = if is_pinned { "📌" } else { GUTTER_STR };

        let pin_highlight_style = if is_selected && is_pinned && chat_log_active {
            Style::default()
                .fg(self.theme.gutter_bg)
                .bg(gutter_active_color)
        } else if is_selected && is_pinned {
            Style::default()
                .fg(self.theme.gutter_bg)
                .bg(gutter_inactive_color)
        } else {
            Style::default()
        };

        let entry_wrapped: u16 = if self.content_width == 0 {
            entry_content_lines.len() as u16
        } else {
            Paragraph::new(entry_content_lines.to_vec())
                .wrap(Wrap { trim: false })
                .line_count(self.content_width) as u16
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

        entry_gutter_lines
    }

    // -----------------------------------------------------------------------
    // Step 7: Paint
    // -----------------------------------------------------------------------

    /// Render the final gutter and content paragraph widgets to the frame.
    fn paint(self, frame: &mut Frame<'_>) {
        let paragraph_scroll = self.lines_before_viewport;

        // Render gutter column.
        let gutter_widget = Paragraph::new(self.gutter_lines)
            .block(Block::default().borders(Borders::NONE))
            .scroll((paragraph_scroll, 0));
        frame.render_widget(gutter_widget, self.gutter_area);

        // Render content column.
        let chat_widget = Paragraph::new(self.content_lines)
            .block(Block::default().borders(Borders::NONE))
            .wrap(Wrap { trim: false })
            .scroll((paragraph_scroll, 0));
        frame.render_widget(chat_widget, self.content_area);

        // Render a scroll indicator when the user has scrolled up from the bottom.
        if self.scroll.clamped < self.scroll.max_offset {
            let hidden = self.scroll.max_offset - self.scroll.clamped;
            let label = format!(" ↑ {hidden} lines above ");
            let label_len = label.len();
            let indicator = Paragraph::new(Line::from(Span::styled(
                label,
                Style::default()
                    .fg(self.theme.muted_text)
                    .bg(self.theme.scroll_indicator_bg),
            )));
            let indicator_width = u16::try_from(label_len)
                .unwrap_or(self.area.width)
                .min(self.area.width);
            let indicator_area = Rect {
                x: self.area.x + self.area.width.saturating_sub(indicator_width),
                y: self.area.y + self.area.height.saturating_sub(1),
                width: indicator_width,
                height: 1,
            };
            frame.render_widget(indicator, indicator_area);
        }
    }
}

// ---------------------------------------------------------------------------
// Entry dispatch
// ---------------------------------------------------------------------------

/// Convert a chat entry into one or more visual lines, splitting on `\n`.
///
/// Each entry type is delegated to its own submodule. Lines returned here are
/// content-width only — the gutter is rendered as a separate column.
fn entry_to_lines(entry: &ChatEntry, ctx: &RenderContext) -> Vec<Line<'static>> {
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
        ChatEntryKind::Transient(lines) => transient::to_lines(lines, ctx),
        ChatEntryKind::Compaction {
            summary,
            entries_compacted,
            tokens_before,
            model_used,
        } => compaction::to_lines(summary, *entries_compacted, *tokens_before, model_used, ctx),
    }
}
