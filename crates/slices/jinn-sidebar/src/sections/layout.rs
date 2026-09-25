//! Sidebar document layout — one contiguous scroll model for every section.
//!
//! The sidebar renders its sections as a single document of rows. A section's
//! content height places it at a fixed `first_row` in that document; the column
//! shows a window of `viewport_rows` starting at `offset`.
//!
//! The offset is a pure function of the focused section's cursor:
//!
//! ```text
//! max_offset = total_rows.saturating_sub(viewport_rows)
//! offset     = (cursor_row - viewport_rows / 2).clamp(0, max_offset)
//! ```
//!
//! Centering the cursor keeps it visible without any per-section scroll state.
//! Clamping at both ends means shrinking content (unpinning, removing a phase)
//! needs no separate reaction — the same expression lands on a valid window.
//! When the document is shorter than the column there is nothing to scroll, so
//! the offset is 0 and the sections keep their natural top-down placement.

use jinn_domain::common::app_state::AppState;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::section_trait::SidebarSectionId;
use super::sessions::state::sorted_open_sessions;

/// A section's placement within the sidebar document.
///
/// `first_row` is the section's first row in document coordinates; `rows` is
/// its content height. Zero-height sections still get a span so the table stays
/// a complete, contiguous description of the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionSpan {
    /// Which section this span describes.
    pub id: SidebarSectionId,
    /// The section's first row in document coordinates.
    pub first_row: u16,
    /// The section's content height in rows.
    pub rows: u16,
}

impl SectionSpan {
    /// The first document row at or after `offset` that this span covers.
    ///
    /// Saturates at zero for a section scrolled entirely above the window.
    #[must_use]
    pub const fn top_in_view(&self, offset: u16) -> u16 {
        self.first_row.saturating_sub(offset)
    }

    /// The number of rows of this span that fall inside a window of
    /// `viewport_rows` starting at `offset`.
    ///
    /// Zero when the span is scrolled off either end. Computed by intersecting
    /// the span's document row range with the window's, so a span that starts
    /// above the window contributes only its visible tail.
    #[must_use]
    pub fn visible_height(&self, offset: u16, viewport_rows: u16) -> u16 {
        let window_end = offset.saturating_add(viewport_rows);
        let span_end = self.first_row.saturating_add(self.rows);
        // Intersection of [first_row, span_end) with [offset, window_end).
        let start = self.first_row.max(offset);
        let end = span_end.min(window_end);
        end.saturating_sub(start)
    }
}

/// The full placement table for one frame.
#[derive(Debug, Clone)]
pub struct DocumentLayout {
    /// Every section in registration order, including collapsed ones.
    pub spans: Vec<SectionSpan>,
    /// Total rows across all sections.
    pub total_rows: u16,
    /// The focused section's cursor row, or `None` when there is no cursor.
    pub cursor_row: Option<u16>,
}

impl DocumentLayout {
    /// The first document row rendered at the top of the column.
    ///
    /// Centers the cursor and clamps to the document's ends, so this is always
    /// a window the cursor sits inside whenever the document is taller than the
    /// column. A document shorter than the column has nothing to scroll, so the
    /// offset stays at 0.
    #[must_use]
    pub fn offset(&self, viewport_rows: u16) -> u16 {
        let max_offset = self.total_rows.saturating_sub(viewport_rows);
        let Some(cursor_row) = self.cursor_row else {
            return 0;
        };
        cursor_row.saturating_sub(viewport_rows / 2).min(max_offset)
    }

    /// The slack between the document and the column when the document fits.
    ///
    /// Positive when the document is shorter than the column, which is the gap
    /// the sessions block is pushed down by so it sits at the bottom of a tall
    /// terminal. Zero once the document overflows and scrolling takes over.
    #[must_use]
    pub fn bottom_slack(&self, viewport_rows: u16) -> u16 {
        viewport_rows.saturating_sub(self.total_rows)
    }

    /// Returns the span for `id`, if that section has one.
    #[must_use]
    pub fn span(&self, id: SidebarSectionId) -> Option<&SectionSpan> {
        self.spans.iter().find(|span| span.id == id)
    }

    /// The span for `id`, treating an absent section as a zero-height one.
    ///
    /// Keeps popup anchoring total: asking for a section that is not in the
    /// table yields a zero span rather than an error.
    #[must_use]
    pub fn span_or_empty(&self, id: SidebarSectionId) -> SectionSpan {
        self.span(id).copied().unwrap_or(SectionSpan {
            id,
            first_row: 0,
            rows: 0,
        })
    }

    /// Whether `id` is the trailing section of the document.
    ///
    /// The trailing section is the one pushed down to the bottom of a column
    /// that has rows to spare; every other section stays at the top.
    #[must_use]
    pub fn is_last(&self, id: SidebarSectionId) -> bool {
        self.spans.last().is_some_and(|span| span.id == id)
    }
}

// ---------------------------------------------------------------------------
// Content heights
// ---------------------------------------------------------------------------

/// Persona section content height: header, blank, entry, trailing gap.
const PERSONA_ROWS: u16 = 4;

/// Rows a section spends on its header, blank separator, and trailing gap.
///
/// Entries occupy whatever is left, so a section with `n` entries is
/// `CHROME_ROWS + n`.
const CHROME_ROWS: u16 = 3;

/// Persona section content height in rows.
#[must_use]
pub fn persona_rows(_state: &AppState) -> u16 {
    PERSONA_ROWS
}

/// Pins section content height in rows; zero when nothing is pinned.
#[must_use]
pub fn pins_rows(state: &AppState) -> u16 {
    let count = state.active_session().pinned_entries().len();
    if count == 0 {
        return 0;
    }
    CHROME_ROWS.saturating_add(count as u16)
}

/// Task list section content height in rows; zero when the list is empty.
///
/// Delegates to the section's own width-aware measurement so the document table
/// and the rendered phase headers can never disagree about how many rows a
/// wrapped description takes.
#[must_use]
pub fn task_list_rows(state: &AppState) -> u16 {
    super::task_list_section::task_list_content_height(state)
}

/// McpServers section content height in rows; zero when no server is enabled.
#[must_use]
pub fn mcp_servers_rows(state: &AppState) -> u16 {
    let enabled = state.active_session().enabled_mcp_servers();
    let count = state
        .frontend
        .preferences
        .mcp_server
        .iter()
        .filter(|(name, _)| enabled.contains(name.as_str()))
        .count();
    if count == 0 {
        return 0;
    }
    CHROME_ROWS.saturating_add(count as u16)
}

/// Sessions section content height in rows: every entry plus the footer.
///
/// Uncapped — the document window is what limits what is drawn, so every
/// loaded session stays reachable. The `max(1)` keeps a placeholder row for the
/// empty list so the footer does not float up.
#[must_use]
pub fn sessions_rows(state: &AppState) -> u16 {
    let entry_count = sorted_open_sessions(state).len() as u16;
    entry_count.max(1).saturating_add(1)
}

// ---------------------------------------------------------------------------
// Document table
// ---------------------------------------------------------------------------

/// Every section in registration order, which is also document order.
///
/// The `SidebarSectionId` enum declares a different order; this list is
/// authoritative because `Sidebar` stores sections in registration order and
/// renders them top-down. Sessions must stay last — it is the document tail.
pub const REGISTRATION_ORDER: [SidebarSectionId; 5] = [
    SidebarSectionId::Persona,
    SidebarSectionId::Pins,
    SidebarSectionId::TaskList,
    SidebarSectionId::McpServers,
    SidebarSectionId::Sessions,
];

/// The content height of `id` for the current state.
#[must_use]
pub fn content_height_of(state: &AppState, id: SidebarSectionId) -> u16 {
    match id {
        SidebarSectionId::Persona => persona_rows(state),
        SidebarSectionId::Pins => pins_rows(state),
        SidebarSectionId::TaskList => task_list_rows(state),
        SidebarSectionId::McpServers => mcp_servers_rows(state),
        SidebarSectionId::Sessions => sessions_rows(state),
    }
}

/// Builds the document placement table for the current state.
#[must_use]
pub fn document(state: &AppState) -> DocumentLayout {
    document_for(state, &REGISTRATION_ORDER)
}

/// Builds the document placement table for an explicit set of sections.
///
/// `ids` must be the sections actually registered with the sidebar, in
/// registration order. Building the table from the real list — rather than from
/// a fixed order — keeps the total height in agreement with what gets rendered,
/// so a partially registered sidebar lays out correctly.
#[must_use]
pub fn document_for(state: &AppState, ids: &[SidebarSectionId]) -> DocumentLayout {
    let spans = build_spans(state, ids);
    let total_rows = spans
        .iter()
        .fold(0u16, |total, span| total.saturating_add(span.rows));
    DocumentLayout {
        spans,
        total_rows,
        cursor_row: None,
    }
}

/// Accumulates each section's height into a contiguous prefix-sum table.
fn build_spans(state: &AppState, ids: &[SidebarSectionId]) -> Vec<SectionSpan> {
    let mut spans = Vec::with_capacity(ids.len());
    let mut first_row = 0u16;
    for id in ids {
        let rows = content_height_of(state, *id);
        spans.push(SectionSpan {
            id: *id,
            first_row,
            rows,
        });
        first_row = first_row.saturating_add(rows);
    }
    spans
}

/// Builds the document table including the focused section's cursor row.
#[must_use]
pub fn document_with_cursor(state: &AppState) -> DocumentLayout {
    with_cursor(document(state), state)
}

/// The full registration order as a slice, for callers that render a sidebar
/// with every built-in section.
#[must_use]
pub fn full_registration_order() -> &'static [SidebarSectionId] {
    &REGISTRATION_ORDER
}

/// Adds the focused section's cursor row to an existing table.
#[must_use]
pub fn with_cursor(mut document: DocumentLayout, state: &AppState) -> DocumentLayout {
    document.cursor_row = focused_cursor_row(state, &document);
    document
}

// ---------------------------------------------------------------------------
// Cursor rows
// ---------------------------------------------------------------------------

/// The focused section's cursor row in document coordinates.
///
/// `None` when no section is focused or the focused section has no cursor —
/// callers then leave the window at the top of the document.
#[must_use]
pub fn focused_cursor_row(state: &AppState, document: &DocumentLayout) -> Option<u16> {
    let focused = state.frontend.sidebar_section()?;
    let row_in_section = cursor_row_in_section(state, focused)?;
    let first_row = document.span_or_empty(focused).first_row;
    Some(first_row.saturating_add(row_in_section))
}

/// A section's cursor row relative to the section's own first rendered row.
///
/// Returns `None` when the section has no cursor to keep visible.
#[must_use]
pub fn cursor_row_in_section(state: &AppState, id: SidebarSectionId) -> Option<u16> {
    match id {
        SidebarSectionId::Persona => persona_cursor_row(state),
        SidebarSectionId::Pins => pins_cursor_row(state),
        SidebarSectionId::TaskList => task_list_cursor_row(state),
        SidebarSectionId::McpServers => mcp_servers_cursor_row(state),
        SidebarSectionId::Sessions => sessions_cursor_row(state),
    }
}

/// Persona has a single entry below its header and blank separator.
fn persona_cursor_row(state: &AppState) -> Option<u16> {
    let has_cursor = state
        .frontend
        .with_sections(|sections| sections.persona.cursor.is_some(), || false);
    has_cursor.then_some(2)
}

/// Pins entries start after the header and blank separator.
fn pins_cursor_row(state: &AppState) -> Option<u16> {
    let sorted_ids = state.sorted_pinned_ids();
    let has_selection = state
        .frontend
        .with_sections(|sections| sections.pins.selected_id().is_some(), || false);
    if !has_selection {
        return None;
    }
    let index = state
        .frontend
        .with_sections(|sections| sections.pins.selection_index(&sorted_ids), || 0);
    Some(2u16.saturating_add(index as u16))
}

/// The row of the selected phase's first header line.
///
/// Only headers render inline — a phase's tasks live in the preview popup — so
/// the cursor sits on the header's first row, not its middle.
fn task_list_cursor_row(state: &AppState) -> Option<u16> {
    let selected = state
        .frontend
        .with_sections(|sections| sections.task_list.selected_phase_index, || None)?;
    let heights = super::task_list_section::phase_row_heights(state);
    // Header and blank separator come first, then every phase before the
    // selected one contributes its wrapped header rows.
    let preceding: u16 = heights
        .iter()
        .take(selected)
        .fold(0u16, |total, rows| total.saturating_add(*rows));
    Some(2u16.saturating_add(preceding))
}

/// McpServers entries start after the header and blank separator.
fn mcp_servers_cursor_row(state: &AppState) -> Option<u16> {
    let index = state
        .frontend
        .with_sections(|sections| sections.mcp_servers.selected_index, || None)?;
    Some(2u16.saturating_add(index as u16))
}

/// Sessions has no header — entries start at the section's first row.
fn sessions_cursor_row(state: &AppState) -> Option<u16> {
    let index = state
        .frontend
        .with_sections(|sections| sections.sessions.selected_index, || None)?;
    Some(index as u16)
}

// ---------------------------------------------------------------------------
// Screen placement
// ---------------------------------------------------------------------------

/// The rect a section occupies in the column for a given window.
///
/// Returns `None` when the section is scrolled entirely out of view. The
/// returned rect's `y` may sit above the column when the section is partially
/// scrolled off the top; `skip_rows` carries how many of its own rows are
/// hidden above, which the section must drop before drawing.
#[must_use]
pub fn visible_rect(
    area: Rect,
    span: SectionSpan,
    offset: u16,
    push_down: u16,
) -> Option<(Rect, u16)> {
    let height = span.visible_height(offset, area.height);
    if height == 0 {
        return None;
    }
    let rect = Rect {
        x: area.x,
        y: area
            .y
            .saturating_add(span.top_in_view(offset))
            .saturating_add(push_down),
        width: area.width,
        height,
    };
    // The section's own first row sits at document row `first_row`; when the
    // window starts below it, that many of its leading rows are already gone.
    let skip_rows = offset.saturating_sub(span.first_row).min(span.rows);
    Some((rect, skip_rows))
}

/// Resolves a (section, row) pair to an absolute row in the frame.
///
/// Applies the same scroll offset and bottom slack the `Sidebar` container
/// renders with, so overlays anchored to a row stay attached to it while the
/// column scrolls. The result is clamped inside the column.
#[must_use]
pub fn frame_row_of(sidebar_rect: Rect, state: &AppState, id: SidebarSectionId, row: u16) -> u16 {
    let document = document_with_cursor(state);
    let offset = document.offset(sidebar_rect.height);
    let slack = document.bottom_slack(sidebar_rect.height);
    let top = document.span_or_empty(id).top_in_view(offset);
    // Only the trailing section is pushed down by the slack; the leading
    // sections stay at the top of the column, matching `Sidebar::render`.
    let push_down = if document.is_last(id) { slack } else { 0 };
    sidebar_rect
        .y
        .saturating_add(push_down)
        .saturating_add(top)
        .saturating_add(row)
        .min(
            sidebar_rect
                .y
                .saturating_add(sidebar_rect.height.saturating_sub(1)),
        )
}

/// Draws the column's scroll indicators, if the document overflows the window.
///
/// One pair for the whole column rather than one per section, since there is
/// now a single scroll position.
pub fn render_scroll_indicators(
    frame: &mut Frame<'_>,
    area: Rect,
    offset: u16,
    total_rows: u16,
    theme: &jinn_theme::Theme,
) {
    let viewport_rows = area.height;
    let has_above = offset > 0;
    let has_below = offset.saturating_add(viewport_rows) < total_rows;
    if !has_above && !has_below {
        return;
    }
    let style = Style::default().fg(Color::Black).bg(theme.age_fresh);
    if has_above {
        render_scroll_tag(frame, area, "\u{2191}", area.y, style);
    }
    if has_below {
        render_scroll_tag(frame, area, "\u{2193}", area.y + viewport_rows - 1, style);
    }
}

/// Renders a single scroll indicator at the column's right edge.
fn render_scroll_tag(frame: &mut Frame<'_>, area: Rect, label: &str, y: u16, style: Style) {
    let tag_area = Rect {
        x: area.x + area.width.saturating_sub(1),
        y,
        width: 1,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(label, style))),
        tag_area,
    );
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        reason = "test code"
    )]
    use jinn_domain::common::app_state::AppState;
    use jinn_domain::protocol::ChatEntry;
    use jinn_domain::protocol::PinPosition;
    use jinn_session_state::ChatSessionState;
    use jinn_tools_msg::{PhaseInput, TaskStatus};

    use super::{DocumentLayout, SectionSpan, content_height_of, document, sessions_rows};
    use crate::sections::section_trait::SidebarSectionId;

    /// A state with `pins` pinned entries and `phases` single-task phases.
    fn state_with(pins: usize, phases: usize) -> AppState {
        let mut state = AppState::default_with_scope_focus();
        for i in 0..pins {
            let entry = ChatEntry::user(format!("entry {i}"));
            let id = entry.id.clone();
            state.active_session_mut().push_entry(entry);
            state.active_session_mut().pin_entry(&id, PinPosition::Top);
        }
        let inputs: Vec<PhaseInput> = (0..phases)
            .map(|i| PhaseInput {
                description: format!("Phase {i}"),
                tasks: vec![("do the thing".to_owned(), TaskStatus::Pending)],
            })
            .collect();
        state
            .active_session_mut()
            .task_list_mut()
            .set_from_inputs(&inputs);
        state
    }

    /// A state with `count` extra loaded sessions.
    fn state_with_sessions(count: usize) -> AppState {
        let mut state = AppState::default_with_scope_focus();
        for i in 1..count {
            state.session.insert({
                let mut session = ChatSessionState::new();
                session.push_entry(ChatEntry::user(format!("message {i}")));
                session
            });
        }
        state
    }

    #[rstest::rstest]
    fn total_rows_sums_every_section_height() {
        // Given state with 3 pins, 2 phases, and 4 sessions.
        let state = state_with(3, 2);
        let state = {
            let mut state = state;
            for i in 1..4 {
                state.session.insert({
                    let mut session = ChatSessionState::new();
                    session.push_entry(ChatEntry::user(format!("extra {i}")));
                    session
                });
            }
            state
        };

        // When building the document table.
        let document = document(&state);

        // Then the total is the sum of the individual section heights.
        let sum: u16 = document
            .spans
            .iter()
            .map(|span| content_height_of(&state, span.id))
            .sum();
        assert_eq!(document.total_rows, sum);
    }

    #[rstest::rstest]
    fn spans_cover_a_contiguous_document() {
        // Given state with pins and phases.
        let state = state_with(4, 3);

        // When building the document table.
        let document = document(&state);

        // Then each span starts where the previous one ended.
        let mut expected_first_row = 0u16;
        for span in &document.spans {
            assert_eq!(
                span.first_row, expected_first_row,
                "span {:?} should start at {expected_first_row}",
                span.id
            );
            expected_first_row = expected_first_row.saturating_add(span.rows);
        }
        // And the last span ends at the total.
        assert_eq!(expected_first_row, document.total_rows);
    }

    #[rstest::rstest]
    fn offset_is_zero_when_the_document_shorter_than_the_viewport_fits() {
        // Given a document of 6 rows and a 40-row column.
        let document = DocumentLayout {
            spans: Vec::new(),
            total_rows: 6,
            cursor_row: Some(2),
        };

        // When resolving the offset.
        let offset = document.offset(40);

        // Then there is nothing to scroll.
        assert_eq!(offset, 0);
    }

    #[rstest::rstest]
    fn offset_is_zero_when_there_is_no_cursor() {
        // Given a tall document with no focused cursor.
        let document = DocumentLayout {
            spans: Vec::new(),
            total_rows: 100,
            cursor_row: None,
        };

        // When resolving the offset.
        let offset = document.offset(20);

        // Then the window starts at the top.
        assert_eq!(offset, 0);
    }

    #[rstest::rstest]
    fn offset_pins_to_zero_when_the_cursor_is_near_the_document_start() {
        // Given a document of 100 rows, a 20-row column, and a cursor at row 5.
        let document = DocumentLayout {
            spans: Vec::new(),
            total_rows: 100,
            cursor_row: Some(5),
        };

        // When resolving the offset.
        let offset = document.offset(20);

        // Then centering would scroll above the start, so it clamps to zero.
        assert_eq!(offset, 0);
    }

    #[rstest::rstest]
    fn offset_centers_the_cursor_when_the_document_has_slack() {
        // Given a document of 100 rows, a 20-row column, and a cursor at row 50.
        let document = DocumentLayout {
            spans: Vec::new(),
            total_rows: 100,
            cursor_row: Some(50),
        };

        // When resolving the offset.
        let offset = document.offset(20);

        // Then the cursor lands on the middle row of the window.
        assert_eq!(offset, 50 - 10);
        assert_eq!(offset + 10, 50);
    }

    #[rstest::rstest]
    fn offset_pins_to_the_end_when_the_cursor_is_near_the_document_end() {
        // Given a document of 100 rows, a 20-row column, and a cursor at row 95.
        let document = DocumentLayout {
            spans: Vec::new(),
            total_rows: 100,
            cursor_row: Some(95),
        };

        // When resolving the offset.
        let offset = document.offset(20);

        // Then the window shows the document's last row.
        assert_eq!(offset, 80);
        assert_eq!(offset + 19, 99);
    }

    #[rstest::rstest]
    fn bottom_slack_is_the_gap_when_the_document_fits() {
        // Given a document of 6 rows and a 40-row column.
        let document = DocumentLayout {
            spans: Vec::new(),
            total_rows: 6,
            cursor_row: None,
        };

        // When measuring the slack.
        let slack = document.bottom_slack(40);

        // Then it equals the unused rows below the document.
        assert_eq!(slack, 34);
    }

    #[rstest::rstest]
    fn bottom_slack_is_zero_once_the_document_overflows() {
        // Given a document of 100 rows and a 20-row column.
        let document = DocumentLayout {
            spans: Vec::new(),
            total_rows: 100,
            cursor_row: None,
        };

        // When measuring the slack.
        let slack = document.bottom_slack(20);

        // Then there is no slack — scrolling owns the overflow.
        assert_eq!(slack, 0);
    }

    #[rstest::rstest]
    fn sessions_height_counts_every_session() {
        // Given a state with 40 sessions.
        let state = state_with_sessions(40);

        // When measuring the sessions section.
        let rows = sessions_rows(&state);

        // Then all 40 entries plus the footer are counted.
        assert_eq!(rows, 41);
    }

    #[rstest::rstest]
    fn sessions_height_keeps_a_placeholder_row_when_empty() {
        // Given a state with no sessions.
        let mut state = AppState::default_with_scope_focus();
        let ids: Vec<_> = state.session.iter().map(|(id, _)| id.clone()).collect();
        for id in ids {
            state.session.remove(&id);
        }

        // When measuring the sessions section.
        let rows = sessions_rows(&state);

        // Then the placeholder and the footer are both present.
        assert_eq!(rows, 2);
    }

    #[rstest::rstest]
    fn a_span_scrolled_above_the_window_reports_no_visible_height() {
        // Given a span covering rows 0..4 with a window starting at row 10.
        let span = SectionSpan {
            id: SidebarSectionId::Pins,
            first_row: 0,
            rows: 4,
        };

        // When measuring the visible height.
        let height = span.visible_height(10, 20);

        // Then nothing of it is visible.
        assert_eq!(height, 0);
    }

    #[rstest::rstest]
    fn a_partially_scrolled_span_reports_its_visible_slice() {
        // Given a span covering rows 0..10 with a window starting at row 4.
        let span = SectionSpan {
            id: SidebarSectionId::Pins,
            first_row: 0,
            rows: 10,
        };

        // When measuring the visible height.
        let height = span.visible_height(4, 20);

        // Then only the last six rows fall inside the window.
        assert_eq!(height, 6);
    }
}
