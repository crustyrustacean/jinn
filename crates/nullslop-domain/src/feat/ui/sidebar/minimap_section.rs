//! Minimap sidebar section — compact visual summary of conversation history.
//!
//! Renders colored blocks (█) representing sequences of same-type chat entries.
//! Entries marked `ignored` use a half block (▄) instead. Blocks are displayed
//! horizontally and wrap to the next line at the container edge. No header,
//! borders, or padding — blocks go edge-to-edge.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::common::app_state::AppState;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind};
use super::section_trait::{
    EnterFrom, SectionNavResult, SidebarIntent, SidebarSection, SidebarSectionId,
};

/// Full block character for non-ignored entries.
const FULL_BLOCK: &str = "\u{2588}";
/// Half block (lower) character for ignored entries.
const HALF_BLOCK: &str = "\u{2584}";

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
    /// System and info messages — yellow.
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
            ChatEntryKind::System(..) | ChatEntryKind::Info(..) => Some(Self::System),
            ChatEntryKind::Skill { .. } => Some(Self::Skill),
            // Excluded: Actor, Thinking, Table.
            ChatEntryKind::Actor { .. }
            | ChatEntryKind::Thinking(..)
            | ChatEntryKind::Table(..) => None,
        }
    }
}

/// A single block in the minimap, representing a run of same-category entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MinimapBlock {
    category: MinimapCategory,
    is_ignored: bool,
}

/// Computes the minimap blocks from session history.
///
/// Iterates entries, filters to included types, and collapses consecutive
/// entries with the same `(category, is_ignored)` into a single block.
fn compute_blocks(history: &[ChatEntry]) -> Vec<MinimapBlock> {
    let mut blocks = Vec::new();
    for entry in history {
        let Some(category) = MinimapCategory::from_kind(&entry.kind) else {
            continue;
        };
        let block = MinimapBlock {
            category,
            is_ignored: entry.ignored,
        };
        if blocks.last() == Some(&block) {
            continue;
        }
        blocks.push(block);
    }
    blocks
}

/// Navigate within the minimap section.
///
/// Display-only — always returns `Exhausted` so sidebar focus moves to the
/// next section.
pub fn navigate(_intent: &SidebarIntent, _state: &mut AppState) -> SectionNavResult {
    SectionNavResult::Exhausted
}

/// Place the cursor on this section.
///
/// Display-only — no-op.
pub fn receive_cursor(_state: &mut AppState, _enter_from: EnterFrom) {}

/// The minimap sidebar section.
///
/// Renders a compact visual summary of the conversation history as colored
/// blocks. No header, borders, or padding — blocks fill the available width
/// and wrap to additional rows as needed.
#[derive(Debug)]
pub struct MinimapSection {
    /// The last known container width, used for `content_height` calculations.
    /// Updated on each `render` call. Default is a reasonable starting guess.
    last_width: u16,
}

impl Default for MinimapSection {
    fn default() -> Self {
        Self { last_width: 30 }
    }
}

impl SidebarSection for MinimapSection {
    fn id(&self) -> SidebarSectionId {
        SidebarSectionId::Minimap
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        // Update stored width for future content_height calls.
        self.last_width = area.width;

        let blocks = compute_blocks(state.active_session().history());
        if blocks.is_empty() {
            return;
        }

        let width = area.width as usize;
        if width == 0 {
            return;
        }

        // Build lines of blocks, wrapping at container width.
        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut current_spans: Vec<Span<'static>> = Vec::new();

        for block in &blocks {
            let ch = if block.is_ignored {
                HALF_BLOCK
            } else {
                FULL_BLOCK
            };
            current_spans.push(Span::styled(
                ch.to_owned(),
                Style::default().fg(block.category.color()),
            ));

            if current_spans.len() >= width {
                lines.push(Line::from(std::mem::take(&mut current_spans)));
            }
        }

        // Flush remaining spans.
        if !current_spans.is_empty() {
            lines.push(Line::from(current_spans));
        }

        let widget = Paragraph::new(lines);
        frame.render_widget(widget, area);
    }

    fn content_height(&self, state: &AppState) -> u16 {
        let blocks = compute_blocks(state.active_session().history());
        if blocks.is_empty() {
            return 0;
        }
        let width = self.last_width as usize;
        if width == 0 {
            return 0;
        }
        (blocks.len().div_ceil(width)) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::app_state::AppState;
    use crate::feat::session::chat_entry::ChatEntry;

    // Helper to build history from a list of entries.
    fn history_with(entries: Vec<ChatEntry>) -> Vec<ChatEntry> {
        entries
    }

    // --- Block computation ---

    #[rstest::rstest]
    fn empty_history_produces_no_blocks() {
        // Given an empty history.
        let history = history_with(vec![]);

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then there are no blocks.
        assert!(blocks.is_empty());
    }

    #[rstest::rstest]
    fn single_entry_produces_one_block() {
        // Given a history with one user entry.
        let history = history_with(vec![ChatEntry::user("hello")]);

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then there is one User block, not ignored.
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].category, MinimapCategory::User);
        assert!(!blocks[0].is_ignored);
    }

    #[rstest::rstest]
    fn consecutive_same_type_collapses() {
        // Given three consecutive user entries.
        let history = history_with(vec![
            ChatEntry::user("msg 1"),
            ChatEntry::user("msg 2"),
            ChatEntry::user("msg 3"),
        ]);

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then there is one collapsed User block.
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].category, MinimapCategory::User);
    }

    #[rstest::rstest]
    fn different_type_produces_separate_blocks() {
        // Given a user entry, then a tool call, then another user entry.
        let history = history_with(vec![
            ChatEntry::user("hello"),
            ChatEntry::tool_call("id1", "bash", "echo hi"),
            ChatEntry::user("world"),
        ]);

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then there are three blocks: User, Tool, User.
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].category, MinimapCategory::User);
        assert_eq!(blocks[1].category, MinimapCategory::Tool);
        assert_eq!(blocks[2].category, MinimapCategory::User);
    }

    #[rstest::rstest]
    fn ignored_entries_produce_separate_blocks() {
        // Given two user entries, one ignored and one not.
        let mut entry1 = ChatEntry::user("old");
        entry1.ignored = true;
        let entry2 = ChatEntry::user("new");

        let history = history_with(vec![entry1, entry2]);

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then there are two blocks (same category, different ignored status).
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].is_ignored);
        assert!(!blocks[1].is_ignored);
    }

    #[rstest::rstest]
    fn excluded_types_are_filtered() {
        // Given entries of excluded types (Actor, Thinking, Table).
        let history = history_with(vec![
            ChatEntry::actor("bash", "output"),
            ChatEntry::thinking("reasoning..."),
            ChatEntry::table(crate::feat::session::chat_entry::TableData {
                headers: vec![],
                rows: vec![],
            }),
        ]);

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then there are no blocks.
        assert!(blocks.is_empty());
    }

    // --- Rendering ---

    fn render_rows(
        section: &mut MinimapSection,
        state: &AppState,
        width: u16,
        height: u16,
    ) -> Vec<String> {
        let (mut terminal, area) = nullslop_testutil::setup_term(width, height);
        terminal
            .draw(|frame| {
                section.render(frame, area, state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        nullslop_testutil::buffer_rows(buffer, width, height)
    }

    #[rstest::rstest]
    fn render_full_block_for_non_ignored() {
        // Given a minimap with one non-ignored user entry.
        let mut section = MinimapSection::default();
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("hello"));

        // When rendering.
        let rows = render_rows(&mut section, &state, 30, 5);

        // Then the first row contains a full block character.
        assert!(rows[0].contains('\u{2588}'), "should contain full block, got: {}", rows[0]);
    }

    #[rstest::rstest]
    fn render_half_block_for_ignored() {
        // Given a minimap with one ignored user entry.
        let mut section = MinimapSection::default();
        let mut state = AppState::default();
        let mut entry = ChatEntry::user("old");
        entry.ignored = true;
        state.active_session_mut().push_entry(entry);

        // When rendering.
        let rows = render_rows(&mut section, &state, 30, 5);

        // Then the first row contains a half block character.
        assert!(rows[0].contains('\u{2584}'), "should contain half block, got: {}", rows[0]);
    }

    #[rstest::rstest]
    fn render_wraps_at_container_edge() {
        // Given a minimap with 5 different-type blocks in a 3-wide container.
        let mut section = MinimapSection::default();
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("msg"));
        state.active_session_mut().push_entry(ChatEntry::assistant("resp"));
        state.active_session_mut().push_entry(ChatEntry::system("info"));
        state.active_session_mut().push_entry(ChatEntry::error("err"));
        state.active_session_mut().push_entry(ChatEntry::user("msg2"));

        // When rendering in a 3-wide container.
        let rows = render_rows(&mut section, &state, 3, 5);

        // Then row 0 has 3 blocks, row 1 has 2 blocks.
        let row0_count = rows[0].chars().filter(|c| *c == '\u{2588}').count();
        let row1_count = rows[1].chars().filter(|c| *c == '\u{2588}').count();
        assert_eq!(row0_count, 3, "row 0 should have 3 blocks");
        assert_eq!(row1_count, 2, "row 1 should have 2 blocks");
    }

    // --- Section integration ---

    #[rstest::rstest]
    fn content_height_is_zero_for_empty_history() {
        // Given a MinimapSection with empty history.
        let section = MinimapSection::default();
        let state = AppState::default();

        // When asking for content height.
        let height = section.content_height(&state);

        // Then it is 0.
        assert_eq!(height, 0);
    }

    #[rstest::rstest]
    fn content_height_is_correct_for_known_blocks_and_width() {
        // Given a MinimapSection with last_width=10 and 25 blocks worth of entries.
        let mut section = MinimapSection::default();
        let mut state = AppState::default();
        // 25 entries alternating type to produce 25 separate blocks.
        for i in 0..25 {
            if i % 2 == 0 {
                state.active_session_mut().push_entry(ChatEntry::user(format!("msg {i}")));
            } else {
                state.active_session_mut().push_entry(ChatEntry::assistant(format!("resp {i}")));
            }
        }
        // Set last_width by simulating render area.
        section.last_width = 10;

        // When asking for content height.
        let height = section.content_height(&state);

        // Then it is ceil(25 / 10) = 3.
        assert_eq!(height, 3);
    }

    #[rstest::rstest]
    fn section_id_is_minimap() {
        // Given a MinimapSection.
        let section = MinimapSection::default();

        // When asking for its ID.
        // Then it returns Minimap.
        assert_eq!(section.id(), SidebarSectionId::Minimap);
    }
}
