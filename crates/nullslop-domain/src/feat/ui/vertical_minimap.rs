//! Vertical minimap — one block per chat entry in a single-column display.
//!
//! Renders colored blocks (`█`) representing chat entries in a vertical column,
//! one entry per row. Excluded entry types (Actor, Thinking) produce no block.
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
            // Excluded: Actor, Thinking.
            ChatEntryKind::Actor { .. } | ChatEntryKind::Thinking(..) => None,
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
/// is out of range. Falls back to the last visible entry if no match.
fn find_block_index(
    history_idx: Option<usize>,
    visible: &[VisibleEntry],
) -> Option<usize> {
    match history_idx {
        Some(idx) => visible
            .iter()
            .position(|e| e.history_index == idx)
            .or_else(|| visible.len().checked_sub(1)),
        None => visible.len().checked_sub(1),
    }
}

/// Computes the scroll offset for the minimap viewport.
///
/// Keeps `selected_block` visible. If everything fits, offset is 0.
/// Otherwise, clamps so the selected block is within the viewport.
/// Default alignment is bottom (selected near the bottom of viewport).
fn compute_minimap_scroll(selected_block: usize, total_blocks: usize, viewport_height: usize) -> usize {
    if total_blocks <= viewport_height {
        return 0;
    }

    let max_offset = total_blocks.saturating_sub(viewport_height);

    // Default: bottom-aligned.
    let default_offset = max_offset;

    // If selected is already visible in the default viewport, keep it.
    if selected_block >= default_offset && selected_block < default_offset + viewport_height {
        return default_offset;
    }

    // Center the selected block in the viewport.
    let centered = selected_block.saturating_sub(viewport_height / 2);
    centered.min(max_offset)
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
pub fn render_vertical_minimap(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
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

    // Compute scroll offset.
    let scroll_offset = compute_minimap_scroll(selected_block, total_blocks, viewport_height);

    // Build lines for visible blocks.
    let mut lines: Vec<Line<'static>> = Vec::new();
    let end = (scroll_offset + viewport_height).min(total_blocks);

    for block_i in scroll_offset..end {
        let entry = &visible[block_i];
        let color = if entry.ignored {
            darken(entry.category.color(), 0.4)
        } else {
            entry.category.color()
        };
        lines.push(Line::from(Span::styled(
            FULL_BLOCK.to_owned(),
            Style::default().fg(color),
        )));
    }

    // Pad with empty lines if viewport is larger than remaining blocks.
    while lines.len() < viewport_height {
        lines.push(Line::from(""));
    }

    let widget = Paragraph::new(lines);
    frame.render_widget(widget, area);

    // Compute arrow row (offset from top of minimap area).
    let arrow_row = selected_block.saturating_sub(scroll_offset) as u16;

    Some(MinimapArrow { row: arrow_row })
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
    use crate::feat::theme::Theme;

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
    fn find_block_index_falls_back_to_last_for_excluded_entry() {
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

        // When looking for history index 3 (excluded).
        let result = find_block_index(Some(3), &visible);

        // Then it falls back to the last visible entry (block index 1).
        assert_eq!(result, Some(1));
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
    fn scroll_is_zero_when_everything_fits() {
        // Given 5 blocks in a 10-row viewport.
        let offset = compute_minimap_scroll(4, 5, 10);

        // Then scroll offset is 0.
        assert_eq!(offset, 0);
    }

    #[rstest::rstest]
    fn scroll_keeps_selected_visible() {
        // Given 50 blocks in a 10-row viewport, selected at block 45.
        let offset = compute_minimap_scroll(45, 50, 10);

        // Then the selected block is within the viewport.
        assert!(offset <= 45);
        assert!(45 < offset + 10);
    }

    #[rstest::rstest]
    fn scroll_defaults_to_bottom() {
        // Given 50 blocks in a 10-row viewport, selected at block 49 (last).
        let offset = compute_minimap_scroll(49, 50, 10);

        // Then viewport is bottom-aligned.
        assert_eq!(offset, 40);
    }

    #[rstest::rstest]
    fn scroll_centers_selected_when_far_from_bottom() {
        // Given 50 blocks in a 10-row viewport, selected at block 5.
        let offset = compute_minimap_scroll(5, 50, 10);

        // Then selected is centered-ish.
        assert!(offset <= 5);
        assert!(5 < offset + 10);
    }

    #[rstest::rstest]
    fn scroll_clamps_at_max_offset() {
        // Given 20 blocks in a 10-row viewport, selected at block 0.
        let offset = compute_minimap_scroll(0, 20, 10);

        // Then offset is 0 (selected is at the top).
        assert_eq!(offset, 0);
    }

    // --- Rendering ---

    fn render_to_buffer(
        state: &AppState,
        width: u16,
        height: u16,
    ) -> (Option<MinimapArrow>, Vec<String>) {
        let (mut terminal, area) = nullslop_testutil::setup_term(width, height);
        let mut arrow_result = None;
        terminal
            .draw(|frame| {
                arrow_result = render_vertical_minimap(frame, area, state);
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
    fn single_entry_renders_one_block() {
        // Given a session with one user entry.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("hello"));

        // When rendering.
        let (arrow, rows) = render_to_buffer(&state, 1, 10);

        // Then one block is rendered.
        assert!(arrow.is_some());
        assert!(rows[0].contains('\u{2588}'));
    }

    #[rstest::rstest]
    fn arrow_points_at_last_entry_when_no_selection() {
        // Given 3 entries, no explicit selection.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("a"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant("b"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("c"));
        // After push, selection is at last entry (index 2).

        // When rendering.
        let (arrow, _rows) = render_to_buffer(&state, 1, 10);

        // Then arrow points at row 2 (third block).
        assert_eq!(arrow.expect("arrow exists").row, 2);
    }

    #[rstest::rstest]
    fn excluded_entries_produce_no_blocks() {
        // Given a history with Actor and Thinking entries mixed in.
        let mut state = AppState::default();
        state
            .active_session_mut()
            .push_entry(ChatEntry::user("a"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::actor("bash", "output"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::thinking("reasoning"));
        state
            .active_session_mut()
            .push_entry(ChatEntry::assistant("b"));

        // When rendering.
        let (arrow, rows) = render_to_buffer(&state, 1, 10);

        // Then only 2 blocks (user and assistant), arrow at row 1.
        let block_count = rows
            .iter()
            .filter(|r| r.contains('\u{2588}'))
            .count();
        assert_eq!(block_count, 2);
        assert_eq!(arrow.expect("arrow exists").row, 1);
    }

    #[rstest::rstest]
    fn ignored_entry_uses_darkened_color() {
        // Given an ignored user entry.
        let mut state = AppState::default();
        let entry = ChatEntry::user("old").with_ignored(true);
        state.active_session_mut().push_entry(entry);

        // When rendering.
        let (_arrow, rows) = render_to_buffer(&state, 1, 10);

        // Then a block is rendered (not empty).
        assert!(rows[0].contains('\u{2588}'));
        // The color is darkened cyan — we can't easily check the exact color
        // in a buffer test, but we verify the block exists.
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

        // Then arrow row is within viewport bounds.
        let arrow = arrow.expect("arrow exists");
        assert!(arrow.row < 5);
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
}
