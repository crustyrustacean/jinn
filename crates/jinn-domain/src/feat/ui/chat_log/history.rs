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
use jinn_core_types::SessionId;
use jinn_session_msg::PhaseKind;
use jinn_session_state::ChatSessionState;
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

/// Default number of lines to show for tool entries (calls and results) before truncating.
const DEFAULT_TOOL_ENTRY_MAX_LINES: u16 = 6;

/// Minimum time between loading-indicator animation frames.
const LOADING_ANIMATION_INTERVAL: Duration = Duration::from_millis(80);

/// The text drawn beside the spinner glyph while a session is loading.
const LOADING_LABEL: &str = " Loading session...";

/// Width of the loading indication, in columns.
///
/// `Throbber` has no alignment, so centering is done by handing it a sub-rectangle
/// this wide. One column is added for the spinner's trailing space, so the label
/// is not clipped when the indication fits.
const LOADING_INDICATION_WIDTH: u16 = LOADING_LABEL.len() as u16 + 1;

// alternatives: |❚┃╏⣿𜺏░▒▓
const GUTTER_STR: &str = "𜺏 ";

/// Hash of the status-derived render inputs that change an entry's rendered
/// lines without changing its content fingerprint (paired tool-result
/// background tint, streaming flag, subagent-waiting line). Used as the
/// render-variant component of the entry line cache key so the cache
/// invalidates when any of them flips.
///
/// Shared with the off-thread layout worker, which must produce byte-identical
/// variants or every count it publishes would miss.
pub(crate) fn render_variant(
    paired_status: Option<ToolResultStatus>,
    is_streaming: bool,
    is_waiting_on_subagent: bool,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (paired_status, is_streaming, is_waiting_on_subagent).hash(&mut hasher);
    hasher.finish()
}

/// The per-entry render inputs that live in application state rather than in
/// the history itself.
///
/// Snapshotted once per layout job so the off-thread measurement sees a stable
/// set: the render pass gathers the same three things every frame, and a
/// measurement taken across a state change would produce counts that no frame
/// could ever hit.
pub(crate) struct LayoutInputs {
    /// Theme colors, cloned once per job rather than once per entry.
    theme: Theme,
    /// Entries whose tool result content is expanded.
    expanded: HashSet<ChatEntryId>,
    /// Tool call entries still streaming their arguments.
    streaming: HashSet<ChatEntryId>,
    /// Child sessions loaded and actively running, by session id.
    running_children: HashSet<SessionId>,
}

impl LayoutInputs {
    /// Snapshots the layout inputs for one session.
    pub(crate) fn snapshot(state: &AppState, session_id: &SessionId) -> Self {
        use jinn_session_msg::PhaseKind;

        let running_children = state
            .session
            .iter()
            .filter(|(_, child)| matches!(child.phase(), PhaseKind::Sending | PhaseKind::Streaming))
            .map(|(id, _)| id.clone())
            .collect();

        let session = state.session.get(session_id);
        Self {
            theme: state.frontend.theme.clone(),
            expanded: session.map_or_else(HashSet::new, ChatSessionState::expanded_entry_ids),
            streaming: session.map_or_else(HashSet::new, ChatSessionState::streaming_tool_call_ids),
            running_children,
        }
    }

    /// Whether this entry's tool result content is expanded.
    pub(crate) fn is_expanded(&self, id: &ChatEntryId) -> bool {
        self.expanded.contains(id)
    }

    /// The theme to render this job's entries with.
    pub(crate) fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Whether this tool call is still streaming its arguments.
    pub(crate) fn is_streaming(&self, id: &ChatEntryId) -> bool {
        self.streaming.contains(id)
    }

    /// Whether this `task` call is waiting on a loaded, running child session.
    pub(crate) fn is_task_waiting(
        &self,
        entry: &ChatEntry,
        tool_result_statuses: &HashMap<String, ToolResultStatus>,
    ) -> bool {
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
        // A paired result means the tool already finished.
        if tool_result_statuses.contains_key(id) {
            return false;
        }
        child_session
            .as_ref()
            .is_some_and(|child| self.running_children.contains(child))
    }
}

/// Display element for the full conversation history.
#[derive(Debug)]
pub struct ChatLogElement {
    /// Visual-only state for the loading throbber's animation step.
    throbber_state: ThrobberState,
    /// Timestamp of the last animation frame advance.
    last_animation_step: Instant,
}

impl ChatLogElement {
    /// Create a new chat log element.
    #[must_use]
    pub fn new() -> Self {
        Self {
            throbber_state: ThrobberState::default(),
            last_animation_step: Instant::now(),
        }
    }

    /// Advances the loading animation if enough time has elapsed.
    ///
    /// A session load on a large session is long enough that a static label
    /// reads as a hang, so the indication has to visibly move.
    fn maybe_advance_animation(&mut self) {
        if self.last_animation_step.elapsed() >= LOADING_ANIMATION_INTERVAL {
            self.throbber_state.calc_next();
            self.last_animation_step = Instant::now();
        }
    }
}

impl Default for ChatLogElement {
    fn default() -> Self {
        Self::new()
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
            render_loading_animated(frame, area, &state.frontend.theme, &mut self.throbber_state);
            // Advanced after drawing, so the painted glyph is the one this
            // frame's state describes.
            self.maybe_advance_animation();
            return;
        }

        let mut render = HistoryRender::new(state, area);
        render.compute_visual_items();
        render.build_tool_result_map();
        {
            let mut cache = state.frontend.caches.entry_line_cache.write();
            render.compute_line_ranges(&mut cache);
            render.compute_scroll();

            {
                let session = state.active_session();
                session.set_last_max_offset(render.scroll.max_offset);
                session.set_entry_line_ranges_if_changed(&render.entry_line_ranges);
                session.set_viewport_height(area.height);
                session.set_blank_count(render.scroll.blank_count as u32);
                session.set_rendered_scroll_offset(render.scroll.clamped);
                // Published so a session loaded later measures at the width
                // this frame used, instead of being measured at a guessed
                // width and thrown away as stale.
                session.set_content_width(render.content_width);
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

/// Draws the animated loading indication for a session that is being read from
/// disk and measured.
///
/// Centred within the chat log's own area rather than the terminal: the pane is
/// what the user is looking at while it fills, and a spinner drifting away from
/// the pane's centre reads as belonging to something else. A zero-sized pane has
/// no row to centre within and no column to centre across, so it is left blank
/// rather than having the indication drawn outside it.
fn render_loading_animated(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: &Theme,
    throbber_state: &mut ThrobberState,
) {
    let width = area.width.min(LOADING_INDICATION_WIDTH);
    if width == 0 || area.height == 0 {
        return;
    }
    let centered = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(1) / 2,
        width,
        height: 1,
    };
    let throbber = Throbber::default()
        .label(LOADING_LABEL)
        .style(Style::default().fg(theme.muted_text))
        .throbber_style(Style::default().fg(theme.streaming))
        .throbber_set(throbber_widgets_tui::ASCII)
        .use_type(WhichUse::Spin);
    frame.render_stateful_widget(throbber, centered, throbber_state);
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
        reason = "Arc keeps cloning a rendered entry's line buffer O(1) where a plain Vec would deep-copy every line on each cache hit"
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
        let streaming_tool_call_ids = state.active_session().streaming_tool_call_ids();
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
    /// Must be called before `compute_line_ranges`. The computed list is
    /// published to the session's view state only when it differs from the
    /// stored one, so a frame over unchanged history does not copy the list
    /// back.
    fn compute_visual_items(&mut self) {
        let shown_ignored_blocks = {
            let session = self.state.active_session();
            session.shown_ignored_blocks_snapshot()
        };
        let min_collapse = self
            .state
            .frontend
            .preferences
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
            .set_visual_items_if_changed(&visual_items);
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
                            .state
                            .frontend
                            .preferences
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
                        .state
                        .frontend
                        .preferences
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
