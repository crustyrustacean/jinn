#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

use jinn_testutil::setup_term;
use ratatui::style::Color;

use crate::common::app_state::{AppState, FocusScope};
use crate::feat::ui::sidebar::pins::pins_section::*;
use crate::feat::ui::sidebar::section_trait::{SidebarSection, SidebarSectionId};
use crate::protocol::{ChatEntry, Command, PinPosition};

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

// --- Persona edit tests ---

#[rstest::rstest]
fn sidebar_persona_edit_opens_picker_when_persona_focused() {
    // Given a state with persona section focused and sidebar scope.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::SidebarPersona);
    state
        .frontend
        .scope_stack
        .set_sidebar_section(SidebarSectionId::Persona);

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
    state.frontend.scope_stack.push(FocusScope::SidebarPersona);
    state
        .frontend
        .scope_stack
        .set_sidebar_section(SidebarSectionId::Pins);

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
    state.frontend.scope_stack.push(FocusScope::SidebarPins);

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
    // When sidebar is not focused, no selection indicator is shown.
    let cell0 = buffer.cell((0, 2)).expect("cell 0,2");
    assert_eq!(cell0.symbol(), " ");
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

// --- SessionNew section restriction tests ---

// Note: SessionNew section-scoping is now handled by the keymap (n is only
// bound in SidebarSessions scope), so the IntentHandler no longer checks
// which section is focused. These tests validated the old handler-level check.

#[rstest::rstest]
fn session_new_works_when_sidebar_sessions_focused() {
    // Given a state in Sidebar scope with Sessions section focused.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::SidebarPersona);
    state
        .frontend
        .scope_stack
        .set_sidebar_section(SidebarSectionId::Sessions);
    let old_id = state.session.active_session_id().clone();

    // When handling SessionNew via IntentHandler.
    let result = crate::feat::intent::IntentHandler::handle(&crate::Intent::SessionNew, &mut state);

    // Then a new session is created.
    assert_ne!(*state.session.active_session_id(), old_id);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn session_new_works_when_not_in_sidebar() {
    // Given a state in Normal scope (not sidebar).
    let mut state = AppState::default();
    let old_id = state.session.active_session_id().clone();

    // When handling SessionNew via IntentHandler.
    let _result =
        crate::feat::intent::IntentHandler::handle(&crate::Intent::SessionNew, &mut state);

    // Then a new session is created (no section restriction outside sidebar).
    assert_ne!(*state.session.active_session_id(), old_id);
}

// --- sync_chat_log_cursor tests ---

#[rstest::rstest]
fn sync_chat_log_cursor_sets_cursor_by_entry_id_with_visual_items() {
    // Given a session with ignored entries (causing visual-item index != history index)
    // and a pinned entry deep in history.
    use crate::feat::ui::chat_log::visual_item::{
        DEFAULT_MIN_COLLAPSE_COUNT, PROXIMITY_COUNT, build_visual_items,
    };

    let mut state = AppState::default();
    state.active_session_mut().push_entry(ChatEntry::user("a")); // hist 0
    for _ in 0..15 {
        let mut entry = ChatEntry::user("ignored");
        entry.context_override = crate::protocol::ContextOverride::ForcedExclude;
        state.active_session_mut().push_entry(entry);
    } // hist 1..15 (collapsed into 1 visual item)
    state.active_session_mut().push_entry(ChatEntry::user("b")); // hist 16

    // Pin the entry at history index 12 (inside the ignored block).
    // When the block is expanded, this entry should be selectable.
    let pinned_id = state.active_session().history()[12].id.clone();
    state
        .active_session_mut()
        .pin_entry(&pinned_id, PinPosition::Top);

    // Expand the ignored block so the pinned entry is visible.
    let block_start_id = state.active_session().history()[1].id.clone();
    state
        .active_session_mut()
        .toggle_ignored_block_visibility(&block_start_id);

    // Build visual items (now expanded — individual Entry items).
    let items = build_visual_items(
        state.active_session().history(),
        &state.active_session().ui.shown_ignored_blocks,
        PROXIMITY_COUNT,
        DEFAULT_MIN_COLLAPSE_COUNT,
    );
    state.active_session_mut().set_visual_items(items);

    // Select the pinned entry in the pins section.
    state.frontend.pins.select_by_id(pinned_id.clone());

    // Set cursor to something else first.
    let first_entry_id = state.active_session().history()[0].id.clone();
    state
        .active_session_mut()
        .set_selected_cursor_id(first_entry_id);
    assert_ne!(
        state.active_session().selected_cursor_id(),
        Some(&pinned_id),
        "precondition: cursor should not be on pinned entry"
    );

    // When sync_chat_log_cursor is called.
    crate::feat::ui::sidebar::pins::pins_section::sync_chat_log_cursor(&mut state);

    // Then the chat log cursor is set to the pinned entry by ID.
    assert_eq!(
        state.active_session().selected_cursor_id(),
        Some(&pinned_id),
        "sync_chat_log_cursor should set cursor to pinned entry by ID"
    );
}

#[rstest::rstest]
fn sync_chat_log_cursor_sets_correct_entry_when_multiple_entries_exist() {
    // Given a session with 3 entries and the middle one pinned.
    let mut state = AppState::default();
    let entry_a = ChatEntry::user("entry a");
    let id_a = entry_a.id.clone();
    state.active_session_mut().push_entry(entry_a);

    let entry_b = ChatEntry::user("entry b");
    let id_b = entry_b.id.clone();
    state.active_session_mut().push_entry(entry_b);

    let entry_c = ChatEntry::user("entry c");
    let id_c = entry_c.id.clone();
    state.active_session_mut().push_entry(entry_c);

    // Pin entry_b and select it in the pins section.
    state.active_session_mut().pin_entry(&id_b, PinPosition::Top);
    state.frontend.pins.select_by_id(id_b.clone());

    // Set cursor to entry_a first.
    state.active_session_mut().set_selected_cursor_id(id_a.clone());

    // When sync_chat_log_cursor is called.
    sync_chat_log_cursor(&mut state);

    // Then the cursor is set to entry_b (not entry_a or entry_c).
    // This kills the == -> != mutant which would match the wrong entry.
    assert_eq!(
        state.active_session().selected_cursor_id(),
        Some(&id_b),
        "sync_chat_log_cursor should set cursor to the pinned entry, not others"
    );
    assert_ne!(
        state.active_session().selected_cursor_id(),
        Some(&id_a),
        "cursor should not be on entry a"
    );
    assert_ne!(
        state.active_session().selected_cursor_id(),
        Some(&id_c),
        "cursor should not be on entry c"
    );
}

#[rstest::rstest]
fn resolve_selected_entry_id_returns_real_session_and_entry_ids() {
    // Given a state with a pinned entry.
    let mut state = AppState::default();
    let entry = ChatEntry::user("pinned entry");
    let entry_id = entry.id.clone();
    state.active_session_mut().push_entry(entry);
    state.active_session_mut().pin_entry(&entry_id, PinPosition::Top);
    state.frontend.pins.select_by_id(entry_id.clone());

    // When handling pins unpin (which uses resolve_selected_entry_id internally).
    let result = handle_pins_unpin(&mut state);

    // Then the command contains the real session ID and the real entry ID
    // (not default/placeholder values).
    let cmd = result.commands.iter().find_map(|c| match c {
        Command::UnpinChatEntry(payload) => Some(payload.clone()),
        _ => None,
    });
    assert!(cmd.is_some(), "should return an UnpinChatEntry command");
    let payload = cmd.unwrap();
    assert_eq!(
        payload.session_id, *state.session.active_session_id(),
        "session_id should be the real active session ID, not a default"
    );
    assert_eq!(
        payload.entry_id, entry_id,
        "entry_id should be the real pinned entry ID, not a default"
    );
}
