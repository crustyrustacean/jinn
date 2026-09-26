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

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::common::app_state::AppState;
use crate::common::render_ctx::RenderCtx;
use crate::common::ui_element::UiElement;
use crate::protocol::ToolResultStatus;
use crate::protocol::{ChatEntry, ChatEntryId, ChatEntryKind};
use jinn_chat_log_view_msg::{
    DEFAULT_MIN_COLLAPSE_COUNT, PROXIMITY_COUNT, VisualItem, build_visual_items,
};
use jinn_session_msg::PhaseKind;
use jinn_theme::Theme;
use jinn_tools_msg::TASK_TOOL_NAME;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use throbber_widgets_tui::{Throbber, ThrobberState, WhichUse};

use jinn_chat_log_view::chat_log::EntryLineCache;
use jinn_chat_log_view::chat_log::{
    GUTTER_WIDTH, GutterStyle, RenderContext, ScrollState, build_blank_gutter_lines,
    build_collapsed_block_gutter_line, build_entry_gutter_lines, compute_scroll, entry_to_lines,
    find_visible_indices, render_scroll_indicator,
};
use jinn_preferences_config::schemas::ChatLogConfig;

/// Default number of lines to show for tool entries (calls and results) before truncating.
const DEFAULT_TOOL_ENTRY_MAX_LINES: u16 = 6;
// alternatives: |❚┃╏⣿𜺏░▒▓
const GUTTER_STR: &str = "𜺏 ";

/// Hash of the status-derived render inputs that change an entry's rendered
/// lines without changing its content fingerprint (paired tool-result
/// background tint, streaming flag, subagent-waiting line). Used as the
/// render-variant component of the entry line cache key so the cache
/// invalidates when any of them flips.
fn render_variant(
    paired_status: Option<ToolResultStatus>,
    is_streaming: bool,
    is_waiting_on_subagent: bool,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (paired_status, is_streaming, is_waiting_on_subagent).hash(&mut hasher);
    hasher.finish()
}

/// Display element for the full conversation history.
#[derive(Debug, Default)]
pub struct ChatLogElement {
    /// Drives the loading throbber's animation.
    throbber_state: ThrobberState,
    /// Wall-clock of the last animation advance, so the spinner only steps
    /// once the animation interval has elapsed.
    last_advance: Option<Instant>,
}

impl ChatLogElement {
    /// Create a new chat log element.
    #[must_use]
    pub fn new() -> Self {
        Self {
            throbber_state: ThrobberState::default(),
            last_advance: None,
        }
    }
}

impl UiElement for ChatLogElement {
    fn name(&self) -> String {
        "chat-log".to_owned()
    }

    fn is_selectable(&self) -> bool {
        true
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &RenderCtx) {
        let state = ctx.state;
        if state.session.is_loading() {
            render_loading(
                frame,
                area,
                &state.frontend.theme,
                &mut self.throbber_state,
                &mut self.last_advance,
            );
            return;
        }

        let mut render = HistoryRender::new(state, area, ctx.config);
        render.compute_visual_items();
        render.build_tool_result_map();
        {
            let mut cache = state.frontend.caches.entry_line_cache.write();
            render.compute_line_ranges(&mut cache);
            render.compute_scroll();

            {
                let session = state.active_session();
                session.set_last_max_offset(render.scroll.max_offset);
                session.set_entry_line_ranges(render.entry_line_ranges.clone());
                session.set_viewport_height(area.height);
                session.set_blank_count(render.scroll.blank_count as u32);
                session.set_rendered_scroll_offset(render.scroll.clamped);
            }

            render.find_visible_indices();
            render.build_blank_lines();
            render.render_visible_entries(&mut cache);
        }
        render.paint(frame);
    }
}

// ---------------------------------------------------------------------------
// Loading indicator
// ---------------------------------------------------------------------------

/// Minimum time between loading-throbber animation frame advances.
const ANIMATION_INTERVAL: Duration = Duration::from_millis(80);

/// The text shown while a session is loading, with a leading space so it
/// clears the spinner glyph.
const LOADING_LABEL: &str = " Loading session...";

/// The row the loading line is drawn on, counted up from the chat log's bottom.
///
/// The chat log's own area already excludes the indicator and bottom-line rows
/// that `render_chat_tab` reserves, so its last row sits directly above the
/// chat bar. Anchoring there keeps the message clear of both the indicator
/// and the input box.
const LOADING_ROW_FROM_BOTTOM: u16 = 1;

/// Renders an animated "Loading session..." line while a session loads.
///
/// Drawn on the chat log's last row — the row directly above the chat bar — so
/// the message reads as a status line under the conversation rather than
/// floating in the middle of it. Coloured with the same theme color as the
/// streaming indicator's spinner.
fn render_loading(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: &Theme,
    throbber_state: &mut ThrobberState,
    last_advance: &mut Option<Instant>,
) {
    let Some(row) = area
        .height
        .checked_sub(LOADING_ROW_FROM_BOTTOM)
        .filter(|row| *row > 0)
    else {
        return;
    };
    let row_area = Rect {
        y: area.y.saturating_add(row),
        height: 1,
        ..area
    };

    let style = Style::default().fg(theme.streaming);
    let throbber = Throbber::default()
        .label(LOADING_LABEL)
        .style(style)
        .throbber_style(style)
        .throbber_set(throbber_widgets_tui::ASCII)
        .use_type(WhichUse::Spin);

    // `Throbber` renders left-aligned with no alignment option, so centre the
    // line by starting it half the leftover space in. When the log is narrower
    // than the label there is no slack, and it simply starts at the edge.
    let glyph_and_label = u16::try_from(LOADING_LABEL.len())
        .unwrap_or(u16::MAX)
        .saturating_add(1);
    let slack = row_area.width.saturating_sub(glyph_and_label);
    let start = row_area.x.saturating_add(slack / 2);
    let width = row_area.width.min(glyph_and_label).max(1).min(
        row_area
            .x
            .saturating_add(row_area.width)
            .saturating_sub(start),
    );
    if width == 0 {
        return;
    }
    let centered = Rect {
        x: start,
        width,
        ..row_area
    };
    frame.render_stateful_widget(throbber, centered, throbber_state);

    // Advance the animation only once the interval has elapsed, matching the
    // streaming indicator's pacing.
    let now = Instant::now();
    if last_advance.is_none_or(|last| now.duration_since(last) >= ANIMATION_INTERVAL) {
        throbber_state.calc_next();
        *last_advance = Some(now);
    }
}

// ---------------------------------------------------------------------------
// History render pipeline
// ---------------------------------------------------------------------------

/// Accumulates state across the two-pass render pipeline.
///
/// The render pipeline is:
/// 1. `build_tool_result_map` - pair tool calls with their result status
/// 2. `compute_line_ranges` - cache-aware entry line counting (pass 1)
/// 3. `compute_scroll` - blank count, max offset, clamp, scroll-to-selected
/// 4. `find_visible_indices` - determine which entries overlap the viewport
/// 5. `build_blank_lines` - push blank spacer lines above content
/// 6. `render_visible_entries` - build content + gutter lines for visible entries (pass 2)
/// 7. `paint` - render the final paragraph widgets to the frame
struct HistoryRender<'a> {
    // Inputs (set once at construction)
    history: &'a [ChatEntry],
    visual_items: Vec<VisualItem>,
    selected_idx: Option<usize>,
    state: &'a AppState,
    /// This frame's chat-log settings, read once from the configuration
    /// layer. Resolving once per frame rather than per entry keeps the
    /// three call sites below consistent with each other even if a
    /// `reload` lands mid-frame.
    config: ChatLogConfig,
    content_width: u16,
    theme: Theme,
    area: Rect,
    gutter_area: Rect,
    content_area: Rect,

    // Built by pipeline steps
    tool_result_statuses: HashMap<String, ToolResultStatus>,
    /// Ids of the `ToolCall` entries streaming arguments right now.
    ///
    /// Snapshotted once per frame so layout can test membership per entry instead of
    /// scanning the whole history for each tool call.
    streaming_tool_call_ids: HashSet<ChatEntryId>,
    /// Per-visual-item wrapped line ranges: `entry_line_ranges[vi_idx] = (start, end)`.
    entry_line_ranges: Vec<(u32, u32)>,
    miss_lines: HashMap<usize, Vec<Line<'static>>>,
    #[expect(
        clippy::rc_buffer,
        reason = "Vec<Line> not Send, Arc used for cheap clone within same thread"
    )]
    cached_lines: HashMap<usize, Arc<Vec<Line<'static>>>>,
    total_wrapped: u32,
    scroll: ScrollState,
    visible_indices: Vec<usize>,
    content_lines: Vec<Line<'static>>,
    gutter_lines: Vec<Line<'static>>,
    lines_before_viewport: u32,
}

impl<'a> HistoryRender<'a> {
    fn new(state: &'a AppState, area: Rect, config: &jinn_config::ConfigLayer) -> Self {
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
        let streaming_tool_call_ids = state.active_session().streaming_tool_call_ids();
        Self {
            history: state.active_session().history(),
            selected_idx: state.active_session().selected_entry_index(),
            state,
            config: config.read::<ChatLogConfig>(),
            content_width: content_area.width,
            theme: state.frontend.theme.clone(),
            area,
            gutter_area,
            content_area,
            tool_result_statuses: HashMap::new(),
            streaming_tool_call_ids,
            entry_line_ranges: Vec::new(),
            miss_lines: HashMap::new(),
            cached_lines: HashMap::new(),
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
            visual_items: Vec::new(),
        }
    }

    /// Compute visual items from flat history and store on session state.
    ///
    /// Must be called before `compute_line_ranges`.
    fn compute_visual_items(&mut self) {
        let session = self.state.active_session();
        let shown_ignored_blocks = session.shown_ignored_blocks_snapshot();
        let min_collapse = self
            .config
            .min_collapse_count
            .unwrap_or(DEFAULT_MIN_COLLAPSE_COUNT);
        let visual_items = build_visual_items(
            self.history,
            &shown_ignored_blocks,
            PROXIMITY_COUNT,
            min_collapse,
        );
        self.state
            .active_session()
            .set_visual_items(visual_items.clone());
        self.visual_items = visual_items;
    }

    // -----------------------------------------------------------------------
    // Step 1: Build tool result status map
    // -----------------------------------------------------------------------

    /// Whether `entry` is a `ToolCall` still streaming arguments.
    fn is_streaming_tool_call(&self, entry: &ChatEntry) -> bool {
        matches!(&entry.kind, ChatEntryKind::ToolCall { .. })
            && self.streaming_tool_call_ids.contains(&entry.id)
    }

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
    // Step 2: Pass 1 - compute entry line ranges
    // -----------------------------------------------------------------------

    /// Walk all entries, compute wrapped line counts (using cache where possible),
    /// and record the (start, end) wrapped-line range for each entry.
    ///
    /// On a cache hit with rendered lines, the lines are stored in `cached_lines`
    /// for reuse in Pass 2. On a miss, lines are rendered, stored in both the cache
    /// (via `insert_with_lines`) and `miss_lines`.
    #[expect(clippy::expect_used, reason = "infallible")]
    fn compute_line_ranges(&mut self, cache: &mut EntryLineCache) {
        let mut wrapped_cursor: u32 = 0;

        for (vi_idx, item) in self.visual_items.iter().enumerate() {
            match item {
                VisualItem::Entry(hist_idx) => {
                    let entry = self
                        .history
                        .get(*hist_idx)
                        .expect("hist_idx from visual_items");
                    let is_expanded = self.state.active_session().is_entry_expanded(&entry.id);

                    // Variant hash covers status-derived look inputs; a
                    // changed variant forces a re-render even when the
                    // entry's content fingerprint is unchanged.
                    let variant = render_variant(
                        self.paired_status_for_entry(entry),
                        self.is_streaming_tool_call(entry),
                        self.is_task_waiting(entry),
                    );
                    let probe = cache.probe(entry, is_expanded, variant, self.content_width);
                    if let Some(hit) = probe.hit {
                        let start = wrapped_cursor;
                        let end = wrapped_cursor + hit.wrapped_count;
                        self.entry_line_ranges.push((start, end));
                        wrapped_cursor = end;
                        if let Some(lines) = hit.lines {
                            self.cached_lines.insert(vi_idx, lines);
                        }
                    } else {
                        let is_selected = self.selected_idx == Some(vi_idx);
                        let max_lines = self
                            .config
                            .tool_entry_max_lines
                            .unwrap_or(DEFAULT_TOOL_ENTRY_MAX_LINES);
                        let paired_status = self.paired_status_for_entry(entry);
                        let is_streaming = self.is_streaming_tool_call(entry);
                        let is_waiting_on_subagent = self.is_task_waiting(entry);
                        let variant =
                            render_variant(paired_status, is_streaming, is_waiting_on_subagent);
                        let ctx = RenderContext {
                            content_width: self.content_width,
                            is_selected,
                            is_expanded,
                            tool_entry_max_lines: max_lines,
                            theme: self.theme.clone(),
                            paired_status,
                            is_streaming,
                            is_waiting_on_subagent,
                        };
                        let lines = entry_to_lines(entry, &ctx);
                        let wrapped_count: u32 = if self.content_width == 0 {
                            lines.len() as u32
                        } else {
                            Paragraph::new(lines.clone())
                                .wrap(Wrap { trim: false })
                                .line_count(self.content_width) as u32
                        };
                        cache.insert_with_lines(
                            entry,
                            probe.content,
                            is_expanded,
                            variant,
                            self.content_width,
                            wrapped_count,
                            Arc::new(lines.clone()),
                        );

                        let start = wrapped_cursor;
                        let end = wrapped_cursor + wrapped_count;
                        self.entry_line_ranges.push((start, end));
                        wrapped_cursor = end;

                        self.miss_lines.insert(vi_idx, lines);
                    }
                }
                VisualItem::CollapsedIgnoredBlock { .. } => {
                    // Collapsed block is exactly 1 line.
                    let start = wrapped_cursor;
                    let end = wrapped_cursor + 1;
                    self.entry_line_ranges.push((start, end));
                    wrapped_cursor = end;
                }
            }
        }

        self.total_wrapped = wrapped_cursor;
        cache.evict_if_needed();
    }

    /// Look up the paired tool result status for an entry (if applicable).
    fn paired_status_for_entry(&self, entry: &ChatEntry) -> Option<ToolResultStatus> {
        match &entry.kind {
            ChatEntryKind::ToolCall { id, .. } => self.tool_result_statuses.get(id).copied(),
            ChatEntryKind::ToolResult { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// Whether this entry is a `task` tool call still awaiting its result
    /// while its linked child session is loaded in memory and actively
    /// running (sending or streaming).
    ///
    /// Drives the "Waiting for subagent session to complete" render line.
    fn is_task_waiting(&self, entry: &ChatEntry) -> bool {
        let ChatEntryKind::ToolCall {
            id,
            name,
            child_session,
            ..
        } = &entry.kind
        else {
            return false;
        };
        if name != TASK_TOOL_NAME {
            return false;
        }
        // No paired result yet: a pending entry exists only before the tool
        // starts executing, so any status here means the result has landed.
        if self.tool_result_statuses.contains_key(id) {
            return false;
        }
        let Some(child_id) = child_session else {
            return false;
        };
        let Some(child) = self.state.session.get(child_id) else {
            return false;
        };
        matches!(child.phase(), PhaseKind::Sending | PhaseKind::Streaming)
    }

    // -----------------------------------------------------------------------
    // Step 3: Scroll math (delegates to the chat-log-view slice)
    // -----------------------------------------------------------------------

    fn compute_scroll(&mut self) {
        self.scroll = compute_scroll(
            self.area.height,
            self.total_wrapped,
            self.selected_idx,
            &self.entry_line_ranges,
            self.state.active_session().scroll_offset(),
        );
    }

    // -----------------------------------------------------------------------
    // Step 4: Find visible entries (delegates to the chat-log-view slice)
    // -----------------------------------------------------------------------

    fn find_visible_indices(&mut self) {
        self.visible_indices = find_visible_indices(
            &self.entry_line_ranges,
            self.scroll.blank_count,
            self.scroll.clamped,
            self.area.height,
        );
    }

    // -----------------------------------------------------------------------
    // Step 5: Blank lines above content
    // -----------------------------------------------------------------------

    /// Push blank spacer lines above the content when history is shorter than viewport.
    fn build_blank_lines(&mut self) {
        let blank_count = self.scroll.blank_count;
        let viewport_top = self.scroll.clamped;

        if blank_count > 0 && viewport_top < blank_count as u32 {
            for _ in 0..blank_count {
                self.content_lines.push(Line::from(""));
            }
            self.gutter_lines.extend(build_blank_gutter_lines(
                blank_count,
                &self.theme,
                GUTTER_STR,
            ));
            self.lines_before_viewport = viewport_top;
        }
    }

    // -----------------------------------------------------------------------
    // Step 6: Pass 2 - render visible entries
    // -----------------------------------------------------------------------

    /// Store freshly painted lines for an entry and mark them as recently used.
    ///
    /// The wrapped count is read back from the range Pass 1 computed, so the
    /// cache never disagrees with the layout that was just used to paint.
    fn cache_lines(
        &self,
        cache: &mut EntryLineCache,
        entry: &ChatEntry,
        vi_idx: usize,
        is_expanded: bool,
        variant: u64,
        lines: Vec<Line<'static>>,
    ) {
        let wrapped_count = self
            .entry_line_ranges
            .get(vi_idx)
            .map_or(0, |(start, end)| end - start);
        let content = cache
            .probe(entry, is_expanded, variant, self.content_width)
            .content;
        cache.insert_with_lines(
            entry,
            content,
            is_expanded,
            variant,
            self.content_width,
            wrapped_count,
            Arc::new(lines),
        );
        cache.touch(&entry.id);
    }

    /// Build content and gutter lines for all visible entries.
    #[expect(clippy::expect_used, reason = "infallible")]
    fn render_visible_entries(&mut self, cache: &mut EntryLineCache) {
        let viewport_top = self.scroll.clamped;
        let chat_log_active =
            matches!(self.state.frontend.scope(), jinn_slices::FocusScope::Normal);
        let cursor_color = self.theme.focus_accent;

        for &vi_idx in &self.visible_indices {
            let (entry_start, entry_end) = self
                .entry_line_ranges
                .get(vi_idx)
                .copied()
                .expect("vi_idx from visible_indices");
            let abs_entry_start = entry_start + self.scroll.blank_count as u32;

            match self.visual_items.get(vi_idx) {
                Some(VisualItem::Entry(hist_idx)) => {
                    let entry = self
                        .history
                        .get(*hist_idx)
                        .expect("hist_idx from visual_items");
                    let is_selected = self.selected_idx == Some(vi_idx);
                    let is_expanded = self.state.active_session().is_entry_expanded(&entry.id);
                    let max_lines = self
                        .config
                        .tool_entry_max_lines
                        .unwrap_or(DEFAULT_TOOL_ENTRY_MAX_LINES);
                    let variant = render_variant(
                        self.paired_status_for_entry(entry),
                        self.is_streaming_tool_call(entry),
                        self.is_task_waiting(entry),
                    );

                    // Get content lines - cached lines → miss lines → render fresh.
                    let entry_content_lines = if let Some(lines) = self.cached_lines.remove(&vi_idx)
                    {
                        // Painted from the cache, so this entry counts as used.
                        cache.touch(&entry.id);
                        Arc::unwrap_or_clone(lines)
                    } else if let Some(lines) = self.miss_lines.remove(&vi_idx) {
                        // Freshly rendered during layout this frame; store it so
                        // a scroll away and back can reuse it.
                        self.cache_lines(cache, entry, vi_idx, is_expanded, variant, lines.clone());
                        lines
                    } else {
                        // Nothing available: render, then cache for the next frame.
                        let paired_status = self.paired_status_for_entry(entry);
                        let is_streaming = self.is_streaming_tool_call(entry);
                        let is_waiting_on_subagent = self.is_task_waiting(entry);
                        let ctx = RenderContext {
                            content_width: self.content_width,
                            is_selected,
                            is_expanded,
                            tool_entry_max_lines: max_lines,
                            theme: self.theme.clone(),
                            paired_status,
                            is_streaming,
                            is_waiting_on_subagent,
                        };
                        let lines = entry_to_lines(entry, &ctx);
                        self.cache_lines(cache, entry, vi_idx, is_expanded, variant, lines.clone());
                        lines
                    };

                    // Build gutter lines for this entry.
                    let is_pinned = entry.pin_position.is_some();
                    let is_included_in_context = entry.is_in_context();
                    let gutter_ctx = GutterStyle {
                        is_pinned,
                        is_selected,
                        chat_log_active,
                        content_width: self.content_width,
                        // Pass 1 already measured how many rows this entry
                        // wraps to at this width.
                        wrapped_count: entry_end - entry_start,
                        theme: &self.theme,
                        cursor_color,
                        is_included_in_context,
                        gutter_context_color: self.theme.gutter_context_included,
                    };
                    let entry_gutter_lines =
                        build_entry_gutter_lines(&entry_content_lines, &gutter_ctx);

                    // Track lines above viewport for scroll calculation.
                    if abs_entry_start < viewport_top {
                        self.lines_before_viewport += viewport_top.saturating_sub(abs_entry_start);
                    }

                    self.content_lines.extend(entry_content_lines);
                    self.gutter_lines.extend(entry_gutter_lines);
                }
                Some(VisualItem::CollapsedIgnoredBlock { count, .. }) => {
                    let is_selected = self.selected_idx == Some(vi_idx);

                    // Content: gray summary line.
                    let text = format!("{count} hidden entries (press h to show)");
                    let style = Style::default().fg(self.theme.border_unfocused);
                    let line = Line::from(Span::styled(text, style));
                    self.content_lines.push(line);

                    // Gutter: gray indicator with optional cursor.
                    let gutter_line = build_collapsed_block_gutter_line(
                        is_selected,
                        chat_log_active,
                        &self.theme,
                        cursor_color,
                    );
                    self.gutter_lines.push(gutter_line);

                    // Track lines above viewport.
                    if abs_entry_start < viewport_top {
                        self.lines_before_viewport += viewport_top.saturating_sub(abs_entry_start);
                    }
                }
                None => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // Step 7: Paint
    // -----------------------------------------------------------------------

    /// Render the final gutter and content paragraph widgets to the frame.
    fn paint(self, frame: &mut Frame<'_>) {
        // ratatui's `Paragraph::scroll` takes u16, so the u32 line math is
        // narrowed at this boundary. A session long enough to overflow u16 rows
        // cannot be scrolled to in one frame anyway.
        let paragraph_scroll = u16::try_from(self.lines_before_viewport).unwrap_or(u16::MAX);

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

        // Render scroll indicator (delegates to the chat-log-view slice).
        // The indicator is a u16 widget; clamping both values preserves the
        // `clamped >= max_offset` "at the bottom" check it relies on.
        render_scroll_indicator(
            frame,
            self.area,
            u16::try_from(self.scroll.clamped).unwrap_or(u16::MAX),
            u16::try_from(self.scroll.max_offset).unwrap_or(u16::MAX),
            &self.theme,
        );
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use super::*;
    use jinn_testutil::{buffer_row, setup_term};
    use ratatui::style::Color;

    /// Renders the element once with a session load in flight.
    fn render_loading_into(state: &AppState, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let mut element = ChatLogElement::new();
        let (mut terminal, area) = setup_term(width, height);
        {
            let slices = jinn_slices::Slices::new();
            let overlay_views = jinn_slices::OverlayViews::new();
            let ctx = RenderCtx::new_with_default_config(state, &slices, &overlay_views);
            terminal
                .draw(|frame| element.render(frame, area, &ctx))
                .expect("draw");
        }
        terminal.backend().buffer().clone()
    }

    /// A session with a load in flight.
    fn loading_state() -> AppState {
        let mut state = AppState::default();
        state.session.begin_load(jinn_core_types::SessionId::new());
        assert!(state.session.is_loading(), "guard must be set");
        state
    }

    #[rstest::rstest]
    fn loading_line_shows_the_label() {
        // Given a session that is loading.
        let state = loading_state();

        // When rendering the chat log.
        let buffer = render_loading_into(&state, 30, 10);

        // Then the loading label appears.
        let rows: Vec<String> = (0..10)
            .map(|y| {
                (0..30)
                    .map(|x| {
                        buffer
                            .cell((x, y))
                            .map_or("?", ratatui::buffer::Cell::symbol)
                    })
                    .collect()
            })
            .collect();
        assert!(
            rows.iter().any(|row| row.contains("Loading session...")),
            "expected the loading label, got: {rows:?}"
        );
    }

    #[rstest::rstest]
    fn loading_line_sits_directly_above_the_chat_input() {
        // Given a session that is loading in a 10-row area.
        let state = loading_state();

        // When rendering the chat log.
        let buffer = render_loading_into(&state, 30, 10);

        // Then the label sits on the chat log's last row, directly above the
        // indicator and chat bar.
        let label_row = buffer_row(&buffer, 10 - LOADING_ROW_FROM_BOTTOM, 30);
        assert!(
            label_row.contains("Loading session..."),
            "expected the label on row {}, got: {label_row}",
            10 - LOADING_ROW_FROM_BOTTOM
        );
    }

    #[rstest::rstest]
    fn loading_label_uses_the_streaming_color() {
        // Given a session that is loading.
        let state = loading_state();

        // When rendering the chat log.
        let buffer = render_loading_into(&state, 30, 10);

        // Then the label's cells carry the streaming theme color, not the
        // muted grey it used to use.
        let y = 10 - LOADING_ROW_FROM_BOTTOM;
        let label_start = buffer_row(&buffer, y, 30)
            .find("Loading session...")
            .expect("label present") as u16;
        let fg = buffer.cell((label_start, y)).expect("cell").fg;
        assert_eq!(fg, state.frontend.theme.streaming);
        assert_ne!(fg, Color::Gray, "the loading label must not stay grey");
    }

    #[rstest::rstest]
    fn loading_line_is_centred_across_the_log() {
        // Given a session that is loading in a 30-column log.
        let state = loading_state();

        // When rendering the chat log.
        let buffer = render_loading_into(&state, 30, 10);

        // Then the whole line — spinner glyph and label — is centred, leaving
        // roughly equal blank space on either side.
        let y = 10 - LOADING_ROW_FROM_BOTTOM;
        let row = buffer_row(&buffer, y, 30);
        let label_start = row.find("Loading session...").expect("label present") as u16;
        // The spinner glyph sits one column left of the label, which itself
        // starts with a space, so the line begins two columns earlier.
        let start = label_start.saturating_sub(2);
        let end = label_start.saturating_add(LOADING_LABEL.trim().len() as u16);
        let left_gap = start;
        let right_gap = 30u16.saturating_sub(end);
        // The glyph is followed by a space and the label by its own leading
        // space, so the drawn line is two cells wider than the text itself;
        // allow for that when comparing the gaps.
        assert!(
            left_gap.abs_diff(right_gap) <= 2,
            "line should be centred: left gap {left_gap}, right gap {right_gap}, row: {row:?}"
        );
    }

    #[rstest::rstest]
    fn loading_line_is_hidden_when_the_area_is_too_short() {
        // Given a session that is loading in an area with no room for the row.
        let state = loading_state();

        // When rendering into a one-row area.
        let buffer = render_loading_into(&state, 30, 1);

        // Then nothing is drawn.
        assert_eq!(buffer_row(&buffer, 0, 30).trim(), "");
    }
}
