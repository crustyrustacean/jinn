//! The cursor arrow overlaid on the chat log, and the context accounting it shows.
//!
//! Separate from the minimap strip it points at. The strip is a self-contained
//! column of token-sized blocks whose only output is where the selected entry
//! sits; the arrow is a separate three-row widget drawn over the *chat log's*
//! area, and it is the only thing in the pair that reports how many tokens sit
//! above and below the cursor — the number that tells the user whether a
//! compaction is worth running.
//!
//! Nothing here needs to know how the strip colours or scrolls its blocks.
//! What flows between them is one value: the row the arrow should sit on,
//! plus the counts. Keeping that a plain data hand-off rather than a shared
//! renderer is what lets the strip be dropped or restyled without touching
//! this, and this be tested without building a minimap column.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use jinn_chat_log_view_msg::VisualItem;
use jinn_kernel::common::app_state::AppState;

/// Where the arrow sits, and the context accounting it reports.
///
/// Built by the minimap strip, drawn by [`render_minimap_arrow`] over the
/// chat log area. A plain data hand-off: neither side reaches into the other.
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

/// Returns the cursor's occupied history range as `[start, end)`.
///
/// - For `Entry(hist_idx)` → `[hist_idx, hist_idx + 1)`.
/// - For `CollapsedIgnoredBlock { start, count }` → `[start, start + count)`.
/// - When no visual item is selected (or the index is out of range) →
///   `[0, history_len)` (full range; both above and below sums become empty).
pub(super) fn cursor_history_range(
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
pub(super) fn compute_tokens_above(state: &AppState, end: usize) -> Option<u32> {
    compute_token_sum_in_range(state, 0, end)
}

/// Sums cached token counts for all `is_in_context()` entries in
/// history range `start..history.len()` (exclusive lower bound).
/// Returns `Some(0)` when the range is empty or all entries are excluded.
pub(super) fn compute_tokens_below(state: &AppState, start: usize) -> Option<u32> {
    let history_len = state.active_session().history().len();
    compute_token_sum_in_range(state, start, history_len)
}

/// Sums cached token counts across `start..end`, counting only entries that
/// are still in the context window.
///
/// Returns `Some(0)` for an empty range rather than `None`, because "nothing
/// above the cursor" is a real answer the arrow renders as `0 ▲`, and
/// collapsing it into `None` would render the glyph alone instead. The
/// saturating add keeps one pathological entry from wrapping the total.
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
    use jinn_kernel::protocol::ChatEntry;
    use jinn_theme::default_theme;

    use jinn_chat_log_view_msg::{DEFAULT_MIN_COLLAPSE_COUNT, PROXIMITY_COUNT, build_visual_items};

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
}
