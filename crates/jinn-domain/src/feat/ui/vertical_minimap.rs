//! Vertical minimap — one block per chat entry showing token-count sizing.
//!
//! Renders colored blocks (`█`) representing chat entries in a single-column
//! display, one entry per row. The color indicates approximate token count.
//! Entries without token counts produce an empty row. Excluded entry types
//! (Actor, empty assistant) produce no row at all.
//! The viewport scrolls to keep the selected entry visible. A `>` arrow overlay
//! on the chat log area points at the selected entry's row.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::common::app_state::AppState;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind};
use crate::feat::ui::chat_log::visual_item::VisualItem;

#[cfg(test)]
use crate::feat::ui::chat_log::visual_item::{
    DEFAULT_MIN_COLLAPSE_COUNT, PROXIMITY_COUNT, build_visual_items,
};

/// Full block character for minimap entries.
const FULL_BLOCK: &str = "\u{2588}";

/// Token count thresholds for minimap coloring.
const TOKEN_THRESHOLD_SMALL: u32 = 100;
const TOKEN_THRESHOLD_MEDIUM: u32 = 500;
const TOKEN_THRESHOLD_LARGE: u32 = 1000;

/// Returns the color for a token-count block based on size thresholds.
fn token_threshold_color(count: u32) -> Color {
    if count < TOKEN_THRESHOLD_SMALL {
        Color::Green
    } else if count < TOKEN_THRESHOLD_MEDIUM {
        Color::Yellow
    } else if count < TOKEN_THRESHOLD_LARGE {
        Color::Red
    } else {
        Color::Rgb(255, 0, 255) // bright magenta
    }
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
                let entry = &history[*hist_idx];
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
    /// Cached tiktoken token count, if available.
    token_count: Option<u32>,
}

/// Computes the list of visible (non-excluded) entries from visual items.
fn compute_visible_entries(state: &AppState) -> Vec<VisibleEntry> {
    let session = state.active_session();
    let history = session.history();
    let items = session.visual_items();

    let token_cache = state.frontend.caches.entry_token_cache.read();

    items
        .iter()
        .enumerate()
        .filter_map(|(vi_idx, item)| {
            if !item.is_minimap_visible(history) {
                return None;
            }
            let ignored = match item {
                VisualItem::CollapsedIgnoredBlock { .. } => false,
                VisualItem::Entry(hist_idx) => !history[*hist_idx].is_in_context(),
            };
            let token_count = if ignored {
                None
            } else {
                match item {
                    VisualItem::CollapsedIgnoredBlock { .. } => None,
                    VisualItem::Entry(hist_idx) => {
                        let entry = &history[*hist_idx];
                        token_cache.get(&entry.id)
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
}

pub fn render_vertical_minimap(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    muted_text_color: Color,
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
            let entry = &visible[block_index as usize];
            let span = match entry.token_count {
                Some(count) => Span::styled(
                    FULL_BLOCK.to_owned(),
                    Style::default().fg(token_threshold_color(count)),
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

    render_scroll_arrows(frame, area, selected_block, total_blocks, viewport_height, muted_text_color);

    let arrow_row = midpoint as u16;
    let selected_token_count = visible.get(selected_block).and_then(|e| e.token_count);
    Some(MinimapArrow { row: arrow_row, token_count: selected_token_count })
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
        let arrow_area = Rect { x: area.x, y: area.y, width: 1, height: 1 };
        let arrow = Paragraph::new(Line::from(Span::styled("▲", Style::default().fg(muted_text_color))));
        frame.render_widget(arrow, arrow_area);
    }

    if has_below {
        let bottom_y = area.y + area.height.saturating_sub(1);
        let arrow_area = Rect { x: area.x, y: bottom_y, width: 1, height: 1 };
        let arrow = Paragraph::new(Line::from(Span::styled("▼", Style::default().fg(muted_text_color))));
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

    #[allow(clippy::single_match_else, reason = "different match arms produce different widget layouts")]
    match arrow.token_count {
        Some(count) => {
            let formatted = format_entry_tokens(count);
            let text = format!("{formatted} >");
            let width = text.len() as u16;
            let x = chat_log_area.x.saturating_add(chat_log_area.width).saturating_sub(width);
            let paragraph = Paragraph::new(Line::from(Span::styled(text, Style::default().fg(arrow_color))));
            let arrow_area = Rect { x, y, width: width.min(chat_log_area.width), height: 1 };
            frame.render_widget(paragraph, arrow_area);
        }
        None => {
            let x = chat_log_area.x + chat_log_area.width.saturating_sub(1);
            let paragraph = Paragraph::new(Line::from(Span::styled(">", Style::default().fg(arrow_color))));
            let arrow_area = Rect { x, y, width: 1, height: 1 };
            frame.render_widget(paragraph, arrow_area);
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
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::feat::session::chat_entry::ChatEntry;
    use crate::feat::theme::default_theme;

    #[rstest::rstest]
    fn find_block_index_returns_position_for_existing_entry() {
        let visible = vec![
            VisibleEntry { vi_index: 0, token_count: None },
            VisibleEntry { vi_index: 2, token_count: None },
            VisibleEntry { vi_index: 5, token_count: None },
        ];
        assert_eq!(find_block_index(Some(2), &visible), Some(1));
    }

    #[rstest::rstest]
    fn find_block_index_returns_none_for_excluded_entry() {
        let visible = vec![
            VisibleEntry { vi_index: 0, token_count: None },
            VisibleEntry { vi_index: 2, token_count: None },
        ];
        assert!(find_block_index(Some(1), &visible).is_none());
    }

    #[rstest::rstest]
    fn find_block_index_returns_last_when_none() {
        let visible = vec![
            VisibleEntry { vi_index: 0, token_count: None },
            VisibleEntry { vi_index: 2, token_count: None },
        ];
        assert_eq!(find_block_index(None, &visible), Some(1));
    }

    #[rstest::rstest]
    fn find_block_index_returns_none_for_empty() {
        let visible: Vec<VisibleEntry> = vec![];
        assert!(find_block_index(Some(0), &visible).is_none());
    }

    #[rstest::rstest]
    fn scroll_is_midpoint_based() { assert_eq!(compute_minimap_scroll(4, 5, 10), 0); }

    #[rstest::rstest]
    fn scroll_centers_selected() { assert_eq!(compute_minimap_scroll(45, 50, 10), 40); }

    #[rstest::rstest]
    fn scroll_at_start_is_zero() { assert_eq!(compute_minimap_scroll(0, 50, 10), 0); }

    #[rstest::rstest]
    fn scroll_at_last_block() { assert_eq!(compute_minimap_scroll(49, 50, 10), 44); }

    #[rstest::rstest]
    fn scroll_near_midpoint() { assert_eq!(compute_minimap_scroll(5, 50, 10), 0); }

    fn render_to_buffer(state: &AppState, width: u16, height: u16) -> (Option<MinimapArrow>, Vec<String>) {
        setup_visual_items(state);
        let (mut terminal, area) = jinn_testutil::setup_term(width, height);
        let theme = default_theme();
        let mut arrow_result = None;
        terminal.draw(|frame| {
            arrow_result = render_vertical_minimap(frame, area, state, theme.muted_text);
        }).unwrap();
        let buffer = terminal.backend().buffer();
        let rows = jinn_testutil::buffer_rows(buffer, width, height);
        (arrow_result, rows)
    }

    #[rstest::rstest]
    fn empty_history_renders_nothing() {
        let state = AppState::default();
        let (arrow, rows) = render_to_buffer(&state, 1, 10);
        assert!(arrow.is_none());
        assert!(rows[0].trim().is_empty());
    }

    #[rstest::rstest]
    fn single_entry_no_cache_shows_space_at_midpoint() {
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("hello"));
        let (arrow, rows) = render_to_buffer(&state, 1, 10);
        assert!(arrow.is_some());
        assert!(!rows[5].contains('\u{2588}'), "no block without token count");
    }

    #[rstest::rstest]
    fn single_entry_with_cache_shows_block_at_midpoint() {
        let mut state = AppState::default();
        let entry = ChatEntry::user("hello world");
        let entry_id = entry.id.clone();
        state.active_session_mut().push_entry(entry);
        state.frontend.caches.entry_token_cache.write().insert(entry_id, 50);
        let (arrow, rows) = render_to_buffer(&state, 1, 10);
        assert!(arrow.is_some());
        assert!(rows[5].contains('\u{2588}'), "expected block at midpoint");
    }

    #[rstest::rstest]
    fn arrow_at_midpoint_when_last_entry_selected() {
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state.active_session_mut().push_entry(ChatEntry::assistant("b"));
        state.active_session_mut().push_entry(ChatEntry::user("c"));
        let (arrow, _) = render_to_buffer(&state, 1, 10);
        assert_eq!(arrow.expect("arrow exists").row, 5);
    }

    #[rstest::rstest]
    fn excluded_entries_produce_no_blocks() {
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state.active_session_mut().push_entry(ChatEntry::actor("bash", "output"));
        state.active_session_mut().push_entry(ChatEntry::thinking("reasoning"));
        state.active_session_mut().push_entry(ChatEntry::assistant("b"));
        let (arrow, rows) = render_to_buffer(&state, 1, 10);
        assert_eq!(rows.iter().filter(|r| r.contains('\u{2588}')).count(), 0);
        assert_eq!(arrow.expect("arrow").row, 5);
    }

    #[rstest::rstest]
    fn arrow_clamps_to_viewport_height() {
        let mut state = AppState::default();
        for i in 0..20 { state.active_session_mut().push_entry(ChatEntry::user(format!("msg {i}"))); }
        let (arrow, _) = render_to_buffer(&state, 1, 5);
        assert_eq!(arrow.expect("arrow").row, 2);
    }

    #[rstest::rstest]
    fn arrow_renders_greater_than_character() {
        let (mut terminal, area) = jinn_testutil::setup_term(40, 10);
        let arrow = MinimapArrow { row: 3, token_count: None };
        let theme = default_theme();
        terminal.draw(|frame| { render_minimap_arrow(frame, area, &arrow, theme.border_unfocused); }).unwrap();
        let rows = jinn_testutil::buffer_rows(terminal.backend().buffer(), 40, 10);
        assert!(rows[3].contains('>'));
    }

    #[rstest::rstest]
    fn scroll_down_arrow_at_bottom() {
        let mut state = AppState::default();
        for i in 0..20 { state.active_session_mut().push_entry(ChatEntry::user(format!("msg {i}"))); }
        state.active_session_mut().set_selected_entry_index(0);
        let (_, rows) = render_to_buffer(&state, 1, 5);
        assert!(rows[4].contains('▼'));
    }

    #[rstest::rstest]
    fn scroll_up_arrow_at_top() {
        let mut state = AppState::default();
        for i in 0..20 { state.active_session_mut().push_entry(ChatEntry::user(format!("msg {i}"))); }
        let (_, rows) = render_to_buffer(&state, 1, 5);
        assert!(rows[0].contains('▲'));
    }

    #[rstest::rstest]
    fn no_arrows_when_all_entries_fit() {
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state.active_session_mut().push_entry(ChatEntry::assistant("b"));
        state.active_session_mut().push_entry(ChatEntry::user("c"));
        let (_, rows) = render_to_buffer(&state, 1, 10);
        assert!(!rows.iter().any(|r| r.contains('▲')));
        assert!(!rows.iter().any(|r| r.contains('▼')));
    }

    #[rstest::rstest]
    fn empty_assistant_entry_produces_no_block() {
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::assistant(""));
        let (arrow, rows) = render_to_buffer(&state, 1, 10);
        assert!(arrow.is_none());
        assert_eq!(rows.iter().filter(|r| r.contains('\u{2588}')).count(), 0);
    }

    fn setup_visual_items(state: &AppState) {
        let session = state.active_session();
        let items = build_visual_items(session.history(), &session.ui.shown_ignored_blocks, PROXIMITY_COUNT, DEFAULT_MIN_COLLAPSE_COUNT);
        state.active_session().set_visual_items(items);
    }

    #[rstest::rstest]
    fn in_context_entry_with_cache_shows_block() {
        let mut state = AppState::default();
        let entry = ChatEntry::user("hello world this is a test");
        let entry_id = entry.id.clone();
        state.active_session_mut().push_entry(entry);
        state.frontend.caches.entry_token_cache.write().insert(entry_id, 500);
        let (_, rows) = render_to_buffer(&state, 1, 10);
        assert_eq!(rows[5].chars().filter(|&c| c == '\u{2588}').count(), 1);
    }

    #[rstest::rstest]
    fn entry_without_cache_shows_space() {
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("hello"));
        let (_, rows) = render_to_buffer(&state, 1, 10);
        assert_eq!(rows[5].chars().filter(|&c| c == '\u{2588}').count(), 0);
    }

    #[rstest::rstest]
    fn token_threshold_small_is_green() {
        assert_eq!(token_threshold_color(0), Color::Green);
        assert_eq!(token_threshold_color(99), Color::Green);
    }

    #[rstest::rstest]
    fn token_threshold_medium_is_yellow() {
        assert_eq!(token_threshold_color(100), Color::Yellow);
        assert_eq!(token_threshold_color(499), Color::Yellow);
    }

    #[rstest::rstest]
    fn token_threshold_large_is_red() {
        assert_eq!(token_threshold_color(500), Color::Red);
        assert_eq!(token_threshold_color(999), Color::Red);
    }

    #[rstest::rstest]
    fn token_threshold_extra_large_is_magenta() {
        assert_eq!(token_threshold_color(1000), Color::Rgb(255, 0, 255));
    }

    #[rstest::rstest]
    fn format_entry_tokens_small() { assert_eq!(format_entry_tokens(42), "42"); }

    #[rstest::rstest]
    fn format_entry_tokens_k() {
        assert_eq!(format_entry_tokens(1_000), "1.0k");
        assert_eq!(format_entry_tokens(42_500), "42.5k");
    }

    #[rstest::rstest]
    fn format_entry_tokens_m() {
        assert_eq!(format_entry_tokens(1_000_000), "1.0M");
    }

    #[rstest::rstest]
    fn arrow_with_token_count_renders_formatted_count() {
        let (mut terminal, area) = jinn_testutil::setup_term(40, 10);
        let arrow = MinimapArrow { row: 3, token_count: Some(3000) };
        let theme = default_theme();
        terminal.draw(|frame| { render_minimap_arrow(frame, area, &arrow, theme.border_unfocused); }).unwrap();
        let rows = jinn_testutil::buffer_rows(terminal.backend().buffer(), 40, 10);
        assert!(rows[3].contains('3'));
        assert!(rows[3].contains('>'));
    }

    #[rstest::rstest]
    fn arrow_without_token_count_renders_just_gt() {
        let (mut terminal, area) = jinn_testutil::setup_term(40, 10);
        let arrow = MinimapArrow { row: 3, token_count: None };
        let theme = default_theme();
        terminal.draw(|frame| { render_minimap_arrow(frame, area, &arrow, theme.border_unfocused); }).unwrap();
        let rows = jinn_testutil::buffer_rows(terminal.backend().buffer(), 40, 10);
        assert!(rows[3].contains('>'));
        assert!(!rows[3].contains('k'));
    }
}
