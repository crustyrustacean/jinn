//! Vertical minimap - one block per chat entry showing token-count sizing.
//!
//! Renders colored blocks (`█`) representing chat entries in a single-column
//! display, one entry per row. The color indicates approximate token count.
//! Entries without token counts produce an empty row. Excluded entry types
//! (Actor, empty assistant) produce no row at all.
//! The viewport scrolls to keep the selected entry visible. A `>` arrow overlay
//! on the chat log area points at the selected entry's row.

use jinn_preferences_config::schemas::MinimapConfig;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use jinn_chat_log_view_msg::VisualItem;
use jinn_kernel::common::app_state::AppState;
use jinn_kernel::protocol::{ChatEntry, ChatEntryKind};

#[cfg(test)]
use jinn_chat_log_view_msg::{DEFAULT_MIN_COLLAPSE_COUNT, PROXIMITY_COUNT, build_visual_items};

/// Full block character for minimap entries.
const FULL_BLOCK: &str = "\u{2588}";

/// Number of color bands in the minimap gradient.
const MINIMAP_BANDS: usize = 8;

/// Colorblind-friendly palette - 8 colors ramping from perceptually dark to bright.
/// Order: smallest token count → largest token count.
/// Theme-independent: designed for high contrast on dark backgrounds.
const MINIMAP_PALETTE: [Color; MINIMAP_BANDS] = [
    Color::Rgb(39, 12, 77),    // band 0: deep indigo
    Color::Rgb(39, 12, 77),    // band 1: deep indigo
    Color::Rgb(100, 20, 108),  // band 2: violet
    Color::Rgb(156, 43, 99),   // band 3: magenta-rose
    Color::Rgb(208, 74, 67),   // band 4: warm red
    Color::Rgb(243, 125, 22),  // band 5: orange
    Color::Rgb(251, 197, 51),  // band 6: gold
    Color::Rgb(252, 255, 164), // band 7: pale yellow
];

/// Returns the color for a token-count block using linear banding.
///
/// Divides `[0, max_tokens]` into `MINIMAP_BANDS` equal-width bands.
/// Counts exceeding `max_tokens` get the last band color.
#[expect(clippy::expect_used, reason = "infallible")]
fn token_threshold_color(count: u32, max_tokens: u32) -> Color {
    if max_tokens == 0 {
        return MINIMAP_PALETTE[0];
    }
    let band = (u64::from(count) * MINIMAP_BANDS as u64 / u64::from(max_tokens))
        .min((MINIMAP_BANDS - 1) as u64) as usize;
    MINIMAP_PALETTE
        .get(band)
        .copied()
        .unwrap_or(*MINIMAP_PALETTE.first().expect("non-empty"))
}

/// Extension trait for determining whether a visual item should produce
/// a minimap block.
trait MinimapVisibility {
    fn is_minimap_visible(&self, history: &[ChatEntry]) -> bool;
}

impl MinimapVisibility for VisualItem {
    fn is_minimap_visible(&self, history: &[ChatEntry]) -> bool {
        match self {
            VisualItem::CollapsedIgnoredBlock { .. } => true,
            VisualItem::Entry(hist_idx) => {
                let Some(entry) = history.get(*hist_idx) else {
                    return false;
                };
                if entry.is_empty_assistant() {
                    return false;
                }
                !matches!(entry.kind, ChatEntryKind::Actor { .. })
            }
        }
    }
}

/// A visible entry in the minimap (non-excluded).
struct VisibleEntry {
    /// Visual-item index.
    vi_index: usize,
    /// Persisted token count, if computed.
    token_count: Option<u32>,
}

/// Computes the list of visible (non-excluded) entries from visual items.
#[expect(clippy::expect_used, reason = "infallible")]
fn compute_visible_entries(state: &AppState) -> Vec<VisibleEntry> {
    let session = state.active_session();
    let history = session.history();
    let items = session.visual_items_snapshot();

    items
        .iter()
        .enumerate()
        .filter_map(|(vi_idx, item)| {
            if !item.is_minimap_visible(history) {
                return None;
            }
            let ignored = match item {
                VisualItem::CollapsedIgnoredBlock { .. } => false,
                VisualItem::Entry(hist_idx) => {
                    let entry = history.get(*hist_idx).expect("hist_idx from visual_items");
                    !entry.is_in_context()
                }
            };
            let token_count = if ignored {
                None
            } else {
                match item {
                    VisualItem::CollapsedIgnoredBlock { .. } => None,
                    VisualItem::Entry(hist_idx) => {
                        let entry = history.get(*hist_idx).expect("hist_idx from visual_items");
                        entry.token_count
                    }
                }
            };
            Some(VisibleEntry {
                vi_index: vi_idx,
                token_count,
            })
        })
        .collect()
}

fn find_block_index(selected_vi_idx: Option<usize>, visible: &[VisibleEntry]) -> Option<usize> {
    match selected_vi_idx {
        Some(idx) => visible.iter().position(|e| e.vi_index == idx),
        None => visible.len().checked_sub(1),
    }
}
#[expect(
    clippy::allow_attributes,
    reason = "dead_code is a compiler lint, not clippy"
)]
#[allow(dead_code, reason = "available for future use")]
fn compute_minimap_scroll(
    selected_block: usize,
    _total_blocks: usize,
    viewport_height: usize,
) -> usize {
    let midpoint = viewport_height / 2;
    selected_block.saturating_sub(midpoint)
}

pub struct MinimapArrow {
    pub row: u16,
    pub token_count: Option<u32>,
    /// Sum of cached token counts for `is_in_context()` entries strictly
    /// before the cursor's occupied history range.
    pub tokens_above: Option<u32>,
    /// Sum of cached token counts for `is_in_context()` entries strictly
    /// after the cursor's occupied history range.
    pub tokens_below: Option<u32>,
}

pub fn render_vertical_minimap(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    muted_text_color: Color,
    config: &jinn_config::ConfigLayer,
) -> Option<MinimapArrow> {
    if state.session.is_loading() {
        return None;
    }

    let visible = compute_visible_entries(state);
    if visible.is_empty() {
        return None;
    }

    let total_blocks = visible.len();
    let viewport_height = area.height as usize;
    if viewport_height == 0 {
        return None;
    }

    let selected_idx = state.active_session().selected_entry_index();
    let selected_block = find_block_index(selected_idx, &visible)?;
    let midpoint = viewport_height / 2;

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(viewport_height);
    for row in 0..viewport_height {
        let block_index = selected_block as isize + row as isize - midpoint as isize;
        if block_index >= 0 && (block_index as usize) < total_blocks {
            let entry = visible.get(block_index as usize)?;
            let span = match entry.token_count {
                Some(count) => Span::styled(
                    FULL_BLOCK.to_owned(),
                    Style::default().fg(token_threshold_color(
                        count,
                        config.read::<MinimapConfig>().max_tokens,
                    )),
                ),
                None => Span::raw(" "),
            };
            lines.push(Line::from(span));
        } else {
            lines.push(Line::from(" "));
        }
    }

    let widget = Paragraph::new(lines);
    frame.render_widget(widget, area);

    render_scroll_arrows(
        frame,
        area,
        selected_block,
        total_blocks,
        viewport_height,
        muted_text_color,
    );

    let arrow_row = midpoint as u16;
    let selected_token_count = visible.get(selected_block).and_then(|e| e.token_count);
    let (cursor_start, cursor_end) = {
        let session = state.active_session();
        let items = session.visual_items_snapshot();
        let history_len = session.history().len();
        // When the user has not yet placed the cursor, fall back to the last
        // visual item so above/below indicators render immediately. This
        // mirrors `find_block_index`'s `None`-arm fallback.
        let effective_idx = selected_idx.or_else(|| items.len().checked_sub(1));
        cursor_history_range(&items, effective_idx, history_len)
    };
    let tokens_above = compute_tokens_above(state, cursor_start);
    let tokens_below = compute_tokens_below(state, cursor_end);

    Some(MinimapArrow {
        row: arrow_row,
        token_count: selected_token_count,
        tokens_above,
        tokens_below,
    })
}

/// Returns the cursor's occupied history range as `[start, end)`.
///
/// - For `Entry(hist_idx)` → `[hist_idx, hist_idx + 1)`.
/// - For `CollapsedIgnoredBlock { start, count }` → `[start, start + count)`.
/// - When no visual item is selected (or the index is out of range) →
///   `[0, history_len)` (full range; both above and below sums become empty).
fn cursor_history_range(
    items: &[VisualItem],
    selected_vi_idx: Option<usize>,
    history_len: usize,
) -> (usize, usize) {
    match selected_vi_idx {
        Some(idx) => match items.get(idx) {
            Some(VisualItem::Entry(hist_idx)) => (*hist_idx, hist_idx.saturating_add(1)),
            Some(VisualItem::CollapsedIgnoredBlock { start, count }) => {
                (*start, start.saturating_add(*count))
            }
            None => (0, history_len),
        },
        None => (0, history_len),
    }
}

/// Sums cached token counts for all `is_in_context()` entries in
/// history range `0..end` (exclusive upper bound). Returns `Some(0)`
/// when the range is empty or all entries are excluded.
fn compute_tokens_above(state: &AppState, end: usize) -> Option<u32> {
    compute_token_sum_in_range(state, 0, end)
}

/// Sums cached token counts for all `is_in_context()` entries in
/// history range `start..history.len()` (exclusive lower bound).
/// Returns `Some(0)` when the range is empty or all entries are excluded.
fn compute_tokens_below(state: &AppState, start: usize) -> Option<u32> {
    let history_len = state.active_session().history().len();
    compute_token_sum_in_range(state, start, history_len)
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "trait contract requires Result return"
)]
fn compute_token_sum_in_range(state: &AppState, start: usize, end: usize) -> Option<u32> {
    let session = state.active_session();
    let history = session.history();

    let end = end.min(history.len());
    let start = start.min(end);
    let mut sum: u32 = 0;
    for entry in history.get(start..end).unwrap_or(&[]) {
        if entry.is_in_context()
            && let Some(count) = entry.token_count
        {
            sum = sum.saturating_add(count);
        }
    }
    Some(sum)
}

fn render_scroll_arrows(
    frame: &mut Frame<'_>,
    area: Rect,
    selected_block: usize,
    total_blocks: usize,
    viewport_height: usize,
    muted_text_color: Color,
) {
    let midpoint = viewport_height / 2;
    let has_above = selected_block > midpoint;
    let has_below = selected_block + (viewport_height - midpoint) < total_blocks;

    if has_above {
        let arrow_area = Rect {
            x: area.x,
            y: area.y,
            width: 1,
            height: 1,
        };
        let arrow = Paragraph::new(Line::from(Span::styled(
            "▲",
            Style::default().fg(muted_text_color),
        )));
        frame.render_widget(arrow, arrow_area);
    }

    if has_below {
        let bottom_y = area.y + area.height.saturating_sub(1);
        let arrow_area = Rect {
            x: area.x,
            y: bottom_y,
            width: 1,
            height: 1,
        };
        let arrow = Paragraph::new(Line::from(Span::styled(
            "▼",
            Style::default().fg(muted_text_color),
        )));
        frame.render_widget(arrow, arrow_area);
    }
}

pub fn render_minimap_arrow(
    frame: &mut Frame<'_>,
    chat_log_area: Rect,
    arrow: &MinimapArrow,
    arrow_color: Color,
) {
    if chat_log_area.width == 0 || chat_log_area.height == 0 {
        return;
    }

    let y = chat_log_area.y + arrow.row.min(chat_log_area.height.saturating_sub(1));

    #[expect(
        clippy::single_match_else,
        reason = "different match arms produce different widget layouts"
    )]
    match arrow.token_count {
        Some(count) => {
            let formatted = format_entry_tokens(count);
            let text = format!("{formatted} >");
            let width = text.len() as u16;
            let x = chat_log_area
                .x
                .saturating_add(chat_log_area.width)
                .saturating_sub(width);
            let paragraph = Paragraph::new(Line::from(Span::styled(
                text,
                Style::default().fg(arrow_color),
            )));
            let arrow_area = Rect {
                x,
                y,
                width: width.min(chat_log_area.width),
                height: 1,
            };
            frame.render_widget(paragraph, arrow_area);
        }
        None => {
            let x = chat_log_area.x + chat_log_area.width.saturating_sub(1);
            let paragraph = Paragraph::new(Line::from(Span::styled(
                ">",
                Style::default().fg(arrow_color),
            )));
            let arrow_area = Rect {
                x,
                y,
                width: 1,
                height: 1,
            };
            frame.render_widget(paragraph, arrow_area);
        }
    }

    // Render ▲ token-count line above the arrow (strict-above: cursor excluded).
    if arrow.row > 0 {
        let row = arrow.row - 1;
        let text = match arrow.tokens_above {
            Some(n) => format!("{} ▲", format_entry_tokens(n)),
            _ => "▲".to_owned(),
        };
        let width = text.as_str().width() as u16;
        let x = chat_log_area
            .x
            .saturating_add(chat_log_area.width)
            .saturating_sub(width);
        let area = Rect {
            x,
            y: chat_log_area.y + row,
            width: width.min(chat_log_area.width),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                text,
                Style::default().fg(arrow_color),
            ))),
            area,
        );
    }

    // Render ▼ token-count line below the arrow (strict-below: cursor excluded).
    {
        let row = arrow.row + 1;
        if row < chat_log_area.height {
            let text = match arrow.tokens_below {
                Some(n) => format!("{} ▼", format_entry_tokens(n)),
                _ => "▼".to_owned(),
            };
            let width = text.as_str().width() as u16;
            let x = chat_log_area
                .x
                .saturating_add(chat_log_area.width)
                .saturating_sub(width);
            let area = Rect {
                x,
                y: chat_log_area.y + row,
                width: width.min(chat_log_area.width),
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    text,
                    Style::default().fg(arrow_color),
                ))),
                area,
            );
        }
    }
}

fn format_entry_tokens(count: u32) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", f64::from(count) / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", f64::from(count) / 1_000.0)
    } else {
        count.to_string()
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
    use super::*;
    use jinn_kernel::common::app_state::AppState;
    use jinn_kernel::protocol::ChatEntry;
    use jinn_theme::default_theme;

    /// A visible-entry list whose token counts are all unmeasured — the
    /// fixture used by the block-index and scroll-position tests.
    fn uncounted_visible(vi_indices: &[usize]) -> Vec<VisibleEntry> {
        vi_indices
            .iter()
            .map(|&vi_index| VisibleEntry {
                vi_index,
                token_count: None,
            })
            .collect()
    }

    #[rstest::rstest]
    fn find_block_index_returns_position_for_existing_entry() {
        // Given three visible entries at visual indices 0, 2, and 5.
        let visible = uncounted_visible(&[0, 2, 5]);

        // When locating the block for the visual index 2.
        let block = find_block_index(Some(2), &visible);

        // Then the block is the second one.
        assert_eq!(block, Some(1));
    }

    #[rstest::rstest]
    fn find_block_index_returns_none_for_excluded_entry() {
        // Given visible entries at visual indices 0 and 2.
        let visible = uncounted_visible(&[0, 2]);

        // When locating the block for the excluded visual index 1.
        let block = find_block_index(Some(1), &visible);

        // Then there is no block for it.
        assert!(block.is_none());
    }

    #[rstest::rstest]
    fn find_block_index_returns_last_when_none() {
        // Given two visible entries at visual indices 0 and 2.
        let visible = uncounted_visible(&[0, 2]);

        // When locating the block with no selected visual index.
        let block = find_block_index(None, &visible);

        // Then the last block is returned.
        assert_eq!(block, Some(1));
    }

    #[rstest::rstest]
    fn find_block_index_returns_none_for_empty() {
        // Given an empty visible-entry list.
        let visible: Vec<VisibleEntry> = vec![];

        // When locating the block for visual index 0.
        let block = find_block_index(Some(0), &visible);

        // Then there is no block.
        assert!(block.is_none());
    }

    #[rstest::rstest]
    fn scroll_is_midpoint_based() {
        // Given a 10-row viewport (midpoint 5) and a selection at block 4.
        let viewport_height = 10;
        let selected_block = 4;

        // When computing the scroll offset.
        let offset = compute_minimap_scroll(selected_block, 5, viewport_height);

        // Then the offset is zero because the selection sits above the midpoint.
        assert_eq!(offset, 0);
    }

    #[rstest::rstest]
    fn scroll_centers_selected() {
        // Given a 10-row viewport (midpoint 5) and a selection at block 45.
        let viewport_height = 10;
        let selected_block = 45;

        // When computing the scroll offset.
        let offset = compute_minimap_scroll(selected_block, 50, viewport_height);

        // Then the selected block lands on the midpoint.
        assert_eq!(offset, 40);
    }

    #[rstest::rstest]
    fn scroll_at_start_is_zero() {
        // Given a 10-row viewport (midpoint 5) and a selection at block 0.
        let viewport_height = 10;
        let selected_block = 0;

        // When computing the scroll offset.
        let offset = compute_minimap_scroll(selected_block, 50, viewport_height);

        // Then the offset is zero.
        assert_eq!(offset, 0);
    }

    #[rstest::rstest]
    fn scroll_at_last_block() {
        // Given a 10-row viewport (midpoint 5) and a selection on the last block.
        let viewport_height = 10;
        let selected_block = 49;

        // When computing the scroll offset.
        let offset = compute_minimap_scroll(selected_block, 50, viewport_height);

        // Then the offset places the last block on the midpoint.
        assert_eq!(offset, 44);
    }

    #[rstest::rstest]
    fn scroll_near_midpoint() {
        // Given a 10-row viewport (midpoint 5) and a selection at block 5.
        let viewport_height = 10;
        let selected_block = 5;

        // When computing the scroll offset.
        let offset = compute_minimap_scroll(selected_block, 50, viewport_height);

        // Then the offset is zero.
        assert_eq!(offset, 0);
    }

    /// A layer holding no config, for tests that do not exercise the
    /// token-threshold colour.
    fn empty_layer() -> jinn_config::ConfigLayer {
        jinn_config::ConfigLayer::load(std::sync::Arc::new(
            jinn_config::InMemoryConfigStorage::default(),
        ))
        .expect("layer loads")
    }

    fn render_to_buffer(
        state: &AppState,
        width: u16,
        height: u16,
    ) -> (Option<MinimapArrow>, Vec<String>) {
        setup_visual_items(state);
        let (mut terminal, area) = jinn_testutil::setup_term(width, height);
        let theme = default_theme();
        let config = empty_layer();
        let mut arrow_result = None;
        terminal
            .draw(|frame| {
                arrow_result =
                    render_vertical_minimap(frame, area, state, theme.muted_text, &config);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rows = jinn_testutil::buffer_rows(buffer, width, height);
        (arrow_result, rows)
    }

    #[rstest::rstest]
    fn empty_history_renders_nothing() {
        // Given a session with no entries.
        let state = AppState::default();

        // When rendering the minimap.
        let (arrow, rows) = render_to_buffer(&state, 1, 10);

        // Then no arrow is produced.
        assert!(arrow.is_none());
        // And the minimap column is blank.
        assert!(rows[0].trim().is_empty());
    }

    #[rstest::rstest]
    fn single_entry_no_cache_shows_space_at_midpoint() {
        // Given one user entry with no cached token count.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));

        // When rendering the minimap.
        let (arrow, rows) = render_to_buffer(&state, 1, 10);

        // Then an arrow is still produced.
        assert!(arrow.is_some());
        // And the midpoint row holds no block.
        assert!(
            !rows[5].contains('\u{2588}'),
            "no block without token count"
        );
    }

    #[rstest::rstest]
    fn single_entry_with_count_shows_block_at_midpoint() {
        // Given one user entry whose cached token count is 50.
        let mut state = AppState::default();
        let mut entry = ChatEntry::user("hello world");
        entry.token_count = Some(50);
        state.active_session_mut().push_entry(entry);

        // When rendering the minimap.
        let (arrow, rows) = render_to_buffer(&state, 1, 10);

        // Then an arrow is produced.
        assert!(arrow.is_some());
        // And a block is drawn on the midpoint row.
        assert!(rows[5].contains('\u{2588}'), "expected block at midpoint");
    }

    #[rstest::rstest]
    fn arrow_at_midpoint_when_last_entry_selected() {
        // Given three entries with no cursor placed.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant("b"));
        state.active_session_mut().push_entry(ChatEntry::user("c"));

        // When rendering the minimap.
        let (arrow, _) = render_to_buffer(&state, 1, 10);

        // Then the arrow sits on the midpoint row.
        assert_eq!(arrow.expect("arrow exists").row, 5);
    }

    #[rstest::rstest]
    fn excluded_entries_produce_no_blocks() {
        // Given a history mixing an actor entry, a thinking entry, and users.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::actor("bash", "output"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::thinking("reasoning"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant("b"));

        // When rendering the minimap.
        let (arrow, rows) = render_to_buffer(&state, 1, 10);

        // Then no block is drawn for the excluded entry types.
        assert_eq!(rows.iter().filter(|r| r.contains('\u{2588}')).count(), 0);
        // And the arrow still lands on the midpoint row.
        assert_eq!(arrow.expect("arrow").row, 5);
    }

    #[rstest::rstest]
    fn arrow_clamps_to_viewport_height() {
        // Given 20 entries in a 5-row minimap viewport.
        let mut state = AppState::default();
        for i in 0..20 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user(format!("msg {i}")));
        }

        // When rendering the minimap.
        let (arrow, _) = render_to_buffer(&state, 1, 5);

        // Then the arrow is clamped to the midpoint of the short viewport.
        assert_eq!(arrow.expect("arrow").row, 2);
    }

    #[rstest::rstest]
    fn arrow_renders_greater_than_character() {
        // Given an arrow on row 3 with no token count.
        let (mut terminal, area) = jinn_testutil::setup_term(40, 10);
        let arrow = MinimapArrow {
            row: 3,
            token_count: None,
            tokens_above: None,
            tokens_below: None,
        };
        let theme = default_theme();

        // When rendering the arrow.
        terminal
            .draw(|frame| {
                render_minimap_arrow(frame, area, &arrow, theme.border_unfocused);
            })
            .unwrap();

        // Then a `>` glyph is drawn on the arrow's row.
        let rows = jinn_testutil::buffer_rows(terminal.backend().buffer(), 40, 10);
        assert!(rows[3].contains('>'));
    }

    #[rstest::rstest]
    fn scroll_down_arrow_at_bottom() {
        // Given 20 entries with the cursor on the first one.
        let mut state = AppState::default();
        for i in 0..20 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user(format!("msg {i}")));
        }
        state.active_session_mut().set_selected_entry_index(0);

        // When rendering the minimap.
        let (_, rows) = render_to_buffer(&state, 1, 5);

        // Then the scroll-down arrow is drawn on the bottom row.
        assert!(rows[4].contains('▼'));
    }

    #[rstest::rstest]
    fn scroll_up_arrow_at_top() {
        // Given 20 entries with no cursor placed, so the last one is selected.
        let mut state = AppState::default();
        for i in 0..20 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user(format!("msg {i}")));
        }

        // When rendering the minimap.
        let (_, rows) = render_to_buffer(&state, 1, 5);

        // Then the scroll-up arrow is drawn on the top row.
        assert!(rows[0].contains('▲'));
    }

    #[rstest::rstest]
    fn no_arrows_when_all_entries_fit() {
        // Given three entries that all fit the 10-row minimap viewport.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant("b"));
        state.active_session_mut().push_entry(ChatEntry::user("c"));

        // When rendering the minimap.
        let (_, rows) = render_to_buffer(&state, 1, 10);

        // Then no scroll-up arrow is drawn.
        assert!(!rows.iter().any(|r| r.contains('▲')));
        // And no scroll-down arrow is drawn.
        assert!(!rows.iter().any(|r| r.contains('▼')));
    }

    #[rstest::rstest]
    fn empty_assistant_entry_produces_no_block() {
        // Given a single empty assistant entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant(""));

        // When rendering the minimap.
        let (arrow, rows) = render_to_buffer(&state, 1, 10);

        // Then no arrow is produced.
        assert!(arrow.is_none());
        // And no block is drawn.
        assert_eq!(rows.iter().filter(|r| r.contains('\u{2588}')).count(), 0);
    }

    fn setup_visual_items(state: &AppState) {
        let session = state.active_session();
        let shown_ignored_blocks = session.shown_ignored_blocks_snapshot();
        let items = build_visual_items(
            session.history(),
            &shown_ignored_blocks,
            PROXIMITY_COUNT,
            DEFAULT_MIN_COLLAPSE_COUNT,
        );
        state.active_session().set_visual_items(items);
    }

    #[rstest::rstest]
    fn in_context_entry_with_count_shows_block() {
        // Given one in-context entry with a cached token count of 500.
        let mut state = AppState::default();
        let mut entry = ChatEntry::user("hello world this is a test");
        entry.token_count = Some(500);
        state.active_session_mut().push_entry(entry);

        // When rendering the minimap.
        let (_, rows) = render_to_buffer(&state, 1, 10);

        // Then exactly one block is drawn on the midpoint row.
        assert_eq!(rows[5].chars().filter(|&c| c == '\u{2588}').count(), 1);
    }

    #[rstest::rstest]
    fn entry_without_count_shows_space() {
        // Given one in-context entry with no cached token count.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));

        // When rendering the minimap.
        let (_, rows) = render_to_buffer(&state, 1, 10);

        // Then the midpoint row holds a space rather than a block.
        assert_eq!(rows[5].chars().filter(|&c| c == '\u{2588}').count(), 0);
    }

    #[rstest::rstest]
    fn token_threshold_band_0_is_blue() {
        // Given a max token count of 2000, the low band is [0, 250).

        // When colouring counts at both ends of band 0.
        let low = token_threshold_color(0, 2000);
        let high = token_threshold_color(249, 2000);

        // Then both are the deep-indigo band 0 colour.
        assert_eq!(low, Color::Rgb(39, 12, 77));
        // And the upper edge still belongs to band 0.
        assert_eq!(high, Color::Rgb(39, 12, 77));
    }

    #[rstest::rstest]
    fn token_threshold_band_1_is_cyan() {
        // Given a max token count of 2000, the second band is [250, 500).

        // When colouring counts at both ends of band 1.
        let low = token_threshold_color(250, 2000);
        let high = token_threshold_color(499, 2000);

        // Then both are the deep-indigo band 1 colour.
        assert_eq!(low, Color::Rgb(39, 12, 77));
        // And the upper edge still belongs to band 1.
        assert_eq!(high, Color::Rgb(39, 12, 77));
    }

    #[rstest::rstest]
    fn token_threshold_band_2_is_green() {
        // Given a max token count of 2000, the third band is [500, 750).

        // When colouring counts at both ends of band 2.
        let low = token_threshold_color(500, 2000);
        let high = token_threshold_color(749, 2000);

        // Then both are the violet band 2 colour.
        assert_eq!(low, Color::Rgb(100, 20, 108));
        // And the upper edge still belongs to band 2.
        assert_eq!(high, Color::Rgb(100, 20, 108));
    }

    #[rstest::rstest]
    fn token_threshold_band_3_is_yellow_green() {
        // Given a max token count of 2000, the fourth band is [750, 1000).

        // When colouring counts at both ends of band 3.
        let low = token_threshold_color(750, 2000);
        let high = token_threshold_color(999, 2000);

        // Then both are the magenta-rose band 3 colour.
        assert_eq!(low, Color::Rgb(156, 43, 99));
        // And the upper edge still belongs to band 3.
        assert_eq!(high, Color::Rgb(156, 43, 99));
    }

    #[rstest::rstest]
    fn token_threshold_band_4_is_gold() {
        // Given a max token count of 2000, the fifth band is [1000, 1250).

        // When colouring counts at both ends of band 4.
        let low = token_threshold_color(1000, 2000);
        let high = token_threshold_color(1249, 2000);

        // Then both are the warm-red band 4 colour.
        assert_eq!(low, Color::Rgb(208, 74, 67));
        // And the upper edge still belongs to band 4.
        assert_eq!(high, Color::Rgb(208, 74, 67));
    }

    #[rstest::rstest]
    fn token_threshold_band_5_is_red_orange() {
        // Given a max token count of 2000, the sixth band is [1250, 1500).

        // When colouring counts at both ends of band 5.
        let low = token_threshold_color(1250, 2000);
        let high = token_threshold_color(1499, 2000);

        // Then both are the orange band 5 colour.
        assert_eq!(low, Color::Rgb(243, 125, 22));
        // And the upper edge still belongs to band 5.
        assert_eq!(high, Color::Rgb(243, 125, 22));
    }

    #[rstest::rstest]
    fn token_threshold_band_6_is_dark_red() {
        // Given a max token count of 2000, the seventh band is [1500, 1750).

        // When colouring counts at both ends of band 6.
        let low = token_threshold_color(1500, 2000);
        let high = token_threshold_color(1749, 2000);

        // Then both are the gold band 6 colour.
        assert_eq!(low, Color::Rgb(251, 197, 51));
        // And the upper edge still belongs to band 6.
        assert_eq!(high, Color::Rgb(251, 197, 51));
    }

    #[rstest::rstest]
    fn token_threshold_band_7_is_crimson() {
        // Given a max token count of 2000, the top band is [1750, ∞).

        // When colouring counts at the band start, at the max, and past it.
        let low = token_threshold_color(1750, 2000);
        let at_max = token_threshold_color(2000, 2000);
        let high = token_threshold_color(9999, 2000);

        // Then the band start is the pale-yellow top colour.
        assert_eq!(low, Color::Rgb(252, 255, 164));
        // And the max itself still belongs to that band.
        assert_eq!(at_max, Color::Rgb(252, 255, 164));
        // And a count past the max saturates to that colour.
        assert_eq!(high, Color::Rgb(252, 255, 164));
    }

    #[rstest::rstest]
    fn token_threshold_custom_max_tokens_adjusts_bands() {
        // Given a max token count of 1000, each band is 125 tokens wide.

        // When colouring counts across the narrowed bands.
        let first = token_threshold_color(0, 1000);
        let last_of_first_band = token_threshold_color(124, 1000);
        let first_band_top = token_threshold_color(125, 1000);
        let last_band_top = token_threshold_color(999, 1000);

        // Then the first band keeps the palette's opening colour.
        assert_eq!(first, Color::Rgb(39, 12, 77));
        // And the count just below the band boundary does too.
        assert_eq!(last_of_first_band, Color::Rgb(39, 12, 77));
        // And the boundary itself is still that same opening colour.
        assert_eq!(first_band_top, Color::Rgb(39, 12, 77));
        // And the top of the range is the pale-yellow final colour.
        assert_eq!(last_band_top, Color::Rgb(252, 255, 164));
    }

    #[rstest::rstest]
    fn token_threshold_zero_max_returns_first_band() {
        // Given a max token count of zero.

        // When colouring a count of 100.
        let color = token_threshold_color(100, 0);

        // Then the first palette colour is returned.
        assert_eq!(color, MINIMAP_PALETTE[0]);
    }

    #[rstest::rstest]
    fn format_entry_tokens_small() {
        // Given a count below one thousand.

        // When formatting it.
        let text = format_entry_tokens(42);

        // Then it is rendered verbatim.
        assert_eq!(text, "42");
    }

    #[rstest::rstest]
    fn format_entry_tokens_k() {
        // Given counts of one thousand and forty-two thousand five hundred.

        // When formatting them.
        let one_k = format_entry_tokens(1_000);
        let forty_two_k = format_entry_tokens(42_500);

        // Then one thousand is rendered as `1.0k`.
        assert_eq!(one_k, "1.0k");
        // And the larger count is rendered as `42.5k`.
        assert_eq!(forty_two_k, "42.5k");
    }

    #[rstest::rstest]
    fn format_entry_tokens_m() {
        // Given a count of one million.

        // When formatting it.
        let text = format_entry_tokens(1_000_000);

        // Then it is rendered with an `M` suffix.
        assert_eq!(text, "1.0M");
    }

    #[rstest::rstest]
    fn arrow_with_token_count_renders_formatted_count() {
        // Given an arrow on row 3 carrying a token count of 3000.
        let (mut terminal, area) = jinn_testutil::setup_term(40, 10);
        let arrow = MinimapArrow {
            row: 3,
            token_count: Some(3000),
            tokens_above: None,
            tokens_below: None,
        };
        let theme = default_theme();

        // When rendering the arrow.
        terminal
            .draw(|frame| {
                render_minimap_arrow(frame, area, &arrow, theme.border_unfocused);
            })
            .unwrap();

        // Then the arrow's row shows the formatted count.
        let rows = jinn_testutil::buffer_rows(terminal.backend().buffer(), 40, 10);
        assert!(rows[3].contains('3'));
        // And the row also shows the arrow glyph.
        assert!(rows[3].contains('>'));
    }

    #[rstest::rstest]
    fn arrow_without_token_count_renders_just_gt() {
        // Given an arrow on row 3 with no token count.
        let (mut terminal, area) = jinn_testutil::setup_term(40, 10);
        let arrow = MinimapArrow {
            row: 3,
            token_count: None,
            tokens_above: None,
            tokens_below: None,
        };
        let theme = default_theme();

        // When rendering the arrow.
        terminal
            .draw(|frame| {
                render_minimap_arrow(frame, area, &arrow, theme.border_unfocused);
            })
            .unwrap();

        // Then the arrow's row shows the `>` glyph.
        let rows = jinn_testutil::buffer_rows(terminal.backend().buffer(), 40, 10);
        assert!(rows[3].contains('>'));
        // And no count suffix is drawn.
        assert!(!rows[3].contains('k'));
    }

    // ── Strict-above / strict-below computation tests ──

    /// A session whose history carries the given token counts in order,
    /// paired with visual items built from that history.
    fn state_with_counts(counts: &[u32]) -> AppState {
        let mut state = AppState::default();
        for (i, &count) in counts.iter().enumerate() {
            let mut entry = ChatEntry::user(format!("msg {i}"));
            entry.token_count = Some(count);
            state.active_session_mut().push_entry(entry);
        }
        setup_visual_items(&state);
        state
    }

    /// The `Above` / `Below` pair rendered for a given visual-item cursor.
    fn arrow_counts(state: &AppState, cursor_vi: Option<usize>) -> (Option<u32>, Option<u32>) {
        let items = state.active_session().visual_items_snapshot();
        let history_len = state.active_session().history().len();
        let (start, end) = cursor_history_range(&items, cursor_vi, history_len);
        (
            compute_tokens_above(state, start),
            compute_tokens_below(state, end),
        )
    }

    #[rstest::rstest]
    fn tokens_above_and_below_for_single_entry() {
        // Given one in-context entry with a cached count of 100.
        let state = state_with_counts(&[100]);

        // When summing the counts above and below the selected entry.
        let (above, below) = arrow_counts(&state, Some(0));

        // Then the range above is empty.
        assert_eq!(above, Some(0));
        // And the range below is empty.
        assert_eq!(below, Some(0));
    }

    #[rstest::rstest]
    fn tokens_above_and_below_skip_excluded_entries() {
        // Given a thinking entry (excluded, 7 tokens) followed by a user
        // entry (in context, 200 tokens) that is selected.
        let mut state = AppState::default();
        let mut thinking = ChatEntry::thinking("reasoning");
        thinking.token_count = Some(7);
        let mut user = ChatEntry::user("hello");
        user.token_count = Some(200);
        state.active_session_mut().push_entry(thinking);
        state.active_session_mut().push_entry(user);
        setup_visual_items(&state);

        // When selecting the selected user entry and summing around it.
        let last_vi = state.active_session().visual_items_snapshot().len() - 1;
        let (above, below) = arrow_counts(&state, Some(last_vi));

        // Then nothing counts above it, because the thinking entry is out of context.
        assert_eq!(above, Some(0));
        // And nothing counts below it.
        assert_eq!(below, Some(0));
    }

    #[rstest::rstest]
    fn tokens_above_sums_entries_before_the_cursor() {
        // Given three in-context entries cached at 100, 200, and 300 tokens.
        let state = state_with_counts(&[100, 200, 300]);

        // When summing the counts above a cursor on the last entry.
        let last_vi = state.active_session().visual_items_snapshot().len() - 1;
        let (above, _) = arrow_counts(&state, Some(last_vi));

        // Then the sum is the two entries before the cursor.
        assert_eq!(above, Some(300));
    }

    #[rstest::rstest]
    fn tokens_below_sums_entries_after_the_cursor() {
        // Given three in-context entries cached at 100, 200, and 300 tokens.
        let state = state_with_counts(&[100, 200, 300]);
        let mid_vi = state
            .active_session()
            .visual_items_snapshot()
            .iter()
            .position(|i| matches!(i, VisualItem::Entry(1)))
            .expect("history idx 1 must be a visual item");

        // When summing the counts below a cursor on the middle entry.
        let (_, below) = arrow_counts(&state, Some(mid_vi));

        // Then the sum is the single entry after the cursor.
        assert_eq!(below, Some(300));
    }

    #[rstest::rstest]
    fn tokens_above_excludes_the_cursor_entry() {
        // Given three in-context entries cached at 100, 200, and 300 tokens.
        let state = state_with_counts(&[100, 200, 300]);
        let mid_vi = state
            .active_session()
            .visual_items_snapshot()
            .iter()
            .position(|i| matches!(i, VisualItem::Entry(1)))
            .expect("history idx 1 must be a visual item");

        // When summing the counts above a cursor on the middle entry.
        let (above, _) = arrow_counts(&state, Some(mid_vi));

        // Then only the entry before the cursor contributes.
        assert_eq!(above, Some(100));
    }

    #[rstest::rstest]
    fn tokens_below_is_zero_at_the_last_entry() {
        // Given three in-context entries cached at 100, 200, and 300 tokens.
        let state = state_with_counts(&[100, 200, 300]);
        let last_vi = state.active_session().visual_items_snapshot().len() - 1;

        // When summing the counts below a cursor on the last entry.
        let (_, below) = arrow_counts(&state, Some(last_vi));

        // Then the range below the cursor is empty.
        assert_eq!(below, Some(0));
    }

    #[rstest::rstest]
    fn tokens_above_and_below_are_zero_when_no_counts_computed() {
        // Given two in-context entries with no cached token counts.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant("world"));
        setup_visual_items(&state);

        // When summing the counts around the last entry.
        let last_vi = state.active_session().visual_items_snapshot().len() - 1;
        let (above, below) = arrow_counts(&state, Some(last_vi));

        // Then the sum above is zero.
        assert_eq!(above, Some(0));
        // And the sum below is zero.
        assert_eq!(below, Some(0));
    }

    #[rstest::rstest]
    fn collapsed_ignored_block_cursor_spans_the_whole_block() {
        // Given six entries [100, 50, 60, 70, 200, 300] rendered as entry 0,
        // a collapsed block covering 1..4, then entries 4 and 5.
        let mut state = AppState::default();
        let entries: Vec<ChatEntry> = vec![
            ChatEntry::user("first"),
            ChatEntry::thinking("t1"),
            ChatEntry::thinking("t2"),
            ChatEntry::thinking("t3"),
            ChatEntry::user("second"),
            ChatEntry::user("third"),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, mut entry)| {
            entry.token_count = Some([100u32, 50, 60, 70, 200, 300][i]);
            entry
        })
        .collect();
        for entry in entries {
            state.active_session_mut().push_entry(entry);
        }
        let items = vec![
            VisualItem::Entry(0),
            VisualItem::CollapsedIgnoredBlock { start: 1, count: 3 },
            VisualItem::Entry(4),
            VisualItem::Entry(5),
        ];
        state.active_session().set_visual_items(items);

        // When resolving the history range occupied by the collapsed block.
        let range = state.active_session().visual_items_snapshot();
        let history_len = state.active_session().history().len();
        let (start, end) = cursor_history_range(&range, Some(1), history_len);

        // Then the range covers the block from its first entry.
        assert_eq!(start, 1);
        // And up to the entry after its last.
        assert_eq!(end, 4);
    }

    #[rstest::rstest]
    fn collapsed_ignored_block_cursor_excludes_entire_block() {
        // Given six entries [100, 50, 60, 70, 200, 300] rendered as entry 0,
        // a collapsed block covering 1..4, then entries 4 and 5.
        let mut state = AppState::default();
        let entries: Vec<ChatEntry> = vec![
            ChatEntry::user("first"),
            ChatEntry::thinking("t1"),
            ChatEntry::thinking("t2"),
            ChatEntry::thinking("t3"),
            ChatEntry::user("second"),
            ChatEntry::user("third"),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, mut entry)| {
            entry.token_count = Some([100u32, 50, 60, 70, 200, 300][i]);
            entry
        })
        .collect();
        for entry in entries {
            state.active_session_mut().push_entry(entry);
        }
        let items = vec![
            VisualItem::Entry(0),
            VisualItem::CollapsedIgnoredBlock { start: 1, count: 3 },
            VisualItem::Entry(4),
            VisualItem::Entry(5),
        ];
        state.active_session().set_visual_items(items);

        // When selecting the collapsed block and summing the counts around it.
        let (above, below) = arrow_counts(&state, Some(1));

        // Then the above sum is only entry 0's 100 — the block is excluded.
        assert_eq!(above, Some(100));
        // And the below sum is entries 4 and 5 — the block is excluded.
        assert_eq!(below, Some(500));
    }

    // ── Above/below rendering tests ──

    #[rstest::rstest]
    fn above_and_below_lines_flank_arrow() {
        // Given an arrow on row 3 with token counts on both sides.
        let (mut terminal, area) = jinn_testutil::setup_term(40, 10);
        let arrow = MinimapArrow {
            row: 3,
            token_count: Some(1000),
            tokens_above: Some(6000),
            tokens_below: Some(1500),
        };
        let theme = default_theme();

        // When rendering the arrow.
        terminal
            .draw(|frame| {
                render_minimap_arrow(frame, area, &arrow, theme.border_unfocused);
            })
            .unwrap();

        // Then the arrow's own row carries the cursor's tokens.
        let rows = jinn_testutil::buffer_rows(terminal.backend().buffer(), 40, 10);
        assert!(rows[3].contains('1'));
        // And the row also carries the arrow glyph.
        assert!(rows[3].contains('>'));
        // And the row above carries the above-count and the ▲ glyph.
        assert!(rows[2].contains('6'));
        assert!(rows[2].contains('▲'));
        // And no arrow glyph leaks onto the row above.
        assert!(!rows[2].contains('>'));
        // And the row below carries the below-count and the ▼ glyph.
        assert!(rows[4].contains('1'));
        assert!(rows[4].contains('▼'));
        // And no arrow glyph leaks onto the row below.
        assert!(!rows[4].contains('>'));
    }

    #[rstest::rstest]
    fn below_line_skipped_when_arrow_at_last_row_above_still_renders() {
        // Given an arrow on the last row (9) with counts on both sides.
        let (mut terminal, area) = jinn_testutil::setup_term(40, 10);
        let arrow = MinimapArrow {
            row: 9,
            token_count: Some(100),
            tokens_above: Some(500),
            tokens_below: Some(999),
        };
        let theme = default_theme();

        // When rendering the arrow.
        terminal
            .draw(|frame| {
                render_minimap_arrow(frame, area, &arrow, theme.border_unfocused);
            })
            .unwrap();

        // Then the arrow's row carries the arrow glyph.
        let rows = jinn_testutil::buffer_rows(terminal.backend().buffer(), 40, 10);
        assert!(rows[9].contains('>'));
        // And the ▼ line does not fit, so it is skipped.
        assert!(!rows[9].contains('▼'));
        // And the row above still carries the ▲ glyph.
        assert!(rows[8].contains('▲'));
    }

    #[rstest::rstest]
    fn above_line_skipped_when_arrow_at_row_zero() {
        // Given an arrow on row 0 with counts on both sides.
        let (mut terminal, area) = jinn_testutil::setup_term(40, 10);
        let arrow = MinimapArrow {
            row: 0,
            token_count: Some(100),
            tokens_above: Some(500),
            tokens_below: Some(999),
        };
        let theme = default_theme();

        // When rendering the arrow.
        terminal
            .draw(|frame| {
                render_minimap_arrow(frame, area, &arrow, theme.border_unfocused);
            })
            .unwrap();

        // Then the arrow's row carries the arrow glyph.
        let rows = jinn_testutil::buffer_rows(terminal.backend().buffer(), 40, 10);
        assert!(rows[0].contains('>'));
        // And the ▲ line does not fit, so it is skipped.
        assert!(!rows[0].contains('▲'));
        // And the row below still carries the ▼ glyph.
        assert!(rows[1].contains('▼'));
    }

    #[rstest::rstest]
    fn glyph_alone_rendered_when_no_cached_counts() {
        // Given an arrow on row 3 with no cached counts above or below.
        let (mut terminal, area) = jinn_testutil::setup_term(40, 10);
        let arrow = MinimapArrow {
            row: 3,
            token_count: None,
            tokens_above: None,
            tokens_below: None,
        };
        let theme = default_theme();

        // When rendering the arrow.
        terminal
            .draw(|frame| {
                render_minimap_arrow(frame, area, &arrow, theme.border_unfocused);
            })
            .unwrap();

        // Then the row above shows the ▲ glyph alone.
        let rows = jinn_testutil::buffer_rows(terminal.backend().buffer(), 40, 10);
        assert!(rows[2].contains('▲'));
        // And no digit accompanies it.
        assert!(!rows[2].chars().any(|c| c.is_ascii_digit()));
        // And the row below shows the ▼ glyph alone.
        assert!(rows[4].contains('▼'));
        // And no digit accompanies it.
        assert!(!rows[4].chars().any(|c| c.is_ascii_digit()));
    }

    /// Fix 1: `Some(0)` should render as `"0 ▲"` / `"0 ▼"`, not glyph-alone.
    /// Glyph-alone is reserved for `None` (no cached counts at all).
    #[rstest::rstest]
    fn zero_count_renders_with_digit() {
        // Given an arrow on row 3 whose above/below counts are zero.
        let arrow = MinimapArrow {
            row: 3,
            token_count: None,
            tokens_above: Some(0),
            tokens_below: Some(0),
        };
        let (mut terminal, area) = jinn_testutil::setup_term(40, 10);
        let theme = default_theme();

        // When rendering the arrow.
        terminal
            .draw(|frame| {
                render_minimap_arrow(frame, area, &arrow, theme.border_unfocused);
            })
            .unwrap();

        // Then the row above shows `0 ▲`.
        let rows = jinn_testutil::buffer_rows(terminal.backend().buffer(), 40, 10);
        assert!(rows[2].contains('0'));
        // And the ▲ glyph accompanies the zero.
        assert!(rows[2].contains('▲'));
        // And the row below shows `0 ▼`.
        assert!(rows[4].contains('0'));
        // And the ▼ glyph accompanies the zero.
        assert!(rows[4].contains('▼'));
    }

    /// Fix 2: when no cursor has been placed (`selected_entry_index() == None`),
    /// the cursor is treated as the last visual item so the above/below
    /// counts are meaningful immediately on first render.
    #[rstest::rstest]
    fn no_cursor_falls_back_to_last_visual_item() {
        // Given three entries cached at 100, 200, and 300 tokens and no
        // cursor placed.
        let state = state_with_counts(&[100, 200, 300]);

        // When rendering the minimap.
        let (arrow, _rows) = render_to_buffer(&state, 1, 10);

        // Then the arrow is produced and its above-count is the sum of every
        // entry before the implicit last-item cursor.
        let arrow = arrow.expect("arrow should render even without cursor");
        assert_eq!(arrow.tokens_above, Some(300));
        // And the below-count is zero, since nothing follows the cursor.
        assert_eq!(arrow.tokens_below, Some(0));
    }
}
