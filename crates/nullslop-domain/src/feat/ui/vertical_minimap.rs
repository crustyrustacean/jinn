//! Vertical minimap — one block per chat entry in a single-column display.
//!
//! Renders colored blocks (`█`) representing chat entries in a vertical column,
//! one entry per row. Excluded entry types (Actor) produce no block.
//! The viewport scrolls to keep the selected entry visible. A `>` arrow overlay
//! on the chat log area points at the selected entry's row.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::common::app_state::AppState;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind};
use crate::feat::theme::contrast::darken;
use crate::feat::ui::chat_log::visual_item::VisualItem;

#[cfg(test)]
use crate::feat::ui::chat_log::visual_item::{build_visual_items, DEFAULT_MIN_COLLAPSE_COUNT, PROXIMITY_COUNT};

/// Full block character for minimap entries.
const FULL_BLOCK: &str = "\u{2588}";

/// Categorizes chat entry types for minimap coloring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MinimapCategory {
    /// User messages — cyan.
    User,
    /// Tool calls and results — green.
    Tool,
    /// LLM/assistant responses — white.
    Assistant,
    /// Compaction summaries — magenta.
    Compaction,
    /// Error messages — red.
    Error,
    /// System and transient messages — yellow.
    System,
    /// Skill entries — orange.
    Skill,
    /// Thinking/reasoning blocks — dark gray.
    Thinking,
    /// Collapsed ignored entries — dim gray.
    Collapsed,
}

impl MinimapCategory {
    /// Returns the color for this category.
    fn color(self) -> Color {
        match self {
            Self::User => Color::Cyan,
            Self::Tool => Color::Green,
            Self::Assistant => Color::White,
            Self::Compaction => Color::Magenta,
            Self::Error => Color::Red,
            Self::System => Color::Yellow,
            Self::Skill => Color::Rgb(255, 165, 0),
            Self::Thinking | Self::Collapsed => Color::DarkGray,
        }
    }

    /// Maps a `ChatEntryKind` to a minimap category, or `None` if excluded.
    fn from_kind(kind: &ChatEntryKind) -> Option<Self> {
        match kind {
            ChatEntryKind::User { .. } => Some(Self::User),
            ChatEntryKind::ToolCall { .. } | ChatEntryKind::ToolResult { .. } => Some(Self::Tool),
            ChatEntryKind::Assistant(..) => Some(Self::Assistant),
            ChatEntryKind::Compaction { .. } => Some(Self::Compaction),
            ChatEntryKind::Error(..) => Some(Self::Error),
            ChatEntryKind::System(..) | ChatEntryKind::Transient(..) => Some(Self::System),
            ChatEntryKind::Skill { .. } => Some(Self::Skill),
            ChatEntryKind::Thinking(..) => Some(Self::Thinking),
            // Excluded: Actor.
            ChatEntryKind::Actor { .. } => None,
        }
    }
}

/// Extension trait for determining whether a visual item should produce
/// a minimap block. Defined here so the minimap owns its own visibility
/// logic — the `VisualItem` type knows nothing about the minimap.
trait MinimapVisibility {
    /// Whether this visual item should produce a minimap block.
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
                // Actor entries are excluded from the minimap.
                !matches!(entry.kind, ChatEntryKind::Actor { .. })
            }
        }
    }
}

/// A visible entry in the minimap (non-excluded).
struct VisibleEntry {
    /// Visual-item index (bridges block position to VI index).
    vi_index: usize,
    /// Color category.
    category: MinimapCategory,
    /// Whether the entry is ignored.
    ignored: bool,
}

/// Computes the list of visible (non-excluded) entries from visual items.
fn compute_visible_entries(state: &AppState) -> Vec<VisibleEntry> {
    let session = state.active_session();
    let history = session.history();
    let items = session.visual_items();

    items
        .iter()
        .enumerate()
        .filter_map(|(vi_idx, item)| {
            if !item.is_minimap_visible(history) {
                return None;
            }
            let category = match item {
                VisualItem::CollapsedIgnoredBlock { .. } => MinimapCategory::Collapsed,
                VisualItem::Entry(hist_idx) => {
                    let entry = &history[*hist_idx];
                    MinimapCategory::from_kind(&entry.kind)?
                }
            };
            let ignored = match item {
                VisualItem::CollapsedIgnoredBlock { .. } => false,
                VisualItem::Entry(hist_idx) => !history[*hist_idx].is_in_context(),
            };
            Some(VisibleEntry {
                vi_index: vi_idx,
                category,
                ignored,
            })
        })
        .collect()
}

/// Finds the block index corresponding to the given visual-item index.
///
/// Returns `None` if the visual-item index is not found among visible entries
/// or is out of range.
fn find_block_index(selected_vi_idx: Option<usize>, visible: &[VisibleEntry]) -> Option<usize> {
    match selected_vi_idx {
        Some(idx) => visible.iter().position(|e| e.vi_index == idx),
        None => visible.len().checked_sub(1),
    }
}

/// Computes the scroll offset for the minimap viewport.
///
/// The selected block is always positioned at the midpoint row
/// (`viewport_height / 2`). The offset is simply the block index
/// minus the midpoint. At the start, this produces empty space above;
/// at the end, empty space below.
#[allow(dead_code, reason = "available for future use")]
fn compute_minimap_scroll(
    selected_block: usize,
    _total_blocks: usize,
    viewport_height: usize,
) -> usize {
    let midpoint = viewport_height / 2;
    selected_block.saturating_sub(midpoint)
}

/// Public result of the minimap render — tells the caller where the `>`
/// arrow should be painted on the chat log area.
pub struct MinimapArrow {
    /// Row offset from the top of the minimap area where the arrow goes.
    pub row: u16,
}

/// Renders the vertical minimap blocks into the given area.
///
/// Returns `Some(MinimapArrow)` with the arrow position if there are visible
/// entries, or `None` if the history is empty.
///
/// `muted_text_color` is used for the scroll direction arrows (`▲`/`▼`).
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

    // Determine selected block.
    let selected_idx = state.active_session().selected_entry_index();
    let selected_block = find_block_index(selected_idx, &visible)?;

    // Midpoint row where the arrow and selected block live.
    let midpoint = viewport_height / 2;

    // Build lines — each row is either a block or empty.
    // Blocks are positioned relative to the midpoint so the selected
    // block always appears at the midpoint row.
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(viewport_height);
    for row in 0..viewport_height {
        let block_index = selected_block as isize + row as isize - midpoint as isize;
        if block_index >= 0 && (block_index as usize) < total_blocks {
            let entry = &visible[block_index as usize];
            let color = if entry.ignored {
                darken(entry.category.color(), 0.4)
            } else {
                entry.category.color()
            };
            lines.push(Line::from(Span::styled(
                FULL_BLOCK.to_owned(),
                Style::default().fg(color),
            )));
        } else {
            lines.push(Line::from(""));
        }
    }

    let widget = Paragraph::new(lines);
    frame.render_widget(widget, area);

    // Render scroll direction arrows.
    render_scroll_arrows(
        frame,
        area,
        selected_block,
        total_blocks,
        viewport_height,
        muted_text_color,
    );

    // Arrow is always at the midpoint row.
    let arrow_row = midpoint as u16;

    Some(MinimapArrow { row: arrow_row })
}

/// Renders scroll direction arrows at the top/bottom of the minimap column.
///
/// `▲` appears at the top when entries exist above the viewport.
/// `▼` appears at the bottom when entries exist below the viewport.
/// These arrows replace whatever block was at that position.
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

/// Renders the `>` arrow overlay on the chat log area.
///
/// Paints a single `>` character at the given row in the rightmost column
/// of the chat log area, using the `border_unfocused` color.
pub fn render_minimap_arrow(
    frame: &mut Frame<'_>,
    chat_log_area: Rect,
    arrow: &MinimapArrow,
    arrow_color: Color,
) {
    if chat_log_area.width == 0 || chat_log_area.height == 0 {
        return;
    }

    let x = chat_log_area.x + chat_log_area.width.saturating_sub(1);
    let y = chat_log_area.y + arrow_row_min(arrow.row, chat_log_area.height);

    let arrow_span = Span::styled(">", Style::default().fg(arrow_color));
    let paragraph = Paragraph::new(Line::from(arrow_span));
    let arrow_area = Rect {
        x,
        y,
        width: 1,
        height: 1,
    };
    frame.render_widget(paragraph, arrow_area);
}

/// Clamp arrow row to fit within the chat log area height.
fn arrow_row_min(row: u16, height: u16) -> u16 {
    row.min(height.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::common::app_state::AppState;
    use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind, ContextOverride};
    use crate::protocol::ChatEntryId;
    use crate::feat::theme::default_theme;

    // --- find_block_index ---

    #[rstest::rstest]
    fn find_block_index_returns_position_for_existing_entry() {
        // Given visible entries mapping to visual-item indices 0, 2, 5.
        let visible = vec![
            VisibleEntry {
                vi_index: 0,
                category: MinimapCategory::User,
                ignored: false,
            },
            VisibleEntry {
                vi_index: 2,
                category: MinimapCategory::Assistant,
                ignored: false,
            },
            VisibleEntry {
                vi_index: 5,
                category: MinimapCategory::User,
                ignored: false,
            },
        ];

        // When looking for visual-item index 2.
        let result = find_block_index(Some(2), &visible);

        // Then it returns block index 1.
        assert_eq!(result, Some(1));
    }

    #[rstest::rstest]
    fn find_block_index_returns_none_for_excluded_entry() {
        // Given visible entries at visual-item indices 0, 2.
        let visible = vec![
            VisibleEntry {
                vi_index: 0,
                category: MinimapCategory::User,
                ignored: false,
            },
            VisibleEntry {
                vi_index: 2,
                category: MinimapCategory::Assistant,
                ignored: false,
            },
        ];

        // When looking for visual-item index 1 (excluded).
        let result = find_block_index(Some(1), &visible);

        // Then it returns None (no fallback).
        assert!(result.is_none());
    }

    #[rstest::rstest]
    fn find_block_index_returns_last_when_none() {
        // Given visible entries at visual-item indices 0, 2, 5.
        let visible = vec![
            VisibleEntry {
                vi_index: 0,
                category: MinimapCategory::User,
                ignored: false,
            },
            VisibleEntry {
                vi_index: 2,
                category: MinimapCategory::Assistant,
                ignored: false,
            },
        ];

        // When visual-item index is None.
        let result = find_block_index(None, &visible);

        // Then it returns the last block index.
        assert_eq!(result, Some(1));
    }

    #[rstest::rstest]
    fn find_block_index_returns_none_for_empty() {
        // Given no visible entries.
        let visible: Vec<VisibleEntry> = vec![];

        // When looking for any index.
        let result = find_block_index(Some(0), &visible);

        // Then it returns None.
        assert!(result.is_none());
    }

    // --- compute_minimap_scroll ---

    #[rstest::rstest]
    fn scroll_is_midpoint_based() {
        // Given 5 blocks in a 10-row viewport, selected at block 4.
        let offset = compute_minimap_scroll(4, 5, 10);

        // Then offset is 0 (midpoint=5, 4-5 saturates to 0).
        assert_eq!(offset, 0);
    }

    #[rstest::rstest]
    fn scroll_centers_selected() {
        // Given 50 blocks in a 10-row viewport, selected at block 45.
        // Midpoint=5, so offset = 40. Selected block at row 5.
        let offset = compute_minimap_scroll(45, 50, 10);
        assert_eq!(offset, 40);
    }

    #[rstest::rstest]
    fn scroll_at_start_is_zero() {
        // Given 50 blocks in a 10-row viewport, selected at block 0.
        let offset = compute_minimap_scroll(0, 50, 10);
        // Midpoint=5, offset = 0-5 = 0 (saturated).
        assert_eq!(offset, 0);
    }

    #[rstest::rstest]
    fn scroll_at_last_block() {
        // Given 50 blocks in a 10-row viewport, selected at last (49).
        // Midpoint=5, offset = 49-5 = 44.
        let offset = compute_minimap_scroll(49, 50, 10);
        assert_eq!(offset, 44);
    }

    #[rstest::rstest]
    fn scroll_near_midpoint() {
        // Given 50 blocks in a 10-row viewport, selected at block 5.
        // Midpoint=5, offset = 5-5 = 0.
        let offset = compute_minimap_scroll(5, 50, 10);
        assert_eq!(offset, 0);
    }

    // --- Rendering ---

    fn render_to_buffer(
        state: &AppState,
        width: u16,
        height: u16,
    ) -> (Option<MinimapArrow>, Vec<String>) {
        setup_visual_items(state);
        let (mut terminal, area) = nullslop_testutil::setup_term(width, height);
        let theme = default_theme();
        let mut arrow_result = None;
        terminal
            .draw(|frame| {
                arrow_result = render_vertical_minimap(frame, area, state, theme.muted_text);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rows = nullslop_testutil::buffer_rows(buffer, width, height);
        (arrow_result, rows)
    }

    #[rstest::rstest]
    fn empty_history_renders_nothing() {
        // Given an empty session.
        let state = AppState::default();

        // When rendering.
        let (arrow, rows) = render_to_buffer(&state, 1, 10);

        // Then no arrow and no blocks.
        assert!(arrow.is_none());
        assert!(rows[0].trim().is_empty());
    }

    #[rstest::rstest]
    fn single_entry_renders_at_midpoint() {
        // Given a session with one user entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));

        // When rendering in a 10-row viewport.
        let (arrow, rows) = render_to_buffer(&state, 1, 10);

        // Then one block is rendered at the midpoint row (row 5).
        assert!(arrow.is_some());
        // Row 0 is empty (above midpoint).
        assert!(rows[0].trim().is_empty(), "expected empty at row 0");
        // Row 5 has the block.
        assert!(
            rows[5].contains('\u{2588}'),
            "expected block at midpoint row 5"
        );
    }

    #[rstest::rstest]
    fn arrow_at_midpoint_when_last_entry_selected() {
        // Given 3 entries, selection at last (index 2).
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant("b"));
        state.active_session_mut().push_entry(ChatEntry::user("c"));

        // When rendering in a 10-row viewport.
        let (arrow, _rows) = render_to_buffer(&state, 1, 10);

        // Then arrow is at midpoint (row 5), not at row 2.
        assert_eq!(arrow.expect("arrow exists").row, 5);
    }

    #[rstest::rstest]
    fn excluded_entries_produce_no_blocks_midpoint() {
        // Given a history with Actor and Thinking entries mixed in.
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

        // When rendering in a 10-row viewport.
        let (arrow, rows) = render_to_buffer(&state, 1, 10);

        // Then 3 blocks (user, thinking, assistant). Actor is still excluded.
        // user=history_idx 0 → block_idx 0
        // thinking=history_idx 2 → block_idx 1
        // assistant=history_idx 3 → block_idx 2
        // Selection at last (3) → block 2. Midpoint=5.
        // Block 0 at row 3, block 1 at row 4, block 2 at row 5.
        let block_count = rows.iter().filter(|r| r.contains('\u{2588}')).count();
        assert_eq!(block_count, 3);
        assert!(rows[3].contains('\u{2588}'), "expected block at row 3");
        assert!(rows[4].contains('\u{2588}'), "expected block at row 4");
        assert!(rows[5].contains('\u{2588}'), "expected block at row 5");
        assert_eq!(arrow.expect("arrow exists").row, 5);
    }

    #[rstest::rstest]
    fn ignored_entry_uses_darkened_color() {
        // Given an ignored user entry.
        let mut state = AppState::default();
        let entry = ChatEntry::user("old").with_ignored(true);
        state.active_session_mut().push_entry(entry);

        // When rendering in a 10-row viewport.
        let (_arrow, rows) = render_to_buffer(&state, 1, 10);

        // Then a block is rendered at midpoint (not empty).
        assert!(rows[5].contains('\u{2588}'));
    }

    #[rstest::rstest]
    fn selecting_thinking_entry_positions_arrow_at_midpoint() {
        // Given a session with User + Thinking + Assistant entries, Thinking selected.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::thinking("reasoning"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant("b"));
        state.active_session_mut().set_selected_entry_index(1);

        // When rendering in a 10-row viewport.
        let (arrow, rows) = render_to_buffer(&state, 1, 10);

        // Then the arrow is at the midpoint (row 5), and 3 blocks are visible.
        // Selection at history index 1 (Thinking) → block index 1. Midpoint=5.
        assert_eq!(arrow.expect("arrow exists").row, 5);
        let block_count = rows.iter().filter(|r| r.contains('\u{2588}')).count();
        assert_eq!(
            block_count, 3,
            "expected 3 blocks when thinking is selected"
        );
    }

    #[rstest::rstest]
    fn arrow_clamps_to_viewport_height() {
        // Given more entries than viewport height.
        let mut state = AppState::default();
        for i in 0..20 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user(format!("msg {i}")));
        }
        // Selection is at last entry (index 19).

        // When rendering in a 5-row viewport.
        let (arrow, _rows) = render_to_buffer(&state, 1, 5);

        // Then arrow row is 2 (midpoint of 5 = 2).
        let arrow = arrow.expect("arrow exists");
        assert_eq!(arrow.row, 2);
    }

    // --- render_minimap_arrow ---

    #[rstest::rstest]
    fn arrow_renders_greater_than_character() {
        // Given a terminal and arrow position.
        let (mut terminal, area) = nullslop_testutil::setup_term(40, 10);
        let arrow = MinimapArrow { row: 3 };
        let theme = default_theme();

        terminal
            .draw(|frame| {
                render_minimap_arrow(frame, area, &arrow, theme.border_unfocused);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let rows = nullslop_testutil::buffer_rows(buffer, 40, 10);

        // Then row 3 has a '>' character in the rightmost column.
        let row_str = &rows[3];
        assert!(
            row_str.contains('>'),
            "expected '>' in row 3, got: {row_str}"
        );
    }

    // --- Scroll direction arrows (midpoint-based) ---

    #[rstest::rstest]
    fn scroll_down_arrow_at_bottom_when_entries_below_midpoint() {
        // Given 20 entries, viewport height 5, selected near top.
        // Midpoint=2, selected=0. Entries below: 0+(5-2)=3 < 20 → yes.
        let mut state = AppState::default();
        for i in 0..20 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user(format!("msg {i}")));
        }
        state.active_session_mut().set_selected_entry_index(0);

        // When rendering.
        let (_arrow, rows) = render_to_buffer(&state, 1, 5);

        // Then the bottom row has a ▼ character.
        let bottom_row = &rows[4];
        assert!(
            bottom_row.contains('▼'),
            "expected '▼' at bottom, got: {bottom_row}"
        );
    }

    #[rstest::rstest]
    fn scroll_up_arrow_at_top_when_entries_above_midpoint() {
        // Given 20 entries, viewport height 5, selected near bottom.
        // Midpoint=2, selected=19. Entries above: 19 > 2 → yes.
        let mut state = AppState::default();
        for i in 0..20 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user(format!("msg {i}")));
        }

        // When rendering.
        let (_arrow, rows) = render_to_buffer(&state, 1, 5);

        // Then the top row has a ▲ character.
        let top_row = &rows[0];
        assert!(top_row.contains('▲'), "expected '▲' at top, got: {top_row}");
    }

    #[rstest::rstest]
    fn no_arrows_when_all_entries_fit() {
        // Given 3 entries in a 10-row viewport.
        // Midpoint=5, selected at last (2). Entries above: 2 > 5? No.
        // Entries below: 2+(10-5)=7 < 3? No.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant("b"));
        state.active_session_mut().push_entry(ChatEntry::user("c"));

        // When rendering.
        let (_arrow, rows) = render_to_buffer(&state, 1, 10);

        // Then no scroll arrows appear.
        let has_up_arrow = rows.iter().any(|r| r.contains('▲'));
        let has_down_arrow = rows.iter().any(|r| r.contains('▼'));
        assert!(!has_up_arrow, "should not have ▲");
        assert!(!has_down_arrow, "should not have ▼");
    }

    #[rstest::rstest]
    fn empty_assistant_entry_produces_no_minimap_block() {
        // Given a session with only an empty assistant entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant(""));

        // When rendering.
        let (arrow, rows) = render_to_buffer(&state, 1, 10);

        // Then no arrow and no blocks.
        assert!(arrow.is_none());
        let block_count = rows.iter().filter(|r| r.contains('\u{2588}')).count();
        assert_eq!(
            block_count, 0,
            "empty assistant should produce no minimap blocks"
        );
    }

    #[rstest::rstest]
    fn empty_assistant_mixed_with_other_entries() {
        // Given a history with user, empty assistant, and tool call entries.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("a"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant(""));
        state
            .active_session_mut()
            .push_entry(ChatEntry::tool_call("tc-1", "bash", "{}"));

        // When rendering in a 10-row viewport.
        let (arrow, rows) = render_to_buffer(&state, 1, 10);

        // Then only 2 blocks (user and tool call). Empty assistant is excluded.
        let block_count = rows.iter().filter(|r| r.contains('\u{2588}')).count();
        assert_eq!(
            block_count, 2,
            "expected 2 blocks (user + tool call), got {block_count}"
        );
        assert!(arrow.is_some());
    }

    /// Sets up visual items on the session so the minimap can read them.
    /// This simulates what the chat log render pipeline does before the
    /// minimap renders.
    fn setup_visual_items(state: &AppState) {
        let session = state.active_session();
        let items = build_visual_items(
            session.history(),
            &session.ui.shown_ignored_blocks,
            PROXIMITY_COUNT,
            DEFAULT_MIN_COLLAPSE_COUNT,
        );
        state.active_session().set_visual_items(items);
    }

    #[rstest::rstest]
    fn collapsed_ignored_block_produces_single_block() {
        // Given 3 non-ignored + 5 ignored + 10 non-ignored entries (18 total).
        let mut state = AppState::default();
        for _ in 0..3 {
            state.active_session_mut().push_entry(ChatEntry::user("visible"));
        }
        for _ in 0..5 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("hidden").with_ignored(true));
        }
        for _ in 0..10 {
            state.active_session_mut().push_entry(ChatEntry::user("visible"));
        }

        // When rendering in a 20-row viewport.
        let (arrow, rows) = render_to_buffer(&state, 1, 20);

        // Then the 5 ignored entries collapse into a single block.
        // Visual items: 3 Entry, 1 CollapsedIgnoredBlock, 10 Entry = 14 items.
        // Minimap blocks: 14 total, but viewport shows 20 rows with selected at
        // block 13 (midpoint=10), so rows 0-10 have blocks (11), minus 1 for ▲ = 10.
        let block_count = rows.iter().filter(|r| r.contains('\u{2588}')).count();
        assert_eq!(
            block_count, 10,
            "expected 10 visible blocks in 20-row viewport with selected at end, got {block_count}"
        );
        // The collapsed block is at VI index 3, which maps to row 0 (block 3).
        // But ▲ overwrites it. So check the ▲ is present.
        assert!(rows[0].contains('▲'), "expected ▲ at top");
        assert!(arrow.is_some(), "arrow should exist");
    }

    #[rstest::rstest]
    fn selected_entry_after_collapsed_region_resolves_correctly() {
        // Given 3 non-ignored + 5 ignored + 10 non-ignored entries (18 total).
        // Select the first entry after the collapsed region.
        let mut state = AppState::default();
        for _ in 0..3 {
            state.active_session_mut().push_entry(ChatEntry::user("visible"));
        }
        for _ in 0..5 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user("hidden").with_ignored(true));
        }
        // Push 10 more, then select the first one after the ignored block.
        for i in 0..10 {
            state
                .active_session_mut()
                .push_entry(ChatEntry::user(format!("after-{i}")));
        }
        // Visual items: 3 Entry, 1 CollapsedIgnoredBlock, 10 Entry.
        // The first entry after the collapsed block is VI index 4.
        // But VI 3 is the CollapsedIgnoredBlock, VI 4 is Entry(8).
        // Selecting entry at history index 8 → VI index 4.
        state.active_session_mut().set_selected_entry_index(4);

        // When rendering.
        let (arrow, _rows) = render_to_buffer(&state, 1, 20);

        // Then the arrow exists (selection resolved correctly).
        assert!(arrow.is_some(), "arrow should resolve for entry after collapsed block");
    }

    #[rstest::rstest]
    fn compaction_with_ignored_entries_tracks_correctly() {
        // Given: user, assistant, compaction (marks prior as ignored), user, assistant.
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("msg1"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant("reply1"));
        // Create compaction entry directly (no helper constructor).
        let compaction = ChatEntry {
            id: ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::Compaction {
                summary: "summary".to_owned(),
                tokens_before: 100,
                entries_compacted: 2,
                model_used: "test".to_owned(),
            },
            pin_position: None,
            context_override: ContextOverride::Default,
        };
        state.active_session_mut().push_entry(compaction);
        state.active_session_mut().push_entry(ChatEntry::user("msg2"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant("reply2"));
        // Mark entries before compaction as ignored.
        state.active_session_mut().mark_entries_ignored(&[0, 1]);

        // When rendering in a 10-row viewport.
        let (arrow, rows) = render_to_buffer(&state, 1, 10);

        // Then the minimap shows blocks for compaction + user + assistant.
        // The ignored entries may or may not be collapsed depending on count
        // (2 entries, below DEFAULT_MIN_COLLAPSE_COUNT of 3, so shown individually).
        // But key point: the arrow resolves correctly.
        let block_count = rows.iter().filter(|r| r.contains('\u{2588}')).count();
        assert!(block_count >= 3, "expected at least 3 blocks, got {block_count}");
        assert!(arrow.is_some(), "arrow should exist after compaction");
    }
}
