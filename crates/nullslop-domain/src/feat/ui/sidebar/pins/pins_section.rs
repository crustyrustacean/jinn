//! [`PinsSection`] — the pinned entries sidebar section.
//!
//! Implements [`SidebarSection`] for pinned context entries.
//! Also provides handler functions that the `IntentHandler` calls
//! for sidebar and pins intents.

use crate::common::app_state::AppState;
use crate::common::app_state::pin_sort_key;
use crate::feat::context::protocol::command::{PinChatEntry, UnpinChatEntry};
use crate::feat::theme::Theme;
use crate::feat::ui::sidebar::section_trait::{
    EnterFrom, SectionNavResult, SidebarIntent, SidebarSection, SidebarSectionId,
};
use crate::protocol::{
    ChatEntryId, ChatEntryKind, Command, IntentResult, PickerKind, PinPosition, SessionId,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// The pinned entries sidebar section.
///
/// Renders pinned context entries with position badges and selection highlighting.
/// Handles navigation (up/down) within the pins list and delegates boundary
/// crossings to the sidebar container.
#[derive(Debug)]
pub struct PinsSection;

/// Navigate within the pins section.
///
/// Moves the cursor within the pins list, or returns `Exhausted` when
/// at a boundary or when the list is empty. Does NOT modify cursor state
/// on exhaustion — the sidebar decides what to do.
pub fn navigate(intent: &SidebarIntent, state: &mut AppState) -> SectionNavResult {
    let sorted_ids = state.sorted_pinned_ids();
    if sorted_ids.is_empty() {
        return SectionNavResult::Exhausted;
    }
    match intent {
        SidebarIntent::MoveDown => {
            let current = state.frontend.pins.selection_index(&sorted_ids);
            if current >= sorted_ids.len() - 1 {
                return SectionNavResult::Exhausted;
            }
            state.frontend.pins.select_next(&sorted_ids);
            sync_chat_log_cursor(state);
            SectionNavResult::Moved
        }
        SidebarIntent::MoveUp => {
            let current = state.frontend.pins.selection_index(&sorted_ids);
            if current == 0 {
                return SectionNavResult::Exhausted;
            }
            state.frontend.pins.select_prev(&sorted_ids);
            sync_chat_log_cursor(state);
            SectionNavResult::Moved
        }
        SidebarIntent::Action(_) => SectionNavResult::Moved,
    }
}

/// Place the cursor on this section from a given direction.
pub fn receive_cursor(state: &mut AppState, enter_from: EnterFrom) {
    let sorted_ids = state.sorted_pinned_ids();
    match enter_from {
        EnterFrom::Top => {
            if let Some(first) = sorted_ids.first() {
                state.frontend.pins.select_by_id(first.clone());
            }
        }
        EnterFrom::Bottom => {
            if let Some(last) = sorted_ids.last() {
                state.frontend.pins.select_by_id(last.clone());
            }
        }
    }
    sync_chat_log_cursor(state);
}

impl SidebarSection for PinsSection {
    fn id(&self) -> SidebarSectionId {
        SidebarSectionId::Pins
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, state: &AppState) {
        let sorted_ids = state.sorted_pinned_ids();
        let mut pinned = state.active_session().pinned_entries();
        // Sort to match sorted_ids order (TOP → REL → BOT, stable by history).
        pinned.sort_by_key(|entry| pin_sort_key(entry.pin_position));

        let selected_index = if state.frontend.pins.selected_id().is_some() {
            state.frontend.pins.selection_index(&sorted_ids)
        } else {
            usize::MAX // No pin will match this index.
        };
        let lines = if pinned.is_empty() {
            vec![Line::from(vec![Span::styled(
                " Pinned Context \u{2014} 0",
                Style::default()
                    .fg(state.frontend.theme.primary_text)
                    .add_modifier(Modifier::BOLD),
            )])]
        } else {
            build_entry_list(
                &pinned,
                selected_index,
                area.width,
                state.frontend.scope_stack.is_sidebar(),
                &state.frontend.theme,
            )
        };

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
        // Hide the section entirely when there are no pins.
        if count == 0 {
            return 0;
        }
        // Header line + blank + (entry line + blank) * count - last blank + trailing gap(1)
        (2 + count * 2).saturating_sub(1) as u16 + 1
    }
}

// ---------------------------------------------------------------------------
// Intent handler functions (called by IntentHandler)
// ---------------------------------------------------------------------------

/// Handles `SidebarFocus` — enters sidebar scope.
///
/// Defaults focus to the Persona section (topmost section).
pub fn handle_sidebar_focus(state: &mut AppState) -> IntentResult {
    use crate::common::app_state::FocusScope;
    use crate::feat::ui::sidebar::section_trait::{EnterFrom, SidebarSectionId};

    state.frontend.scope_stack.push(FocusScope::Sidebar);

    // If a section already has cursor state, restore it.
    let has_existing_cursor = state.frontend.persona_section.cursor.is_some()
        || state.frontend.pins.selected_id().is_some()
        || state.frontend.sessions_section.selected_index.is_some();

    if !has_existing_cursor {
        // First entry — default to Persona at top.
        state.frontend.sidebar.focused_section = SidebarSectionId::Persona;
        crate::feat::ui::sidebar::persona_section::receive_cursor(state, EnterFrom::Top);
    }

    IntentResult::empty()
}

/// Handles `SidebarLeave` — pops back to previous scope.
///
/// If the session is busy, activates the cancel stream confirmation prompt
/// instead of immediately leaving.
pub fn handle_sidebar_leave(state: &mut AppState) -> IntentResult {
    if !state.active_session().is_idle() {
        // Session is busy — show cancel confirmation prompt.
        state.frontend.cancel_stream_prompt = true;
    }
    // Preserve cursor positions — they'll be restored on re-entry.
    state.frontend.scope_stack.pop();
    IntentResult::empty()
}

/// Handles `SidebarPersonaEdit` — opens the persona picker when persona section is focused.
///
/// No-op if the pins section is focused.
pub fn handle_sidebar_persona_edit(state: &mut AppState) -> IntentResult {
    use crate::feat::ui::sidebar::section_trait::SidebarSectionId;
    if state.frontend.sidebar.focused_section != SidebarSectionId::Persona {
        return IntentResult::empty();
    }
    crate::feat::picker::intent::handle_open_picker(state, PickerKind::Persona)
}

/// Handles `PinsUnpin`.
pub fn handle_pins_unpin(state: &mut AppState) -> IntentResult {
    if super::validator::validate_unpin(state).is_err() {
        return IntentResult::empty();
    }
    if let Some((session_id, entry_id)) = resolve_selected_entry_id(state) {
        IntentResult::with_commands(vec![Command::UnpinChatEntry(UnpinChatEntry {
            session_id,
            entry_id,
        })])
    } else {
        IntentResult::empty()
    }
}

/// Handles `PinsPinTop/Bottom/Relative`.
pub fn handle_pins_pin(state: &mut AppState, position: PinPosition) -> IntentResult {
    if super::validator::validate_pin(state).is_err() {
        return IntentResult::empty();
    }
    if let Some((session_id, entry_id)) = resolve_selected_entry_id(state) {
        IntentResult::with_commands(vec![Command::PinChatEntry(PinChatEntry {
            session_id,
            entry_id,
            position,
        })])
    } else {
        IntentResult::empty()
    }
}

/// Handles `PinsPinCycle`.
pub fn handle_pins_pin_cycle(state: &mut AppState) -> IntentResult {
    if super::validator::validate_pin_cycle(state).is_err() {
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
    IntentResult::with_commands(vec![Command::PinChatEntry(PinChatEntry {
        session_id,
        entry_id,
        position: next,
    })])
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Sync the chat log cursor to the currently selected pinned entry.
///
/// When a pinned entry is selected in the sidebar, this sets the chat log's
/// `selected_entry_index` to the history index of that pinned entry so the
/// renderer scrolls to show it.
fn sync_chat_log_cursor(state: &mut AppState) {
    let Some(pinned_id) = state.frontend.pins.selected_id().cloned() else {
        return;
    };
    let history_index = state
        .active_session()
        .history()
        .iter()
        .position(|e| e.id == pinned_id);
    if let Some(index) = history_index {
        state.active_session_mut().set_selected_entry_index(index);
    }
}
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
const SELECTED_INDICATOR: &str = "\u{2588}";
/// One space used as the unselected border.
const UNSELECTED_BORDER: &str = " ";

/// Builds the list of lines for the pinned entries panel.
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
        ChatEntryKind::User { display, .. } => ("> ", truncate_str(display, 40)),
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
        // Thinking entries are not shown in the pinned panel summary.
        ChatEntryKind::Thinking(text) => ("", truncate_str(text, 40)),
        ChatEntryKind::Skill { name, .. } => ("\u{1f4cb} ", truncate_str(name, 40)),
        ChatEntryKind::Info(text) => ("\u{2139} ", truncate_str(text, 40)),
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
fn build_entry_list(
    pinned: &[&crate::protocol::ChatEntry],
    selected_index: usize,
    _area_width: u16,
    sidebar_focused: bool,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // Header
    lines.push(Line::from(vec![Span::styled(
        format!(" Pinned Context \u{2014} {}", pinned.len()),
        Style::default()
            .fg(theme.primary_text)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    for (i, entry) in pinned.iter().enumerate() {
        let is_selected = i == selected_index;

        let indicator_color = if sidebar_focused {
            theme.focus_accent
        } else {
            theme.border_unfocused
        };
        let border = if is_selected {
            Span::styled(SELECTED_INDICATOR, Style::default().fg(indicator_color))
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
    use crate::common::app_state::{AppState, FocusScope};
    use crate::feat::ui::sidebar::section_trait::SidebarSectionId;
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

    // --- Sidebar handler tests ---

    #[rstest::rstest]
    fn sidebar_focus_pushes_sidebar_scope() {
        // Given a default state.
        let mut state = AppState::default();

        // When handling sidebar focus.
        let result = handle_sidebar_focus(&mut state);

        // Then Sidebar is on top of the scope stack.
        assert!(state.frontend.scope_stack.is_sidebar());
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn sidebar_focus_defaults_to_persona_section() {
        // Given a default state.
        let mut state = AppState::default();

        // When handling sidebar focus.
        handle_sidebar_focus(&mut state);

        // Then the focused section is Persona.
        assert_eq!(
            state.frontend.sidebar.focused_section,
            SidebarSectionId::Persona
        );
    }

    #[rstest::rstest]
    fn sidebar_focus_does_not_select_pins() {
        // Given a state with 3 pinned entries and no selection.
        let mut state = AppState::default();
        let _ids: Vec<_> = (0..3)
            .map(|i| {
                let entry = ChatEntry::user(format!("entry {i}"));
                let id = entry.id.clone();
                state.active_session_mut().push_entry(entry);
                state.active_session_mut().pin_entry(&id, PinPosition::Top);
                id
            })
            .collect();
        assert!(state.frontend.pins.selected_id().is_none());

        // When handling sidebar focus.
        handle_sidebar_focus(&mut state);

        // Then no pin is selected (persona section is focused).
        assert!(state.frontend.pins.selected_id().is_none());
        // And the persona section has received the cursor.
        assert_eq!(state.frontend.persona_section.cursor, Some(0));
    }

    #[rstest::rstest]
    fn sidebar_leave_pops_scope_stack() {
        // Given a state with Sidebar pushed onto the scope stack.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Sidebar);

        // When handling sidebar leave.
        let result = handle_sidebar_leave(&mut state);

        // Then Sidebar is no longer on the scope stack.
        assert!(!state.frontend.scope_stack.is_sidebar());
        assert!(result.commands.is_empty());
        // And persona section cursor is cleared.
        assert!(state.frontend.persona_section.cursor.is_none());
        // And pins selection is cleared.
        assert!(state.frontend.pins.selected_id().is_none());
    }

    #[rstest::rstest]
    fn sidebar_leave_restores_normal_scope() {
        // Given a state that entered sidebar from Normal mode.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Sidebar);

        // When handling sidebar leave.
        handle_sidebar_leave(&mut state);

        // Then the scope stack is back to Normal.
        assert_eq!(state.frontend.scope_stack.current(), &FocusScope::Normal);
        // And persona section cursor is cleared.
        assert!(state.frontend.persona_section.cursor.is_none());
    }

    #[rstest::rstest]
    fn sidebar_leave_restores_input_scope() {
        // Given a state that entered sidebar from Input mode.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Input);
        state.frontend.scope_stack.push(FocusScope::Sidebar);

        // When handling sidebar leave.
        handle_sidebar_leave(&mut state);

        // Then the scope stack is back to Input.
        assert_eq!(state.frontend.scope_stack.current(), &FocusScope::Input);
        // And persona section cursor is cleared.
        assert!(state.frontend.persona_section.cursor.is_none());
    }

    #[rstest::rstest]
    fn sidebar_leave_sets_cancel_prompt_when_streaming() {
        // Given a state in Sidebar with an active stream.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Sidebar);
        state.active_session_mut().begin_streaming();

        // When handling sidebar leave.
        let result = handle_sidebar_leave(&mut state);

        // Then the cancel prompt is set.
        assert!(state.frontend.cancel_stream_prompt);
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn sidebar_leave_no_prompt_when_idle() {
        // Given a state in Sidebar with idle session.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Sidebar);

        // When handling sidebar leave.
        let result = handle_sidebar_leave(&mut state);

        // Then no cancel prompt.
        assert!(!state.frontend.cancel_stream_prompt);
        assert!(result.commands.is_empty());
    }

    // --- Persona edit tests ---

    #[rstest::rstest]
    fn sidebar_persona_edit_opens_picker_when_persona_focused() {
        // Given a state with persona section focused and sidebar scope.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Sidebar);
        state.frontend.sidebar.focused_section = SidebarSectionId::Persona;

        // When handling sidebar persona edit.
        let result = handle_sidebar_persona_edit(&mut state);

        // Then the persona picker is active.
        assert_eq!(
            state.frontend.scope_stack.picker_kind().copied(),
            Some(crate::protocol::PickerKind::Persona)
        );
        // And a LoadPersonaPickerEntries command is returned.
        assert!(
            result
                .commands
                .iter()
                .any(|c| matches!(c, Command::LoadPersonaPickerEntries(..)))
        );
    }

    #[rstest::rstest]
    fn sidebar_persona_edit_noop_when_pins_focused() {
        // Given a state with pins section focused and sidebar scope.
        let mut state = AppState::default();
        state.frontend.scope_stack.push(FocusScope::Sidebar);
        state.frontend.sidebar.focused_section = SidebarSectionId::Pins;

        // When handling sidebar persona edit.
        let result = handle_sidebar_persona_edit(&mut state);

        // Then nothing changed.
        assert!(!state.frontend.scope_stack.is_picker());
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn pins_unpin_returns_command() {
        // Given a state with pinned entries.
        let mut state = state_with_pinned(2);

        // When handling pins unpin.
        let result = handle_pins_unpin(&mut state);

        // Then an UnpinChatEntry command is returned.
        assert!(
            result
                .commands
                .iter()
                .any(|c| matches!(c, Command::UnpinChatEntry(..)))
        );
    }

    #[rstest::rstest]
    fn pins_unpin_noop_when_empty() {
        // Given a state with no pinned entries.
        let mut state = AppState::default();

        // When handling pins unpin.
        let result = handle_pins_unpin(&mut state);

        // Then no commands.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn pins_pin_top_returns_command() {
        // Given a state with pinned entries.
        let mut state = state_with_pinned(1);

        // When handling pins pin top.
        let result = handle_pins_pin(&mut state, PinPosition::Top);

        // Then a PinChatEntry command with Top is returned.
        let pin_cmd = result.commands.iter().find_map(|c| match c {
            Command::PinChatEntry(payload) => Some(payload.position),
            _ => None,
        });
        assert_eq!(pin_cmd, Some(PinPosition::Top));
    }

    #[rstest::rstest]
    fn pins_pin_bottom_returns_command() {
        // Given a state with pinned entries.
        let mut state = state_with_pinned(1);

        // When handling pins pin bottom.
        let result = handle_pins_pin(&mut state, PinPosition::Bottom);

        // Then a PinChatEntry command with Bottom is returned.
        let pin_cmd = result.commands.iter().find_map(|c| match c {
            Command::PinChatEntry(payload) => Some(payload.position),
            _ => None,
        });
        assert_eq!(pin_cmd, Some(PinPosition::Bottom));
    }

    #[rstest::rstest]
    fn pins_pin_relative_returns_command() {
        // Given a state with pinned entries.
        let mut state = state_with_pinned(1);

        // When handling pins pin relative.
        let result = handle_pins_pin(&mut state, PinPosition::Relative);

        // Then a PinChatEntry command with Relative is returned.
        let pin_cmd = result.commands.iter().find_map(|c| match c {
            Command::PinChatEntry(payload) => Some(payload.position),
            _ => None,
        });
        assert_eq!(pin_cmd, Some(PinPosition::Relative));
    }

    #[rstest::rstest]
    fn pins_pin_cycle_rotates_top_to_bottom() {
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

        // When handling pins pin cycle.
        let result = handle_pins_pin_cycle(&mut state);

        // Then a PinChatEntry command with Bottom is returned.
        let pin_cmd = result.commands.iter().find_map(|c| match c {
            Command::PinChatEntry(payload) => Some(payload.position),
            _ => None,
        });
        assert_eq!(pin_cmd, Some(PinPosition::Bottom));
    }

    #[rstest::rstest]
    fn pins_pin_cycle_noop_when_empty() {
        // Given a state with no pinned entries.
        let mut state = AppState::default();

        // When handling pins pin cycle.
        let result = handle_pins_pin_cycle(&mut state);

        // Then no commands.
        assert!(result.commands.is_empty());
    }

    #[rstest::rstest]
    fn pins_pin_top_noop_when_no_selection() {
        // Given a state with pinned entries but no selection.
        let mut state = AppState::default();
        let entry = ChatEntry::user("entry");
        let entry_id = entry.id.clone();
        state.active_session_mut().push_entry(entry);
        state
            .active_session_mut()
            .pin_entry(&entry_id, PinPosition::Top);
        // Don't select anything.

        // When handling pins pin top.
        let result = handle_pins_pin(&mut state, PinPosition::Top);

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
    fn content_height_is_zero_when_empty() {
        // Given a PinsSection and state with no pinned entries.
        let section = PinsSection;
        let state = AppState::default();

        // When asking for content height.
        let height = section.content_height(&state);

        // Then it returns 0 (section is hidden when empty).
        assert_eq!(height, 0);
    }

    #[rstest::rstest]
    fn content_height_matches_entry_count() {
        // Given a PinsSection and state with 3 pinned entries.
        let section = PinsSection;
        let state = state_with_pinned(3);

        // When asking for content height.
        let height = section.content_height(&state);

        // Then it returns header(1) + blank(1) + (entry(1) + blank(1)) * 3 - last blank(1) + trailing gap(1) = 8.
        assert_eq!(height, 8);
    }

    // --- Rendering tests ---

    use nullslop_testutil::setup_term;

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
    fn render_empty_shows_header_with_zero_count() {
        let mut section = PinsSection;
        let state = AppState::default();
        let rows = render_rows(&mut section, &state, 40, 10);
        assert!(rows[0].contains("Pinned Context"));
        assert!(rows[0].contains('0'));
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
    fn render_selected_entry_has_yellow_marker_when_sidebar_focused() {
        let mut section = PinsSection;
        let mut state = state_with_pinned(2);
        // Sidebar must be focused for the indicator to be yellow.
        state.frontend.scope_stack.push(FocusScope::Sidebar);

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
    fn render_selected_entry_has_darkgray_marker_when_not_focused() {
        let mut section = PinsSection;
        let state = state_with_pinned(2);
        // Sidebar is NOT focused (Normal scope is the default).

        let (mut terminal, area) = setup_term(60, 20);
        terminal
            .draw(|frame| {
                section.render(frame, area, &state);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        // First entry at index 0 is selected by default.
        // The indicator should be DarkGray when sidebar is not focused.
        let cell0 = buffer.cell((0, 2)).expect("cell 0,2");
        assert_eq!(cell0.symbol(), "\u{2588}");
        assert_eq!(cell0.fg, Color::DarkGray);
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
        assert!(
            top_pos < rel_pos,
            "TOP entry should appear before REL entry"
        );
        assert!(
            rel_pos < bot_pos,
            "REL entry should appear before BOT entry"
        );
    }
}
