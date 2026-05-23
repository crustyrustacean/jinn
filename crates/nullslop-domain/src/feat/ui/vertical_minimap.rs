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
use crate::feat::session::chat_entry::ChatEntryKind;
use crate::feat::theme::contrast::darken;

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
            Self::Thinking => Color::DarkGray,
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

/// A visible entry in the minimap (non-excluded).
struct VisibleEntry {
    /// Index into the history slice.
    history_index: usize,
    /// Color category.
    category: MinimapCategory,
    /// Whether the entry is ignored.
    ignored: bool,
}

/// Computes the list of visible (non-excluded) entries from history.
fn compute_visible_entries(state: &AppState) -> Vec<VisibleEntry> {
    let session = state.active_session();
    session
        .history()
        .iter()
        .enumerate()
        .filter_map(|(i, entry)| {
            if entry.is_empty_assistant() {
                return None;
            }
            MinimapCategory::from_kind(&entry.kind).map(|cat| VisibleEntry {
                history_index: i,
                category: cat,
                ignored: entry.ignored,
            })
        })
        .collect()
}

/// Finds the block index corresponding to the given history index.
///
/// Returns `None` if the history index maps to an excluded entry or
/// is out of range.
fn find_block_index(history_idx: Option<usize>, visible: &[VisibleEntry]) -> Option<usize> {
    match history_idx {
        Some(idx) => visible.iter().position(|e| e.history_index == idx),
        None => visible.len().checked_sub(1),
    }
}

/// Computes the scroll offset for the minimap viewport.
///
/// The selected block is always positioned at the midpoint row
/// (`viewport_height / 2`). The offset is simply the block index
/// minus the midpoint. At the start, this produces empty space above;
/// at the end, empty space below.
#[expect(dead_code, reason = "available for future use")]
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
    use crate::feat::session::chat_entry::ChatEntry;
    
    use crate::feat::theme::default_theme;

    // --- find_block_index ---

    #[rstest::rstest]
    fn find_block_index_returns_position_for_existing_entry() {
        // Given visible entries mapping to history indices 0, 2, 5.
        let visible = vec![
            VisibleEntry {
                history_index: 0,
                category: MinimapCategory::User,
                ignored: false,
            },
            VisibleEntry {
                history_index: 2,
                category: MinimapCategory::Assistant,
                ignored: false,
            },
            VisibleEntry {
                history_index: 5,
                category: MinimapCategory::User,
                ignored: false,
            },
        ];

        // When looking for history index 2.
        let result = find_block_index(Some(2), &visible);

        // Then it returns block index 1.
        assert_eq!(result, Some(1));
    }

    #[rstest::rstest]
    fn find_block_index_returns_none_for_excluded_entry() {
        // Given visible entries at history indices 0, 2.
        let visible = vec![
            VisibleEntry {
                history_index: 0,
                category: MinimapCategory::User,
                ignored: false,
            },
            VisibleEntry {
                history_index: 2,
                category: MinimapCategory::Assistant,
                ignored: false,
            },
        ];

        // When looking for history index 1 (excluded).
        let result = find_block_index(Some(1), &visible);

        // Then it returns None (no fallback).
        assert!(result.is_none());
    }

    #[rstest::rstest]
    fn find_block_index_returns_last_when_none() {
        // Given visible entries at history indices 0, 2, 5.
        let visible = vec![
            VisibleEntry {
                history_index: 0,
                category: MinimapCategory::User,
                ignored: false,
            },
            VisibleEntry {
                history_index: 2,
                category: MinimapCategory::Assistant,
                ignored: false,
            },
        ];

        // When history index is None.
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
}
