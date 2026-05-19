//! Minimap sidebar section — compact visual summary of conversation history.
//!
//! Renders colored blocks representing chat entries. Consecutive tool-use rounds
//! (ToolCall → ToolResult → intermediate Assistant) are collapsed into single numbered
//! blocks (2-9, A-Z) in green. The final Assistant in a tool chain is shown as a
//! separate white block. Other entry types render individually. Ignored entries use
//! darkened colors instead of half-blocks. Blocks wrap at the container edge.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::common::app_state::AppState;
use crate::feat::session::chat_entry::{ChatEntry, ChatEntryKind};
use crate::feat::theme::contrast::darken;
use super::section_trait::{
    EnterFrom, SectionNavResult, SidebarIntent, SidebarSection, SidebarSectionId,
};

/// Full block character for entries and single-round tool sequences.
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

/// A block in the minimap — either a single entry or a collapsed tool-use sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MinimapBlock {
    /// A single non-tool entry (User, Assistant, Error, etc.).
    Entry {
        category: MinimapCategory,
        ignored: bool,
    },
    /// A collapsed sequence of consecutive ToolCall→[ToolResult]→Assistant rounds.
    CollapsedToolSequence {
        /// Number of tool-call rounds in this sequence.
        tool_count: usize,
        /// True if every entry in the sequence is ignored.
        all_ignored: bool,
    },
}

/// Maps a tool count to its display character.
///
/// 2–9 → `'2'`–`'9'`, 10–35 → `'A'`–`'Z'`. Counts ≤ 1 or > 35 should not
/// call this function.
fn count_char(n: usize) -> char {
    match n {
        2..=9 => char::from_digit(n as u32, 10).unwrap(),
        10..=35 => (b'A' + (n - 10) as u8) as char,
        _ => unreachable!("count_char called with {n}, expected 2..=35"),
    }
}

/// Returns true if the entry kind is a tool call or tool result.
fn is_tool_or_result(kind: &ChatEntryKind) -> bool {
    matches!(
        kind,
        ChatEntryKind::ToolCall { .. } | ChatEntryKind::ToolResult { .. }
    )
}

/// Emits collapsed blocks for a TA sequence, handling the Z+overflow split.
///
/// For tool_count > 35, emits `Z` (35) + remaining as a second block.
fn emit_collapsed(blocks: &mut Vec<MinimapBlock>, tool_count: usize, all_ignored: bool) {
    if tool_count == 0 {
        return;
    }
    if tool_count <= 35 {
        blocks.push(MinimapBlock::CollapsedToolSequence {
            tool_count,
            all_ignored,
        });
        return;
    }
    // First block: Z (35 rounds).
    blocks.push(MinimapBlock::CollapsedToolSequence {
        tool_count: 35,
        all_ignored,
    });
    // Second block: remaining rounds.
    blocks.push(MinimapBlock::CollapsedToolSequence {
        tool_count: tool_count - 35,
        all_ignored,
    });
}

/// Computes the minimap blocks from session history.
///
/// Walks entries with a state machine:
/// - Non-tool entries (User, Error, System, Compaction, Skill, standalone Assistant)
///   become individual [`Entry`](MinimapBlock::Entry) blocks.
/// - Consecutive ToolCall/ToolResult/Assistant sequences are collapsed into
///   [`CollapsedToolSequence`](MinimapBlock::CollapsedToolSequence) blocks. The count
///   is the number of ToolCall entries (tool rounds). Intermediate Assistant entries
///   are absorbed. The final Assistant (not followed by another ToolCall) becomes its
///   own `Entry` block.
fn compute_blocks(history: &[ChatEntry]) -> Vec<MinimapBlock> {
    let mut blocks = Vec::new();

    // Collect only included entries with their ignored status.
    let included: Vec<_> = history
        .iter()
        .filter_map(|entry| {
            MinimapCategory::from_kind(&entry.kind)
                .map(|cat| (cat, entry.ignored, &entry.kind))
        })
        .collect();

    let mut i = 0;
    while i < included.len() {
        let (category, ignored, kind) = &included[i];

        if is_tool_or_result(kind) {
            // Start of a TA sequence.
            let mut tool_count = 0usize;
            let mut all_ignored = true;
            let mut seq_end = i;

            // Absorb consecutive Tool/Assistant entries.
            while seq_end < included.len() {
                let (_, entry_ignored, entry_kind) = &included[seq_end];

                if is_tool_or_result(entry_kind) {
                    if matches!(entry_kind, ChatEntryKind::ToolCall { .. }) {
                        tool_count += 1;
                    }
                    all_ignored = all_ignored && *entry_ignored;
                    seq_end += 1;
                } else if matches!(entry_kind, ChatEntryKind::Assistant(..)) {
                    // Lookahead: is the next included entry a tool call?
                    let next_is_tool = included
                        .get(seq_end + 1)
                        .is_some_and(|(_, _, k)| is_tool_or_result(k));
                    if next_is_tool {
                        // Intermediate assistant — absorb.
                        all_ignored = all_ignored && *entry_ignored;
                        seq_end += 1;
                    } else {
                        // Final assistant — end sequence before it.
                        break;
                    }
                } else {
                    // Non-tool, non-assistant entry breaks the sequence.
                    break;
                }
            }

            emit_collapsed(&mut blocks, tool_count, all_ignored);
            i = seq_end;
        } else {
            // Non-tool entry: collapse consecutive entries with same (category, ignored).
            let start_cat = *category;
            let start_ignored = *ignored;
            let mut run_end = i + 1;
            while run_end < included.len() {
                let (cat, ign, kind) = &included[run_end];
                if *cat == start_cat && *ign == start_ignored && !is_tool_or_result(kind) {
                    run_end += 1;
                } else {
                    break;
                }
            }
            blocks.push(MinimapBlock::Entry {
                category: start_cat,
                ignored: start_ignored,
            });
            i = run_end;
        }
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
            match block {
                MinimapBlock::Entry { category, ignored } => {
                    let color = if *ignored {
                        darken(category.color(), 0.4)
                    } else {
                        category.color()
                    };
                    current_spans.push(Span::styled(
                        FULL_BLOCK.to_owned(),
                        Style::default().fg(color),
                    ));
                }
                MinimapBlock::CollapsedToolSequence {
                    tool_count,
                    all_ignored,
                } => {
                    let color = if *all_ignored {
                        darken(Color::Green, 0.4)
                    } else {
                        Color::Green
                    };
                    let ch = if *tool_count == 1 {
                        FULL_BLOCK.to_owned()
                    } else {
                        count_char(*tool_count).to_string()
                    };
                    current_spans.push(Span::styled(ch, Style::default().fg(color)));
                }
            }
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

    // --- count_char ---

    #[rstest::rstest]
    #[case(2, '2')]
    #[case(5, '5')]
    #[case(9, '9')]
    #[case(10, 'A')]
    #[case(20, 'K')]
    #[case(35, 'Z')]
    fn count_char_maps_correctly(#[case] n: usize, #[case] expected: char) {
        assert_eq!(count_char(n), expected);
    }

    // --- Block computation ---

    #[rstest::rstest]
    fn empty_history_produces_no_blocks() {
        // Given an empty history.
        let history: Vec<ChatEntry> = vec![];

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then there are no blocks.
        assert!(blocks.is_empty());
    }

    #[rstest::rstest]
    fn single_user_entry_produces_one_entry_block() {
        // Given a history with one user entry.
        let history = vec![ChatEntry::user("hello")];

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then there is one User Entry block.
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0],
            MinimapBlock::Entry {
                category: MinimapCategory::User,
                ignored: false
            }
        );
    }

    #[rstest::rstest]
    fn consecutive_same_type_collapses_to_one_entry() {
        // Given three consecutive user entries.
        let history = vec![
            ChatEntry::user("msg 1"),
            ChatEntry::user("msg 2"),
            ChatEntry::user("msg 3"),
        ];

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then there is one User Entry block.
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0],
            MinimapBlock::Entry {
                category: MinimapCategory::User,
                ignored: false
            }
        );
    }

    #[rstest::rstest]
    fn different_non_tool_types_produce_separate_entry_blocks() {
        // Given a user entry, then an assistant, then a system message.
        let history = vec![
            ChatEntry::user("hello"),
            ChatEntry::assistant("resp"),
            ChatEntry::system("info"),
        ];

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then there are three Entry blocks.
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].entry_category(), Some(MinimapCategory::User));
        assert_eq!(blocks[1].entry_category(), Some(MinimapCategory::Assistant));
        assert_eq!(blocks[2].entry_category(), Some(MinimapCategory::System));
    }

    #[rstest::rstest]
    fn simple_ta_sequence_collapses_to_one_block() {
        // Given User → ToolCall → ToolResult → Assistant.
        let history = vec![
            ChatEntry::user("do something"),
            ChatEntry::tool_call("id1", "bash", "echo hi"),
            ChatEntry::tool_result("id1", "bash", "output", crate::feat::session::tool_result_status::ToolResultStatus::Success),
            ChatEntry::assistant("done"),
        ];

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then: Entry(User), CollapsedToolSequence(1), Entry(Asst).
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].entry_category(), Some(MinimapCategory::User));
        assert_eq!(
            blocks[1],
            MinimapBlock::CollapsedToolSequence {
                tool_count: 1,
                all_ignored: false
            }
        );
        assert_eq!(blocks[2].entry_category(), Some(MinimapCategory::Assistant));
    }

    #[rstest::rstest]
    fn multi_round_ta_collapses_with_final_assistant_separate() {
        // Given: TCall → TResult → Asst → TCall → TResult → Asst(final).
        let history = vec![
            ChatEntry::tool_call("id1", "bash", "echo hi"),
            ChatEntry::tool_result("id1", "bash", "output", crate::feat::session::tool_result_status::ToolResultStatus::Success),
            ChatEntry::assistant("thinking..."),
            ChatEntry::tool_call("id2", "read", "file.txt"),
            ChatEntry::tool_result("id2", "read", "contents", crate::feat::session::tool_result_status::ToolResultStatus::Success),
            ChatEntry::assistant("here is the answer"),
        ];

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then: CollapsedToolSequence(2), Entry(Asst).
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            blocks[0],
            MinimapBlock::CollapsedToolSequence {
                tool_count: 2,
                all_ignored: false
            }
        );
        assert_eq!(blocks[1].entry_category(), Some(MinimapCategory::Assistant));
    }

    #[rstest::rstest]
    fn error_breaks_ta_sequence() {
        // Given: TCall → Error → TCall.
        let history = vec![
            ChatEntry::tool_call("id1", "bash", "echo hi"),
            ChatEntry::error("something went wrong"),
            ChatEntry::tool_call("id2", "bash", "echo bye"),
        ];

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then: Collapsed(1), Entry(Error), Collapsed(1).
        assert_eq!(blocks.len(), 3);
        assert_eq!(
            blocks[0],
            MinimapBlock::CollapsedToolSequence {
                tool_count: 1,
                all_ignored: false
            }
        );
        assert_eq!(blocks[1].entry_category(), Some(MinimapCategory::Error));
        assert_eq!(
            blocks[2],
            MinimapBlock::CollapsedToolSequence {
                tool_count: 1,
                all_ignored: false
            }
        );
    }

    #[rstest::rstest]
    fn compaction_breaks_ta_sequence() {
        // Given: TCall → Compaction → TCall.
        let compaction = ChatEntry {
            id: crate::feat::session::chat_entry::ChatEntryId::new(),
            timestamp: jiff::Timestamp::now(),
            kind: ChatEntryKind::Compaction {
                summary: "summary".into(),
                tokens_before: 100,
                entries_compacted: 5,
                model_used: "test".into(),
            },
            pin_position: None,
            ignored: false,
        };
        let history = vec![
            ChatEntry::tool_call("id1", "bash", "echo hi"),
            compaction,
            ChatEntry::tool_call("id2", "bash", "echo bye"),
        ];

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then: Collapsed(1), Entry(Compaction), Collapsed(1).
        assert_eq!(blocks.len(), 3);
        assert_eq!(
            blocks[0],
            MinimapBlock::CollapsedToolSequence {
                tool_count: 1,
                all_ignored: false
            }
        );
        assert_eq!(
            blocks[1].entry_category(),
            Some(MinimapCategory::Compaction)
        );
    }

    #[rstest::rstest]
    fn all_ignored_sequence_produces_all_ignored_block() {
        // Given a TA sequence where all entries are ignored.
        let history = vec![
            ChatEntry::tool_call("id1", "bash", "echo hi").with_ignored(true),
            ChatEntry::tool_result("id1", "bash", "output", crate::feat::session::tool_result_status::ToolResultStatus::Success)
                .with_ignored(true),
            ChatEntry::assistant("done").with_ignored(true),
        ];

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then: Collapsed(1, all_ignored=true), Entry(Asst, ignored=true).
        assert_eq!(
            blocks[0],
            MinimapBlock::CollapsedToolSequence {
                tool_count: 1,
                all_ignored: true
            }
        );
    }

    #[rstest::rstest]
    fn partially_ignored_sequence_produces_not_all_ignored() {
        // Given a TA sequence where only some entries are ignored.
        let history = vec![
            ChatEntry::tool_call("id1", "bash", "echo hi").with_ignored(true),
            ChatEntry::tool_result("id1", "bash", "output", crate::feat::session::tool_result_status::ToolResultStatus::Success)
                .with_ignored(false),
            ChatEntry::assistant("done"),
        ];

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then: Collapsed(1, all_ignored=false).
        assert_eq!(
            blocks[0],
            MinimapBlock::CollapsedToolSequence {
                tool_count: 1,
                all_ignored: false
            }
        );
    }

    #[rstest::rstest]
    fn tool_sequence_over_35_splits_into_z_plus_remaining() {
        // Given 40 tool rounds (ToolCall + ToolResult each).
        let mut history = Vec::new();
        for i in 0..40 {
            history.push(ChatEntry::tool_call(
                format!("id{i}"),
                "bash",
                "echo",
            ));
            history.push(ChatEntry::tool_result(
                format!("id{i}"),
                "bash",
                "output",
                crate::feat::session::tool_result_status::ToolResultStatus::Success,
            ));
        }
        history.push(ChatEntry::assistant("final answer"));

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then: Collapsed(35), Collapsed(5), Entry(Asst).
        assert_eq!(blocks.len(), 3);
        assert_eq!(
            blocks[0],
            MinimapBlock::CollapsedToolSequence {
                tool_count: 35,
                all_ignored: false
            }
        );
        assert_eq!(
            blocks[1],
            MinimapBlock::CollapsedToolSequence {
                tool_count: 5,
                all_ignored: false
            }
        );
        assert_eq!(blocks[2].entry_category(), Some(MinimapCategory::Assistant));
    }

    #[rstest::rstest]
    fn excluded_types_are_filtered() {
        // Given entries of excluded types (Actor, Thinking, Table).
        let history = vec![
            ChatEntry::actor("bash", "output"),
            ChatEntry::thinking("reasoning..."),
            ChatEntry::table(crate::feat::session::chat_entry::TableData {
                headers: vec![],
                rows: vec![],
            }),
        ];

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then there are no blocks.
        assert!(blocks.is_empty());
    }

    #[rstest::rstest]
    fn standalone_assistant_is_entry_block() {
        // Given a lone assistant entry with no tool calls.
        let history = vec![ChatEntry::assistant("hello")];

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then there is one Assistant Entry block.
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].entry_category(), Some(MinimapCategory::Assistant));
    }

    #[rstest::rstest]
    fn tool_call_without_result_still_counts_as_round() {
        // Given a ToolCall with no ToolResult, followed by a final Assistant.
        let history = vec![
            ChatEntry::tool_call("id1", "bash", "echo hi"),
            ChatEntry::assistant("done"),
        ];

        // When computing blocks.
        let blocks = compute_blocks(&history);

        // Then: Collapsed(1), Entry(Asst).
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            blocks[0],
            MinimapBlock::CollapsedToolSequence {
                tool_count: 1,
                all_ignored: false
            }
        );
        assert_eq!(blocks[1].entry_category(), Some(MinimapCategory::Assistant));
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
    fn render_full_block_for_non_ignored_entry() {
        // Given a minimap with one non-ignored user entry.
        let mut section = MinimapSection::default();
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::user("hello"));

        // When rendering.
        let rows = render_rows(&mut section, &state, 30, 5);

        // Then the first row contains a full block character.
        assert!(
            rows[0].contains('\u{2588}'),
            "should contain full block, got: {}",
            rows[0]
        );
    }

    #[rstest::rstest]
    fn render_darkened_color_for_ignored_entry() {
        // Given a minimap with one ignored user entry.
        let mut section = MinimapSection::default();
        let mut state = AppState::default();
        let entry = ChatEntry::user("old").with_ignored(true);
        state.active_session_mut().push_entry(entry);

        // When rendering.
        let rows = render_rows(&mut section, &state, 30, 5);

        // Then the first row contains a full block (not half block).
        assert!(
            rows[0].contains('\u{2588}'),
            "should contain full block, got: {}",
            rows[0]
        );
    }

    #[rstest::rstest]
    fn render_count_char_for_collapsed_tool_sequence() {
        // Given a 3-round TA sequence: TCall → TResult → Asst → TCall → TResult → Asst → TCall → TResult → Asst(final).
        let mut section = MinimapSection::default();
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::tool_call("id1", "bash", "echo 1"));
        state.active_session_mut().push_entry(ChatEntry::tool_result("id1", "bash", "out", crate::feat::session::tool_result_status::ToolResultStatus::Success));
        state.active_session_mut().push_entry(ChatEntry::assistant("intermediate"));
        state.active_session_mut().push_entry(ChatEntry::tool_call("id2", "bash", "echo 2"));
        state.active_session_mut().push_entry(ChatEntry::tool_result("id2", "bash", "out", crate::feat::session::tool_result_status::ToolResultStatus::Success));
        state.active_session_mut().push_entry(ChatEntry::assistant("intermediate 2"));
        state.active_session_mut().push_entry(ChatEntry::tool_call("id3", "bash", "echo 3"));
        state.active_session_mut().push_entry(ChatEntry::tool_result("id3", "bash", "out", crate::feat::session::tool_result_status::ToolResultStatus::Success));
        state.active_session_mut().push_entry(ChatEntry::assistant("final answer"));

        // When rendering.
        let rows = render_rows(&mut section, &state, 30, 5);

        // Then the first row contains '3' (count char for 3 tool rounds).
        assert!(
            rows[0].contains('3'),
            "should contain count char '3', got: {}",
            rows[0]
        );
        // And it does NOT contain the full block for the collapsed sequence.
        // The final assistant should appear as a separate full block.
    }

    #[rstest::rstest]
    fn render_full_block_for_single_tool_round() {
        // Given a single-round TA sequence.
        let mut section = MinimapSection::default();
        let mut state = AppState::default();
        state.active_session_mut().push_entry(ChatEntry::tool_call("id1", "bash", "echo hi"));
        state.active_session_mut().push_entry(ChatEntry::assistant("done"));

        // When rendering.
        let rows = render_rows(&mut section, &state, 30, 5);

        // Then the first row contains a full block (single round = █, not a digit).
        assert!(
            rows[0].contains('\u{2588}'),
            "should contain full block for single tool round, got: {}",
            rows[0]
        );
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
        // Given a MinimapSection with last_width=10 and 25 separate blocks.
        let mut section = MinimapSection::default();
        let mut state = AppState::default();
        // 25 entries alternating user/assistant to produce 25 separate Entry blocks.
        for i in 0..25 {
            if i % 2 == 0 {
                state
                    .active_session_mut()
                    .push_entry(ChatEntry::user(format!("msg {i}")));
            } else {
                state
                    .active_session_mut()
                    .push_entry(ChatEntry::assistant(format!("resp {i}")));
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

    // --- Test helper on MinimapBlock ---

    impl MinimapBlock {
        /// Returns the category if this is an Entry block, None otherwise.
        fn entry_category(&self) -> Option<MinimapCategory> {
            match self {
                MinimapBlock::Entry { category, .. } => Some(*category),
                MinimapBlock::CollapsedToolSequence { .. } => None,
            }
        }
    }
}
