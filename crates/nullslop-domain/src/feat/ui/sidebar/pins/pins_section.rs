//! [`PinsSection`] — the pinned entries sidebar section.
//!
//! Implements [`SidebarSection`] for pinned context entries.
//! Also provides standalone handler functions that the `IntentHandler`
//! calls during Phase 2 (while intents are still `PinnedPanel*` variants).
//! Phase 3 will migrate intent handling into the section directly.

use crate::common::app_state::AppState;
use crate::common::app_state::pin_sort_key;
use crate::feat::context::protocol::command::{PinChatEntry, UnpinChatEntry};
use crate::feat::ui::sidebar::section_trait::{
    SidebarIntent, SidebarSection, SidebarSectionConfig, SidebarSectionId, SidebarSectionResult,
};
use crate::protocol::{ChatEntryId, ChatEntryKind, Command, IntentResult, PinPosition, SessionId};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::validator;

/// The pinned entries sidebar section.
///
/// Renders pinned context entries with position badges and selection highlighting.
/// Handles navigation (up/down) within the pins list and delegates boundary
/// crossings to the sidebar container.
#[derive(Debug)]
pub struct PinsSection;

impl SidebarSection for PinsSection {
    fn id(&self) -> SidebarSectionId {
        SidebarSectionId::Pins
    }

    fn handle_intent(
        &mut self,
        intent: &SidebarIntent,
        state: &mut AppState,
        config: &SidebarSectionConfig,
    ) -> SidebarSectionResult {
        match intent {
            SidebarIntent::MoveDown => {
                let sorted_ids = state.sorted_pinned_ids();
                if sorted_ids.is_empty() {
                    return SidebarSectionResult::UnhandledDown;
                }
                let current = state.frontend.pins.selection_index(&sorted_ids);
                if current >= sorted_ids.len() - 1 {
                    if config.has_below {
                        state.frontend.pins.clear_selection();
                        return SidebarSectionResult::UnhandledDown;
                    }
                    return SidebarSectionResult::Handled; // sticky at bottom
                }
                state.frontend.pins.select_next(&sorted_ids);
                SidebarSectionResult::Handled
            }
            SidebarIntent::MoveUp => {
                let sorted_ids = state.sorted_pinned_ids();
                if sorted_ids.is_empty() {
                    return SidebarSectionResult::UnhandledUp;
                }
                let current = state.frontend.pins.selection_index(&sorted_ids);
                if current == 0 {
                    if config.has_above {
                        state.frontend.pins.clear_selection();
                        return SidebarSectionResult::UnhandledUp;
                    }
                    return SidebarSectionResult::Handled; // sticky at top
                }
                state.frontend.pins.select_prev(&sorted_ids);
                SidebarSectionResult::Handled
            }
            SidebarIntent::Action(_) => SidebarSectionResult::Handled,
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        let sorted_ids = state.sorted_pinned_ids();
        let mut pinned = state.active_session().pinned_entries();
        // Sort to match sorted_ids order (TOP → REL → BOT, stable by history).
        pinned.sort_by_key(|entry| pin_sort_key(entry.pin_position));
        if pinned.is_empty() {
            render_no_entries(frame, area);
            return;
        }

        let selected_index = state.frontend.pins.selection_index(&sorted_ids);
        let lines = build_entry_list(&pinned, selected_index, area.width);

        let total_lines = lines.len() as u16;
        let max_offset = total_lines.saturating_sub(area.height);
        let scroll_offset = max_offset;

        let widget = Paragraph::new(lines)
            .block(Block::default().borders(Borders::NONE))
            .scroll((scroll_offset, 0));
        frame.render_widget(widget, area);
    }

    fn content_height(&self, state: &AppState) -> u16 {
        let count = state.active_session().pinned_entries().len();
        if count == 0 {
            return 1; // "No pinned entries" message
        }
        // Header line + blank + (entry line + blank) * count - last blank
        (2 + count * 2).saturating_sub(1) as u16
    }
}

// ---------------------------------------------------------------------------
// Bridge handler functions (Phase 2 — temporary until Phase 3 migrates intents)
// ---------------------------------------------------------------------------

/// Handles `PinnedPanelToggle` — sets the toggle signal.
pub fn handle_toggle(state: &mut AppState) -> IntentResult {
    state.frontend.tui_signals.pinned_pane_toggle = true;
    IntentResult::empty()
}

/// Handles `PinnedPanelOpen` — sets the open signal.
pub fn handle_open(state: &mut AppState) -> IntentResult {
    state.frontend.tui_signals.pinned_pane_open = true;
    IntentResult::empty()
}

/// Handles `PinnedPanelClose` — sets the close signal.
pub fn handle_close(state: &mut AppState) -> IntentResult {
    state.frontend.tui_signals.pinned_pane_close = true;
    IntentResult::empty()
}

/// Handles `PinnedPanelSelectDown` — moves selection to the next pinned entry.
pub fn handle_select_down(state: &mut AppState) -> IntentResult {
    let sorted_ids = state.sorted_pinned_ids();
    state.frontend.pins.select_next(&sorted_ids);
    IntentResult::empty()
}

/// Handles `PinnedPanelSelectUp` — moves selection to the previous pinned entry.
pub fn handle_select_up(state: &mut AppState) -> IntentResult {
    let sorted_ids = state.sorted_pinned_ids();
    state.frontend.pins.select_prev(&sorted_ids);
    IntentResult::empty()
}

/// Handles `PinnedPanelUnpin` — unpins the selected entry.
pub fn handle_unpin(state: &mut AppState) -> IntentResult {
    if validator::validate_unpin(state).is_err() {
        return IntentResult::empty();
    }

    if let Some((session_id, entry_id)) = resolve_selected_entry_id(state) {
        IntentResult::with_commands(vec![Command::UnpinChatEntry {
            payload: UnpinChatEntry {
                session_id,
                entry_id,
            },
        }])
    } else {
        IntentResult::empty()
    }
}

/// Handles `PinnedPanelPinTop/Bottom/Relative` — sets the selected entry's pin position.
pub fn handle_pin(state: &mut AppState, position: PinPosition) -> IntentResult {
    if validator::validate_pin_top(state).is_err() {
        return IntentResult::empty();
    }

    if let Some((session_id, entry_id)) = resolve_selected_entry_id(state) {
        IntentResult::with_commands(vec![Command::PinChatEntry {
            payload: PinChatEntry {
                session_id,
                entry_id,
                position,
            },
        }])
    } else {
        IntentResult::empty()
    }
}

/// Handles `PinnedPanelPinCycle` — rotates the selected entry's pin position.
pub fn handle_pin_cycle(state: &mut AppState) -> IntentResult {
    if validator::validate_pin_cycle(state).is_err() {
        return IntentResult::empty();
    }

    let sorted_ids = state.sorted_pinned_ids();
    let index = state.frontend.pins.selection_index(&sorted_ids);

    let mut pinned = state.active_session().pinned_entries();
    pinned.sort_by_key(|entry| pin_sort_key(entry.pin_position));

    let Some(entry) = pinned.get(index) else {
        return IntentResult::empty();
    };

    let current = entry.pin_position.unwrap_or(PinPosition::Relative);
    let next = cycle_position(current);
    let session_id = state.session.active_session.clone();
    let entry_id = entry.id.clone();

    IntentResult::with_commands(vec![Command::PinChatEntry {
        payload: PinChatEntry {
            session_id,
            entry_id,
            position: next,
        },
    }])
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolves the currently selected pinned entry to its session and entry IDs.
fn resolve_selected_entry_id(state: &AppState) -> Option<(SessionId, ChatEntryId)> {
    let sorted_ids = state.sorted_pinned_ids();
    let index = state.frontend.pins.selection_index(&sorted_ids);
    let session_id = state.session.active_session.clone();

    let mut pinned = state.active_session().pinned_entries();
    pinned.sort_by_key(|entry| pin_sort_key(entry.pin_position));

    let entry = pinned.get(index)?;
    Some((session_id, entry.id.clone()))
}

/// Cycles a pin position to the next value in the rotation: Top → Bottom → Relative → Top.
fn cycle_position(pos: PinPosition) -> PinPosition {
    match pos {
        PinPosition::Top => PinPosition::Bottom,
        PinPosition::Bottom => PinPosition::Relative,
        PinPosition::Relative => PinPosition::Top,
    }
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

/// Solid yellow full block used as the selection indicator.
const SELECTED_INDICATOR: &str = "\u{2588}\u{2588}";
/// Two spaces used as the unselected border.
const UNSELECTED_BORDER: &str = "  ";

/// Renders the "No pinned entries." placeholder.
fn render_no_entries(frame: &mut Frame<'_>, area: Rect) {
    let msg = Paragraph::new("No pinned entries.")
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::NONE));
    frame.render_widget(msg, area);
}

/// Returns the badge text and color for a pin position.
fn position_badge(position: PinPosition) -> (&'static str, Color) {
    match position {
        PinPosition::Top => ("[TOP]", Color::Cyan),
        PinPosition::Bottom => ("[BOT]", Color::Magenta),
        PinPosition::Relative => ("[REL]", Color::DarkGray),
    }
}

/// Returns the display prefix and truncated content for a chat entry kind.
fn entry_prefix_and_content(kind: &ChatEntryKind) -> (&'static str, String) {
    match kind {
        ChatEntryKind::User(text) => ("> ", truncate_str(text, 40)),
        ChatEntryKind::Assistant(text) => ("\u{2666} ", truncate_str(text, 40)),
        ChatEntryKind::System(text) => ("\u{2699} ", truncate_str(text, 40)),
        ChatEntryKind::Error(text) => ("\u{26a0} ", truncate_str(text, 40)),
        ChatEntryKind::Actor { source, text } => {
            let content = format!("[{}] {}", source, truncate_str(text, 30));
            ("", content)
        }
        ChatEntryKind::ToolCall { name, .. } => {
            ("\u{2692} ", format!("{}(...)", truncate_str(name, 20)))
        }
        ChatEntryKind::ToolResult {
            name,
            content,
            success,
            ..
        } => {
            let icon = if *success { "\u{2705}" } else { "\u{274c}" };
            (
                "",
                format!(
                    "{} {}: {}",
                    icon,
                    truncate_str(name, 15),
                    truncate_str(content, 20)
                ),
            )
        }
        // Table entries are not shown in the pinned panel summary.
        ChatEntryKind::Table(_) => ("", String::new()),
    }
}

/// Truncates a string to the given max length, appending an ellipsis if needed.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_owned()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(1)).collect();
        format!("{truncated}\u{2026}")
    }
}

/// Builds the list of lines for the pinned entries panel.
#[expect(
    clippy::indexing_slicing,
    reason = "iterating with enumerate over pinned slice"
)]
fn build_entry_list(
    pinned: &[&crate::protocol::ChatEntry],
    selected_index: usize,
    _area_width: u16,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // Header
    lines.push(Line::from(vec![Span::styled(
        format!(" Pinned Context \u{2014} {}", pinned.len()),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    for (i, entry) in pinned.iter().enumerate() {
        let is_selected = i == selected_index;

        let border = if is_selected {
            Span::styled(SELECTED_INDICATOR, Style::default().fg(Color::Yellow))
        } else {
            Span::raw(UNSELECTED_BORDER)
        };

        let (badge_text, badge_color) =
            position_badge(entry.pin_position.unwrap_or(PinPosition::Relative));

        let (prefix, content) = entry_prefix_and_content(&entry.kind);

        let style = if is_selected {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };

        lines.push(Line::from(vec![
            border,
            Span::styled(format!(" {badge_text} "), Style::default().fg(badge_color)),
            Span::styled(format!("{prefix}{content}"), style),
        ]));

        if i < pinned.len() - 1 {
            lines.push(Line::from(""));
        }
    }

    lines
}

#[cfg(test)]
mod tests {
    use crate::common::app_state::AppState;
    use crate::protocol::{ChatEntry, Command, PinPosition};

    use super::*;

    fn state_with_pinned(count: usize) -> AppState {
        let mut state = AppState::default();
        let mut ids = vec![];
        for i in 0..count {
            let entry = ChatEntry::user(format!("entry {i}"));
            let entry_id = entry.id.clone();
            state.active_session_mut().push_entry(entry);
            ids.push(entry_id);
        }
        for id in &ids {
            state.active_session_mut().pin_entry(id, PinPosition::Top);
        }
        // Select the first pinned entry.
        if let Some(first_id) = ids.first() {
            state.frontend.pins.select_by_id(first_id.clone());
        }
        state
    }

    // --- Bridge handler tests ---

    #[rstest::rstest]
    fn toggle_sets_signal() {
        // Given a default state.
        let mut state = AppState::default();

        // When handling toggle.
        let result = handle_toggle(&mut state);

        // Then the toggle signal is set.
        assert!(state.frontend.tui_signals.pinned_pane_toggle);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn open_sets_signal() {
        // Given a default state.
        let mut state = AppState::default();

        // When handling open.
        let result = handle_open(&mut state);

        // Then the open signal is set.
        assert!(state.frontend.tui_signals.pinned_pane_open);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn close_sets_signal() {
        // Given a default state.
        let mut state = AppState::default();

        // When handling close.
        let result = handle_close(&mut state);

        // Then the close signal is set.
        assert!(state.frontend.tui_signals.pinned_pane_close);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn select_down_moves_selection() {
        // Given a state with 3 pinned entries.
        let mut state = state_with_pinned(3);

        // When handling select down.
        let result = handle_select_down(&mut state);

        // Then selection moved.
        let sorted_ids = state.sorted_pinned_ids();
        assert_eq!(state.frontend.pins.selected_id(), Some(&sorted_ids[1]));
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn select_up_moves_selection() {
        // Given a state with 3 pinned entries at index 1.
        let mut state = state_with_pinned(3);
        let sorted_ids = state.sorted_pinned_ids();
        state.frontend.pins.select_next(&sorted_ids);

        // When handling select up.
        let result = handle_select_up(&mut state);

        // Then selection moved back.
        let sorted_ids = state.sorted_pinned_ids();
        assert_eq!(state.frontend.pins.selected_id(), Some(&sorted_ids[0]));
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn unpin_returns_command() {
        // Given a state with pinned entries.
        let mut state = state_with_pinned(2);

        // When handling unpin.
        let result = handle_unpin(&mut state);

        // Then an UnpinChatEntry command is returned.
        assert!(
            result
                .commands
                .iter()
                .any(|c| matches!(c, Command::UnpinChatEntry { .. }))
        );
    }

    #[rstest::rstest]
    fn unpin_noop_when_empty() {
        // Given a state with no pinned entries.
        let mut state = AppState::default();

        // When handling unpin.
        let result = handle_unpin(&mut state);

        // Then no commands.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn pin_top_returns_command() {
        // Given a state with pinned entries.
        let mut state = state_with_pinned(1);

        // When handling pin top.
        let result = handle_pin(&mut state, PinPosition::Top);

        // Then a PinChatEntry command with Top is returned.
        let pin_cmd = result.commands.iter().find_map(|c| match c {
            Command::PinChatEntry { payload } => Some(payload.position),
            _ => None,
        });
        assert_eq!(pin_cmd, Some(PinPosition::Top));
    }

    #[rstest::rstest]
    fn pin_bottom_returns_command() {
        // Given a state with pinned entries.
        let mut state = state_with_pinned(1);

        // When handling pin bottom.
        let result = handle_pin(&mut state, PinPosition::Bottom);

        // Then a PinChatEntry command with Bottom is returned.
        let pin_cmd = result.commands.iter().find_map(|c| match c {
            Command::PinChatEntry { payload } => Some(payload.position),
            _ => None,
        });
        assert_eq!(pin_cmd, Some(PinPosition::Bottom));
    }

    #[rstest::rstest]
    fn pin_relative_returns_command() {
        // Given a state with pinned entries.
        let mut state = state_with_pinned(1);

        // When handling pin relative.
        let result = handle_pin(&mut state, PinPosition::Relative);

        // Then a PinChatEntry command with Relative is returned.
        let pin_cmd = result.commands.iter().find_map(|c| match c {
            Command::PinChatEntry { payload } => Some(payload.position),
            _ => None,
        });
        assert_eq!(pin_cmd, Some(PinPosition::Relative));
    }

    #[rstest::rstest]
    fn pin_cycle_rotates_top_to_bottom() {
        // Given a pinned entry at Top.
        let mut state = AppState::default();
        let entry = ChatEntry::user("entry");
        let entry_id = entry.id.clone();
        state.active_session_mut().push_entry(entry);
        state
            .active_session_mut()
            .pin_entry(&entry_id, PinPosition::Top);
        let sorted_ids = state.sorted_pinned_ids();
        state.frontend.pins.select_by_id(sorted_ids[0].clone());

        // When handling pin cycle.
        let result = handle_pin_cycle(&mut state);

        // Then a PinChatEntry command with Bottom is returned.
        let pin_cmd = result.commands.iter().find_map(|c| match c {
            Command::PinChatEntry { payload } => Some(payload.position),
            _ => None,
        });
        assert_eq!(pin_cmd, Some(PinPosition::Bottom));
    }

    #[rstest::rstest]
    fn pin_cycle_noop_when_empty() {
        // Given a state with no pinned entries.
        let mut state = AppState::default();

        // When handling pin cycle.
        let result = handle_pin_cycle(&mut state);

        // Then no commands.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn pin_top_noop_when_no_selection() {
        // Given a state with pinned entries but no selection.
        let mut state = AppState::default();
        let entry = ChatEntry::user("entry");
        let entry_id = entry.id.clone();
        state.active_session_mut().push_entry(entry);
        state
            .active_session_mut()
            .pin_entry(&entry_id, PinPosition::Top);
        // Don't select anything.

        // When handling pin top.
        let result = handle_pin(&mut state, PinPosition::Top);

        // Then no commands.
        assert!(result.commands.is_empty());
    }

    // --- SidebarSection tests ---

    #[rstest::rstest]
    fn section_id_is_pins() {
        // Given a PinsSection.
        let section = PinsSection;

        // When asking for its ID.
        // Then it returns Pins.
        assert_eq!(section.id(), SidebarSectionId::Pins);
    }

    #[rstest::rstest]
    fn content_height_is_one_when_empty() {
        // Given a PinsSection and state with no pinned entries.
        let section = PinsSection;
        let state = AppState::default();

        // When asking for content height.
        let height = section.content_height(&state);

        // Then it returns 1 (placeholder message).
        assert_eq!(height, 1);
    }

    #[rstest::rstest]
    fn content_height_matches_entry_count() {
        // Given a PinsSection and state with 3 pinned entries.
        let section = PinsSection;
        let state = state_with_pinned(3);

        // When asking for content height.
        let height = section.content_height(&state);

        // Then it returns header(1) + blank(1) + (entry(1) + blank(1)) * 3 - last blank(1) = 7.
        assert_eq!(height, 7);
    }

    // --- Rendering tests ---

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn setup_term(width: u16, height: u16) -> (Terminal<TestBackend>, Rect) {
        let backend = TestBackend::new(width, height);
        let terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, width, height);
        (terminal, area)
    }

    fn render_rows(
        section: &mut PinsSection,
        state: &AppState,
        width: u16,
        height: u16,
    ) -> Vec<String> {
        let (mut terminal, area) = setup_term(width, height);
        terminal
            .draw(|frame| {
                section.render(frame, area, state);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| {
                        buffer
                            .cell((x, y))
                            .map_or(" ", ratatui::buffer::Cell::symbol)
                    })
                    .collect()
            })
            .collect()
    }

    #[rstest::rstest]
    fn render_no_entries_shows_message() {
        let mut section = PinsSection;
        let state = AppState::default();
        let rows = render_rows(&mut section, &state, 40, 10);
        assert!(rows[0].contains("No pinned entries."));
    }

    #[rstest::rstest]
    fn render_shows_pinned_entries() {
        let mut section = PinsSection;
        let state = state_with_pinned(2);
        let rows = render_rows(&mut section, &state, 60, 20);
        let combined = rows.join("\n");
        assert!(
            combined.contains("pinned message 0") || combined.contains("entry 0"),
            "should contain first entry, got: {combined}"
        );
    }

    #[rstest::rstest]
    fn render_selected_entry_has_yellow_marker() {
        let mut section = PinsSection;
        let state = state_with_pinned(2);

        let (mut terminal, area) = setup_term(60, 20);
        terminal
            .draw(|frame| {
                section.render(frame, area, &state);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        // First entry at index 0 is selected by default.
        // No bordered block in section render — content starts at row 0.
        let cell0 = buffer.cell((0, 2)).expect("cell 0,2");
        assert_eq!(cell0.symbol(), "\u{2588}");
        assert_eq!(cell0.fg, Color::Yellow);
    }

    #[rstest::rstest]
    fn render_sorts_entries_by_position() {
        // Given entries pinned with BOT, TOP, REL positions (added in that order).
        let mut section = PinsSection;
        let mut state = AppState::default();

        let bot_entry = ChatEntry::user("bottom entry");
        let bot_id = bot_entry.id.clone();
        state.active_session_mut().push_entry(bot_entry);
        state
            .active_session_mut()
            .pin_entry(&bot_id, PinPosition::Bottom);

        let top_entry = ChatEntry::user("top entry");
        let top_id = top_entry.id.clone();
        state.active_session_mut().push_entry(top_entry);
        state
            .active_session_mut()
            .pin_entry(&top_id, PinPosition::Top);

        let rel_entry = ChatEntry::user("relative entry");
        let rel_id = rel_entry.id.clone();
        state.active_session_mut().push_entry(rel_entry);
        state
            .active_session_mut()
            .pin_entry(&rel_id, PinPosition::Relative);

        // When rendering.
        let rows = render_rows(&mut section, &state, 60, 20);
        let combined = rows.join("\n");

        // Then entries appear in TOP, REL, BOT order.
        let top_pos = combined
            .find("top entry")
            .expect("should contain top entry");
        let rel_pos = combined
            .find("relative entry")
            .expect("should contain relative entry");
        let bot_pos = combined
            .find("bottom entry")
            .expect("should contain bottom entry");
        assert!(top_pos < rel_pos, "TOP entry should appear before REL entry");
        assert!(
            rel_pos < bot_pos,
            "REL entry should appear before BOT entry"
        );
    }
}
