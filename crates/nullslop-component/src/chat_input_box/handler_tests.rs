use crate::test_utils;
use crate::chat_input_box::ChatInputBoxHandler;
use crate::prompt_template::{PromptTemplate, PromptTemplateStore};
use crate::{AppState, AppBus};
use nullslop_component_core::Bus;
use nullslop_protocol::chat_input::{InsertChar, SubmitMessage};
use nullslop_protocol::{Command, SessionId};
use nullslop_services::Services;

/// Helper: create a bus with `ChatInputBoxHandler` registered and a store populated.
fn setup_bus_with_templates() -> (AppBus, AppState, PromptTemplateStore) {
    let mut bus: AppBus = Bus::new();
    ChatInputBoxHandler.register(&mut bus);

    let store = PromptTemplateStore::from_vec(vec![
        PromptTemplate {
            name: "code-review".into(),
            description: "Review code".into(),
            body: "You are a code reviewer.".into(),
        },
        PromptTemplate {
            name: "commit-message".into(),
            description: "Write commit".into(),
            body: "Write a commit message.".into(),
        },
        PromptTemplate {
            name: "codellama".into(),
            description: "Codellama model".into(),
            body: "You are codellama.".into(),
        },
    ]);

    let state = AppState::default();
    (bus, state, store)
}

fn insert_char(bus: &mut AppBus, state: &mut AppState, services: &Services, ch: char) {
    bus.submit_command(Command::InsertChar {
        payload: InsertChar { ch },
    });
    bus.process_commands(state, services);
}

// --- Test 1: Typing $ at start activates autocomplete ---

#[test]
fn typing_dollar_at_start_activates_autocomplete() {
    // Given a bus with handler and templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    // When typing $ at start of buffer.
    insert_char(&mut bus, &mut state, &services, '$');

    // Then autocomplete is active with matches.
    let ac = state.active_chat_input().autocomplete();
    assert!(ac.is_some(), "autocomplete should be active after $");
    let ac = ac.as_ref().unwrap();
    assert_eq!(ac.token_start(), 0);
    assert!(!ac.matches().is_empty(), "should have matches");
}

// --- Test 2: Typing $ after space activates autocomplete ---

#[test]
fn typing_dollar_after_space_activates_autocomplete() {
    // Given a bus with handler and templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    // When typing "foo $".
    for ch in "foo $".chars() {
        insert_char(&mut bus, &mut state, &services, ch);
    }

    // Then autocomplete is active.
    assert!(
        state.active_chat_input().autocomplete().is_some(),
        "autocomplete should activate after space then $"
    );
}

// --- Test 3: Typing $ midword does NOT activate autocomplete ---

#[test]
fn typing_dollar_midword_does_not_activate() {
    // Given a bus with handler and templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    // When typing "foo$".
    for ch in "foo$".chars() {
        insert_char(&mut bus, &mut state, &services, ch);
    }

    // Then autocomplete is NOT active.
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "autocomplete should NOT activate midword"
    );
}

// --- Test 4: Typing space after $ deactivates autocomplete ---

#[test]
fn typing_space_after_dollar_deactivates_autocomplete() {
    // Given a bus with handler and templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    // When typing "$ ".
    insert_char(&mut bus, &mut state, &services, '$');
    assert!(state.active_chat_input().autocomplete().is_some());
    insert_char(&mut bus, &mut state, &services, ' ');

    // Then autocomplete is deactivated.
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "space should deactivate autocomplete"
    );
}

// --- Test 5: Completing a name replaces the token ---

#[test]
fn completing_name_replaces_token() {
    // Given a bus with handler and templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    // When typing "$co" then submitting (Enter).
    insert_char(&mut bus, &mut state, &services, '$');
    insert_char(&mut bus, &mut state, &services, 'c');
    insert_char(&mut bus, &mut state, &services, 'o');

    // Submit should complete instead of enqueueing.
    bus.submit_command(Command::SubmitMessage {
        payload: SubmitMessage {
            session_id: SessionId::new(),
            text: String::new(),
        },
    });
    bus.process_commands(&mut state, &services);

    // Then the buffer contains $<exactname>.
    let text = state.active_chat_input().text();
    assert!(
        text.starts_with('$'),
        "buffer should start with $, got: {text}"
    );
    assert!(
        text.contains("code-review")
            || text.contains("commit-message")
            || text.contains("codellama"),
        "buffer should contain a matched name, got: {text}"
    );
}

// --- Test 6: Double-$ expands template body ---

#[test]
fn double_dollar_expands_template_body() {
    // Given a bus with handler and templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    // When typing "$code-review$" (exact name match + closing $).
    for ch in "$code-review$".chars() {
        insert_char(&mut bus, &mut state, &services, ch);
    }

    // Then the buffer contains the template body and autocomplete is deactivated.
    let text = state.active_chat_input().text();
    assert!(
        text.contains("You are a code reviewer."),
        "buffer should contain the template body, got: {text}"
    );
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "autocomplete should be deactivated after expansion"
    );
}

// --- Test 7: Double-$ with unknown name leaves literal ---

#[test]
fn double_dollar_with_unknown_name_leaves_literal() {
    // Given a bus with handler and templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    // When typing "$unknown$" (no matching template).
    for ch in "$unknown$".chars() {
        insert_char(&mut bus, &mut state, &services, ch);
    }

    // Then the buffer contains "$unknown$" as literal text.
    let text = state.active_chat_input().text();
    assert!(
        text.contains("$unknown$"),
        "buffer should contain literal $unknown$, got: {text}"
    );
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "autocomplete should be deactivated"
    );
}

// --- Test 8: Backspace removing $ deactivates ---

#[test]
fn backspace_removing_dollar_deactivates() {
    // Given a bus with handler and templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    // When typing "$" then backspace.
    insert_char(&mut bus, &mut state, &services, '$');
    assert!(state.active_chat_input().autocomplete().is_some());

    bus.submit_command(Command::DeleteGrapheme);
    bus.process_commands(&mut state, &services);

    // Then autocomplete is deactivated and $ is removed.
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "backspace on $ should deactivate autocomplete"
    );
    assert!(
        state.active_chat_input().text().is_empty(),
        "buffer should be empty after backspace on $"
    );
}

// --- Test 9: Backspace within filter updates matches ---

#[test]
fn backspace_within_filter_updates_matches() {
    // Given a bus with handler and templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    // When typing "$co" then backspace.
    insert_char(&mut bus, &mut state, &services, '$');
    insert_char(&mut bus, &mut state, &services, 'c');
    insert_char(&mut bus, &mut state, &services, 'o');

    bus.submit_command(Command::DeleteGrapheme);
    bus.process_commands(&mut state, &services);

    // Then autocomplete is still active with filter "c".
    let ac = state.active_chat_input().autocomplete();
    assert!(ac.is_some(), "autocomplete should still be active");
    let filter = state.active_chat_input().autocomplete_filter().unwrap();
    assert_eq!(filter, "c", "filter should be 'c' after backspace");
}

// --- Test 10: Cursor left leaving token deactivates ---

#[test]
fn cursor_left_leaving_token_deactivates() {
    // Given a bus with handler and templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    // When typing "$" then moving cursor left past $.
    insert_char(&mut bus, &mut state, &services, '$');
    bus.submit_command(Command::MoveCursorLeft);
    bus.process_commands(&mut state, &services);

    // Then autocomplete is deactivated.
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "moving left past $ should deactivate autocomplete"
    );
}

// --- Test 11: Clear deactivates autocomplete ---

#[test]
fn clear_deactivates_autocomplete() {
    // Given a bus with handler and templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    // When typing "$" then clearing.
    insert_char(&mut bus, &mut state, &services, '$');
    bus.submit_command(Command::Clear);
    bus.process_commands(&mut state, &services);

    // Then autocomplete is deactivated.
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "clear should deactivate autocomplete"
    );
}

// --- Test 12: Interrupt deactivates autocomplete ---

#[test]
fn interrupt_deactivates_autocomplete() {
    // Given a bus with handler and templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    // When typing "$foo" then interrupting.
    insert_char(&mut bus, &mut state, &services, '$');
    insert_char(&mut bus, &mut state, &services, 'f');
    insert_char(&mut bus, &mut state, &services, 'o');
    insert_char(&mut bus, &mut state, &services, 'o');

    bus.submit_command(Command::Interrupt);
    bus.process_commands(&mut state, &services);

    // Then autocomplete is deactivated and buffer is cleared.
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "interrupt should deactivate autocomplete"
    );
    assert!(
        state.active_chat_input().text().is_empty(),
        "interrupt should clear the buffer"
    );
}

// --- Test 13: Arrow up/down navigate matches ---

#[test]
fn arrow_up_down_navigate_matches() {
    // Given a bus with handler and templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    // When typing "$" to activate (multiple matches).
    insert_char(&mut bus, &mut state, &services, '$');

    let initial_idx = state
        .active_chat_input()
        .autocomplete()
        .as_ref()
        .unwrap()
        .selected_index();

    // When pressing up (toward less relevant).
    bus.submit_command(Command::MoveCursorUp);
    bus.process_commands(&mut state, &services);
    let after_up = state
        .active_chat_input()
        .autocomplete()
        .as_ref()
        .unwrap()
        .selected_index();
    assert!(
        after_up <= initial_idx,
        "up should decrease or maintain index"
    );

    // When pressing down (toward more relevant).
    bus.submit_command(Command::MoveCursorDown);
    bus.process_commands(&mut state, &services);
    let after_down = state
        .active_chat_input()
        .autocomplete()
        .as_ref()
        .unwrap()
        .selected_index();
    assert!(
        after_down >= after_up,
        "down should increase or maintain index"
    );
}

// --- Test 14: Tab completes when autocomplete active ---

#[test]
fn tab_completes_when_autocomplete_active() {
    // Given a bus with handler and templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    // When typing "$co" then pressing Tab.
    insert_char(&mut bus, &mut state, &services, '$');
    insert_char(&mut bus, &mut state, &services, 'c');
    insert_char(&mut bus, &mut state, &services, 'o');

    bus.submit_command(Command::AutocompleteConfirm);
    bus.process_commands(&mut state, &services);

    // Then the buffer contains $<matched_name>.
    let text = state.active_chat_input().text();
    assert!(
        text.starts_with('$'),
        "buffer should start with $, got: {text}"
    );
    assert!(
        text.len() > 1,
        "buffer should have content after $, got: {text}"
    );
}

// --- Test 15: Tab switches tab when autocomplete inactive ---

#[test]
fn tab_switches_tab_when_autocomplete_inactive() {
    // Given a bus with handler (no templates needed).
    let mut bus: AppBus = Bus::new();
    ChatInputBoxHandler.register(&mut bus);

    let state = AppState::default();
    let services = test_utils::test_services();
    let mut state = state;

    // Tab without autocomplete should submit SwitchTab without crash.
    bus.submit_command(Command::AutocompleteConfirm);
    bus.process_commands(&mut state, &services);

    // No crash, autocomplete is still None.
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "autocomplete should remain inactive"
    );
}

// --- Test 16: Empty matches selected_index safe ---

#[test]
fn empty_autocomplete_matches_has_zero_count() {
    // Given a bus with handler and no matching templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    // When typing "$zzzzzz" (filter matches nothing).
    for ch in "$zzzzzz".chars() {
        insert_char(&mut bus, &mut state, &services, ch);
    }

    // Then autocomplete is active with 0 matches and selected_index 0.
    let ac = state.active_chat_input().autocomplete();
    assert!(ac.is_some(), "autocomplete should be active");
    let ac = ac.as_ref().unwrap();
    assert_eq!(ac.matches().len(), 0, "should have no matches");
    assert_eq!(ac.selected_index(), 0, "selected_index should be 0");
}

#[test]
fn navigating_with_empty_matches_does_not_panic() {
    // Given a bus with handler and no matching templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    for ch in "$zzzzzz".chars() {
        insert_char(&mut bus, &mut state, &services, ch);
    }

    // When navigating up/down with empty matches, no panic.
    bus.submit_command(Command::MoveCursorUp);
    bus.process_commands(&mut state, &services);
    bus.submit_command(Command::MoveCursorDown);
    bus.process_commands(&mut state, &services);

    // Then still safe with selected_index 0.
    let ac = state.active_chat_input().autocomplete().as_ref().unwrap();
    assert_eq!(ac.selected_index(), 0);
}

// --- Test: Delete forward within filter updates matches ---

#[test]
fn delete_forward_within_filter_updates_matches() {
    // Given a bus with handler and templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    // When typing "$co" then moving cursor left, then forward delete.
    insert_char(&mut bus, &mut state, &services, '$');
    insert_char(&mut bus, &mut state, &services, 'c');
    insert_char(&mut bus, &mut state, &services, 'o');
    // cursor at 3

    // Move cursor left (still within filter, cursor now at 2).
    bus.submit_command(Command::MoveCursorLeft);
    bus.process_commands(&mut state, &services);
    // cursor at 2, token_start=0, cursor > token_start → still active
    assert!(
        state.active_chat_input().autocomplete().is_some(),
        "autocomplete should remain active"
    );

    // Forward delete removes 'o'.
    bus.submit_command(Command::DeleteGraphemeForward);
    bus.process_commands(&mut state, &services);

    // Then filter should be "c".
    assert!(
        state.active_chat_input().autocomplete().is_some(),
        "autocomplete should remain active"
    );
    let filter = state.active_chat_input().autocomplete_filter().unwrap();
    assert_eq!(filter, "c");
}

// --- Test: Multiple $ references in one buffer ---

#[test]
fn multiple_dollar_references_in_one_buffer() {
    // Given a bus with handler and templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    // When typing "$co" then space, then "$co".
    insert_char(&mut bus, &mut state, &services, '$');
    insert_char(&mut bus, &mut state, &services, 'c');
    insert_char(&mut bus, &mut state, &services, 'o');
    insert_char(&mut bus, &mut state, &services, ' '); // deactivates
    assert!(state.active_chat_input().autocomplete().is_none());

    insert_char(&mut bus, &mut state, &services, '$');
    insert_char(&mut bus, &mut state, &services, 'c');
    insert_char(&mut bus, &mut state, &services, 'o');

    // Then autocomplete is active for the second token.
    let ac = state.active_chat_input().autocomplete();
    assert!(ac.is_some(), "second $ should reactivate autocomplete");
    let ac = ac.as_ref().unwrap();
    assert_eq!(ac.token_start(), 4, "second $ should be at position 4");
}

// --- Test: Word-left deactivates autocomplete ---

#[test]
fn word_left_deactivates_autocomplete() {
    // Given a bus with handler and templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    // When typing "foo $co" then word-left.
    for ch in "foo $co".chars() {
        insert_char(&mut bus, &mut state, &services, ch);
    }
    assert!(state.active_chat_input().autocomplete().is_some());

    bus.submit_command(Command::MoveCursorWordLeft);
    bus.process_commands(&mut state, &services);

    // Then autocomplete is deactivated.
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "word-left should deactivate autocomplete"
    );
}

// --- Test: Word-right deactivates autocomplete ---

#[test]
fn word_right_deactivates_autocomplete() {
    // Given a bus with handler and templates.
    let (mut bus, mut state, store) = setup_bus_with_templates();
    let services = test_utils::test_services();
    state.prompt_templates = store;

    // When typing "$co" then word-right.
    insert_char(&mut bus, &mut state, &services, '$');
    insert_char(&mut bus, &mut state, &services, 'c');
    insert_char(&mut bus, &mut state, &services, 'o');

    bus.submit_command(Command::MoveCursorWordRight);
    bus.process_commands(&mut state, &services);

    // Then autocomplete is deactivated.
    assert!(
        state.active_chat_input().autocomplete().is_none(),
        "word-right should deactivate autocomplete"
    );
}
