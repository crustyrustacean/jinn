#![allow(clippy::expect_used, clippy::indexing_slicing)]

use crate::common::app_state::AppState;
use crate::common::app_state::FocusScope;
use crate::feat::picker::intent::{
    handle_backspace, handle_insert_char, handle_move_cursor_left, handle_move_cursor_right,
    handle_move_down, handle_move_up, handle_open_picker, handle_picker_confirm,
    handle_toggle_fork_assistant_filter, handle_toggle_fork_user_filter,
    handle_toggle_keymap_scope_filter,
};
use crate::feat::preferences_actor::protocol::command::{PreferenceUpdate, UpdatePreferences};
use crate::feat::session::fork_entry::ForkEntry;
use crate::feat::theme::default_theme;
use crate::protocol::{
    Command, Intent, KeymapEntry, PickerEntry, PickerKind, SessionEntry, SessionId, StrategyEntry,
};
use ratatui::style::Color;

// --- Open picker ---

#[rstest::rstest]
fn open_picker_provider_sets_kind_and_mode() {
    // Given a default state.
    let mut state = AppState::default();

    // When opening a Provider picker.
    let result = handle_open_picker(&mut state, PickerKind::Provider);

    // Then scope_stack has a Picker(Provider) on top.
    assert_eq!(
        state.frontend.scope_stack.picker_kind().copied(),
        Some(PickerKind::Provider)
    );
    // And a LoadProviderPickerEntries command is returned.
    assert!(
        result
            .commands
            .iter()
            .any(|c| matches!(c, Command::LoadProviderPickerEntries(..)))
    );
}

#[rstest::rstest]
fn open_picker_keymap_resets_show_all() {
    // Given a state with show_all=true.
    let mut state = AppState::default();
    state.frontend.keymap_picker_show_all = true;

    // When opening a Keymap picker.
    let result = handle_open_picker(&mut state, PickerKind::Keymap);

    // Then show_all is false.
    assert!(!state.frontend.keymap_picker_show_all);
    assert_eq!(
        state.frontend.scope_stack.picker_kind().copied(),
        Some(PickerKind::Keymap)
    );
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn open_picker_noop_when_already_in_picker() {
    // Given a state already in picker mode.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Session,
    });

    // When opening a Provider picker.
    let result = handle_open_picker(&mut state, PickerKind::Provider);

    // Then nothing changed.
    assert_eq!(
        state.frontend.scope_stack.picker_kind().copied(),
        Some(PickerKind::Session)
    );
    assert!(result.commands.is_empty());
}

// --- Insert char / backspace ---

#[rstest::rstest]
fn picker_insert_char_updates_filter() {
    // Given a state with active provider picker.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Provider,
    });
    state.provider.provider_picker.set_items(vec![PickerEntry {
        provider_id: "test/model".to_owned(),
        name: "test".to_owned(),
        provider_name: "test".to_owned(),
        backend: "openai".to_owned(),
        model: "Test".to_owned(),
        search_text: "Test test".to_owned(),
        is_alias: false,
        alias_target: None,
        is_available: true,
        is_remote: false,
        is_active: false,
        theme: default_theme(),
    }]);

    // When inserting 't'.
    let result = handle_insert_char(&mut state, 't');

    // Then the filter contains "t".
    assert_eq!(state.provider.provider_picker.filter(), "t");
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn picker_backspace_removes_from_filter() {
    // Given a state with active provider picker and "te" in filter.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Provider,
    });
    state.provider.provider_picker.set_items(vec![PickerEntry {
        provider_id: "test/model".to_owned(),
        name: "test".to_owned(),
        provider_name: "test".to_owned(),
        backend: "openai".to_owned(),
        model: "Test".to_owned(),
        search_text: "Test test".to_owned(),
        is_alias: false,
        alias_target: None,
        is_available: true,
        is_remote: false,
        is_active: false,
        theme: default_theme(),
    }]);
    state.provider.provider_picker.insert_char('t');
    state.provider.provider_picker.insert_char('e');

    // When handling backspace.
    let result = handle_backspace(&mut state);

    // Then the filter is "t".
    assert_eq!(state.provider.provider_picker.filter(), "t");
    assert!(result.commands.is_empty());
}

// --- Confirm ---

#[rstest::rstest]
fn picker_confirm_provider_returns_provider_switch() {
    // Given a state with active provider picker and a selected entry.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Provider,
    });
    state.provider.provider_picker.set_items(vec![PickerEntry {
        provider_id: "test/model".to_owned(),
        name: "test".to_owned(),
        provider_name: "test".to_owned(),
        backend: "openai".to_owned(),
        model: "Test".to_owned(),
        search_text: "Test test".to_owned(),
        is_alias: false,
        alias_target: None,
        is_available: true,
        is_remote: false,
        is_active: false,
        theme: default_theme(),
    }]);

    // When confirming picker.
    let (result, maybe_intent) = handle_picker_confirm(&mut state);

    // Then a ProviderSwitch command is returned and picker is closed.
    assert!(
        result
            .commands
            .iter()
            .any(|c| matches!(c, Command::ProviderSwitch(..)))
    );
    assert!(!state.frontend.scope_stack.is_picker());
    assert!(maybe_intent.is_none());
}

#[rstest::rstest]
fn picker_confirm_session_returns_session_load_command() {
    // Given a state with active session picker and a selected entry.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Session,
    });
    state.frontend.session_picker.set_items(vec![SessionEntry {
        session_id: SessionId::new(),
        title: "Test".to_owned(),
        updated_at: jiff::Timestamp::now(),
        theme: default_theme(),
        session_state: crate::feat::session::chat_session::SessionState::Loaded,
    }]);

    // When confirming picker.
    let (result, maybe_intent) = handle_picker_confirm(&mut state);

    // Then session is loading.
    assert!(state.session.is_loading());
    // And session_load_guard is set.
    assert!(state.session.session_load_guard().is_some());
    // And a SessionLoadRequested command is returned.
    assert!(
        result
            .commands
            .iter()
            .any(|c| matches!(c, Command::SessionLoadRequested(..)))
    );
    // And picker is closed.
    assert!(!state.frontend.scope_stack.is_picker());
    assert!(maybe_intent.is_none());
}

#[rstest::rstest]
fn picker_confirm_keymap_returns_intent_for_redispatch() {
    // Given a state with active keymap picker.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Keymap,
    });
    state.frontend.keymap_picker.set_items(vec![KeymapEntry {
        key_sequence: "q".to_owned(),
        description: "quit".to_owned(),
        scope: "Normal".to_owned(),
        category: "General".to_owned(),
        command: Intent::Quit,
        search_text: "q quit".to_owned(),
        theme: default_theme(),
    }]);

    // When confirming picker.
    let (result, maybe_intent) = handle_picker_confirm(&mut state);

    // Then picker is closed and the intent is returned for redispatch.
    assert!(!state.frontend.scope_stack.is_picker());
    assert!(result.commands.is_empty());
    assert!(matches!(maybe_intent, Some(Intent::Quit)));
}

#[rstest::rstest]
fn picker_confirm_noop_with_no_active_picker() {
    // Given a state with no active picker.
    let mut state = AppState::default();

    // When confirming picker.
    let (result, maybe_intent) = handle_picker_confirm(&mut state);

    // Then no commands and no intent.
    assert!(result.commands.is_empty());
    assert!(maybe_intent.is_none());
}

#[rstest::rstest]
fn picker_confirm_strategy_updates_default() {
    // Given a state with active context strategy picker and manual entries.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::ContextAssembly,
    });
    state.frontend.context_strategy_picker.set_items(vec![
        StrategyEntry {
            strategy_id: crate::protocol::PromptStrategyId::passthrough(),
            name: "Passthrough".to_owned(),
            description: "No processing".to_owned(),
            is_active: false,
            theme: default_theme(),
        },
        StrategyEntry {
            strategy_id: crate::protocol::PromptStrategyId::sliding_window(),
            name: "Sliding Window".to_owned(),
            description: "Sliding window".to_owned(),
            is_active: false,
            theme: default_theme(),
        },
    ]);
    // Navigate to second entry.
    state.frontend.context_strategy_picker.move_down(100);

    // When confirming picker.
    let (result, maybe_intent) = handle_picker_confirm(&mut state);

    // Then SwitchPromptStrategy command is returned.
    assert!(
        result
            .commands
            .iter()
            .any(|c| matches!(c, Command::SwitchPromptStrategy(..)))
    );
    // And picker is closed.
    assert!(!state.frontend.scope_stack.is_picker());
    assert!(maybe_intent.is_none());
}

// --- Move up / down ---

#[rstest::rstest]
fn picker_move_up_decrements_selection() {
    // Given a state with active provider picker at index 1.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Provider,
    });
    state.provider.provider_picker.set_items(vec![
        PickerEntry {
            provider_id: "a".to_owned(),
            name: "a".to_owned(),
            provider_name: "a".to_owned(),
            backend: "a".to_owned(),
            model: "a".to_owned(),
            search_text: "a a".to_owned(),
            is_alias: false,
            alias_target: None,
            is_available: true,
            is_remote: false,
            is_active: false,
            theme: default_theme(),
        },
        PickerEntry {
            provider_id: "b".to_owned(),
            name: "b".to_owned(),
            provider_name: "b".to_owned(),
            backend: "b".to_owned(),
            model: "b".to_owned(),
            search_text: "b b".to_owned(),
            is_alias: false,
            alias_target: None,
            is_available: true,
            is_remote: false,
            is_active: false,
            theme: default_theme(),
        },
    ]);
    state.provider.provider_picker.move_down(100);
    assert_eq!(state.provider.provider_picker.selection(), 1);

    // When handling move up.
    let result = handle_move_up(&mut state);

    // Then selection is 0.
    assert_eq!(state.provider.provider_picker.selection(), 0);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn picker_move_down_increments_selection() {
    // Given a state with active provider picker at index 0.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Provider,
    });
    state.provider.provider_picker.set_items(vec![
        PickerEntry {
            provider_id: "a".to_owned(),
            name: "a".to_owned(),
            provider_name: "a".to_owned(),
            backend: "a".to_owned(),
            model: "a".to_owned(),
            search_text: "a a".to_owned(),
            is_alias: false,
            alias_target: None,
            is_available: true,
            is_remote: false,
            is_active: false,
            theme: default_theme(),
        },
        PickerEntry {
            provider_id: "b".to_owned(),
            name: "b".to_owned(),
            provider_name: "b".to_owned(),
            backend: "b".to_owned(),
            model: "b".to_owned(),
            search_text: "b b".to_owned(),
            is_alias: false,
            alias_target: None,
            is_available: true,
            is_remote: false,
            is_active: false,
            theme: default_theme(),
        },
    ]);

    // When handling move down.
    let result = handle_move_down(&mut state);

    // Then selection is 1.
    assert_eq!(state.provider.provider_picker.selection(), 1);
    assert!(result.commands.is_empty());
}

// --- Cursor movement ---

#[rstest::rstest]
fn picker_move_cursor_left_moves_cursor() {
    // Given a state with active provider picker with "ab" in filter.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Provider,
    });
    state.provider.provider_picker.insert_char('a');
    state.provider.provider_picker.insert_char('b');

    // When handling cursor left.
    let result = handle_move_cursor_left(&mut state);

    // Then cursor moved.
    assert_eq!(state.provider.provider_picker.cursor_pos(), 1);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn picker_move_cursor_right_moves_cursor() {
    // Given a state with cursor at start of filter.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Provider,
    });
    state.provider.provider_picker.insert_char('a');
    state.provider.provider_picker.insert_char('b');
    state.provider.provider_picker.move_cursor_left();
    state.provider.provider_picker.move_cursor_left();

    // When handling cursor right.
    let result = handle_move_cursor_right(&mut state);

    // Then cursor moved.
    assert_eq!(state.provider.provider_picker.cursor_pos(), 1);
    assert!(result.commands.is_empty());
}

// --- Toggle scope filter ---

#[rstest::rstest]
fn toggle_keymap_scope_filter_toggles_flag() {
    // Given a state with keymap entries.
    let mut state = AppState::default();
    state.frontend.all_keymap_entries = vec![KeymapEntry {
        key_sequence: "q".to_owned(),
        description: "quit".to_owned(),
        scope: "Normal".to_owned(),
        category: "General".to_owned(),
        command: Intent::Quit,
        search_text: "q quit".to_owned(),
        theme: state.frontend.theme.clone(),
    }];
    state.frontend.scope_stack.push(FocusScope::Input);
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Keymap,
    });

    // When handling toggle keymap scope filter.
    let result = handle_toggle_keymap_scope_filter(&mut state);

    // Then show_all is toggled to true.
    assert!(state.frontend.keymap_picker_show_all);
    assert!(result.commands.is_empty());
}

// --- Theme picker tests ---

#[rstest::rstest]
fn open_theme_picker_saves_original_theme() {
    // Given a state with a custom theme.
    let mut state = AppState::default();
    state.frontend.theme.focus_accent = Color::Red;

    // When opening the theme picker.
    let result = handle_open_picker(&mut state, PickerKind::Theme);

    // Then the original theme is saved.
    assert_eq!(
        state
            .frontend
            .theme_preview_original
            .as_ref()
            .unwrap()
            .focus_accent,
        Color::Red
    );
    // And the picker has items (at least default).
    assert!(!state.frontend.theme_picker.items().is_empty());
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn confirm_theme_persists_selection() {
    // Given a state with theme picker open and default selected.
    use crate::feat::theme::{ThemeEntry, default_theme};

    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Theme,
    });
    state.frontend.theme_picker.set_items(vec![ThemeEntry {
        name: "default".to_owned(),
        theme: default_theme(),
    }]);

    // When confirming the theme picker.
    let (result, _maybe_intent) = handle_picker_confirm(&mut state);

    // Then a SetTheme command is returned.
    assert!(result.commands.iter().any(|c| matches!(
        c,
        Command::UpdatePreferences(UpdatePreferences {
            updates
        }) if updates.iter().any(|u| matches!(
            u,
            PreferenceUpdate::SetTheme(Some(name)) if name == "default"
        ))
    )));
    // And the scope is popped.
    assert!(!state.frontend.scope_stack.is_picker());
}

#[rstest::rstest]
fn escape_theme_picker_restores_original() {
    // Given a state with theme picker open and a different theme previewed.
    let mut state = AppState::default();
    state.frontend.theme.focus_accent = Color::Red;
    state.frontend.theme_preview_original = Some({
        let mut original = default_theme();
        original.focus_accent = Color::Yellow;
        original
    });
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Theme,
    });

    // When handling enter normal mode (ESC).
    let result = crate::feat::chat_input::intent::handle_enter_normal_mode(&mut state);

    // Then the theme is restored to the original.
    assert_eq!(state.frontend.theme.focus_accent, Color::Yellow);
    // And preview original is cleared.
    assert!(state.frontend.theme_preview_original.is_none());
    assert!(result.commands.is_empty());
}

// --- Session Fork picker ---

#[rstest::rstest]
fn open_fork_picker_populates_entries_from_active_session() {
    // Given a state with chat history containing user and assistant entries.
    let mut state = AppState::default();
    state
        .active_session_mut()
        .push_entry(crate::ChatEntry::user("hello"));
    state
        .active_session_mut()
        .push_entry(crate::ChatEntry::assistant("world"));
    state
        .active_session_mut()
        .push_entry(crate::ChatEntry::system("system msg"));
    state
        .active_session_mut()
        .push_entry(crate::ChatEntry::user("second question"));

    // When opening the fork picker.
    let result = handle_open_picker(&mut state, PickerKind::SessionFork);

    // Then the picker is active.
    assert_eq!(
        state.frontend.scope_stack.picker_kind().copied(),
        Some(PickerKind::SessionFork)
    );
    // And the fork picker has 3 entries (2 user + 1 assistant, no system).
    assert_eq!(state.frontend.fork_picker.items().len(), 3);
    // And no commands (entries come from in-memory state).
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn fork_picker_entries_have_correct_ordinals() {
    // Given a state with chat history.
    let mut state = AppState::default();
    state
        .active_session_mut()
        .push_entry(crate::ChatEntry::user("hello")); // ordinal 0
    state
        .active_session_mut()
        .push_entry(crate::ChatEntry::assistant("world")); // ordinal 1
    state
        .active_session_mut()
        .push_entry(crate::ChatEntry::system("sys")); // ordinal 2 (excluded)
    state
        .active_session_mut()
        .push_entry(crate::ChatEntry::user("q2")); // ordinal 3

    // When opening the fork picker.
    handle_open_picker(&mut state, PickerKind::SessionFork);

    // Then entries have ordinals 0, 1, 3 (skipping system at 2).
    let items = state.frontend.fork_picker.items();
    assert_eq!(items[0].ordinal, 0);
    assert!(items[0].is_user);
    assert_eq!(items[1].ordinal, 1);
    assert!(!items[1].is_user);
    assert_eq!(items[2].ordinal, 3);
    assert!(items[2].is_user);
}

#[rstest::rstest]
fn confirm_fork_picker_emits_fork_command() {
    // Given a state with an active fork picker and a selected entry.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::SessionFork,
    });
    state.frontend.fork_picker.set_items(vec![ForkEntry {
        ordinal: 2,
        text: "hello".to_owned(),
        is_user: true,
        theme: default_theme(),
    }]);
    let source_id = state.session.active_session_id().clone();

    // When confirming picker.
    let (result, maybe_intent) = handle_picker_confirm(&mut state);

    // Then session is loading.
    assert!(state.session.is_loading());
    // And a SessionForkRequested command is returned.
    assert!(result.commands.iter().any(|c| matches!(
        c,
        Command::SessionForkRequested(crate::SessionForkRequested {
            source_session_id,
            at_ordinal: 2,
        }) if source_session_id == &source_id
    )));
    // And picker is closed.
    assert!(!state.frontend.scope_stack.is_picker());
    assert!(maybe_intent.is_none());
}

#[rstest::rstest]
fn confirm_fork_picker_noop_with_no_selection() {
    // Given a state with an active fork picker but no items.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::SessionFork,
    });

    // When confirming picker.
    let (result, maybe_intent) = handle_picker_confirm(&mut state);

    // Then no commands and no intent.
    assert!(result.commands.is_empty());
    assert!(maybe_intent.is_none());
}

// --- Fork filter toggles ---

#[rstest::rstest]
fn toggle_fork_user_filter_removes_user_entries() {
    // Given a state with an active fork picker containing user and assistant entries.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::SessionFork,
    });
    state.frontend.all_fork_entries = vec![
        ForkEntry {
            ordinal: 0,
            text: "user msg".to_owned(),
            is_user: true,
            theme: default_theme(),
        },
        ForkEntry {
            ordinal: 1,
            text: "asst msg".to_owned(),
            is_user: false,
            theme: default_theme(),
        },
        ForkEntry {
            ordinal: 2,
            text: "user msg 2".to_owned(),
            is_user: true,
            theme: default_theme(),
        },
    ];
    state
        .frontend
        .fork_picker
        .set_items(state.frontend.all_fork_entries.clone());

    // When toggling user filter off.
    let result = handle_toggle_fork_user_filter(&mut state);

    // Then the picker has only assistant entries.
    assert!(!state.frontend.fork_show_user);
    assert!(state.frontend.fork_show_assistant);
    assert_eq!(state.frontend.fork_picker.items().len(), 1);
    assert!(!state.frontend.fork_picker.items()[0].is_user);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn toggle_fork_assistant_filter_removes_assistant_entries() {
    // Given a state with an active fork picker.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::SessionFork,
    });
    state.frontend.all_fork_entries = vec![
        ForkEntry {
            ordinal: 0,
            text: "user msg".to_owned(),
            is_user: true,
            theme: default_theme(),
        },
        ForkEntry {
            ordinal: 1,
            text: "asst msg".to_owned(),
            is_user: false,
            theme: default_theme(),
        },
    ];
    state
        .frontend
        .fork_picker
        .set_items(state.frontend.all_fork_entries.clone());

    // When toggling assistant filter off.
    let result = handle_toggle_fork_assistant_filter(&mut state);

    // Then the picker has only user entries.
    assert!(state.frontend.fork_show_user);
    assert!(!state.frontend.fork_show_assistant);
    assert_eq!(state.frontend.fork_picker.items().len(), 1);
    assert!(state.frontend.fork_picker.items()[0].is_user);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn toggle_fork_filter_noop_when_not_fork_picker() {
    // Given a state with a non-fork picker active.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Provider,
    });

    // When toggling fork filters.
    let result = handle_toggle_fork_user_filter(&mut state);

    // Then nothing changed.
    assert!(state.frontend.fork_show_user);
    assert!(result.commands.is_empty());
}

#[rstest::rstest]
fn toggling_both_filters_off_results_in_empty_picker() {
    // Given a state with an active fork picker.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::SessionFork,
    });
    state.frontend.all_fork_entries = vec![
        ForkEntry {
            ordinal: 0,
            text: "user".to_owned(),
            is_user: true,
            theme: default_theme(),
        },
        ForkEntry {
            ordinal: 1,
            text: "asst".to_owned(),
            is_user: false,
            theme: default_theme(),
        },
    ];
    state
        .frontend
        .fork_picker
        .set_items(state.frontend.all_fork_entries.clone());

    // When toggling both filters off.
    handle_toggle_fork_user_filter(&mut state);
    handle_toggle_fork_assistant_filter(&mut state);

    // Then the picker is empty.
    assert!(!state.frontend.fork_show_user);
    assert!(!state.frontend.fork_show_assistant);
    assert!(state.frontend.fork_picker.items().is_empty());
}

#[rstest::rstest]
fn toggling_user_filter_twice_restores_entries() {
    // Given a state with an active fork picker.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::SessionFork,
    });
    state.frontend.all_fork_entries = vec![
        ForkEntry {
            ordinal: 0,
            text: "user".to_owned(),
            is_user: true,
            theme: default_theme(),
        },
        ForkEntry {
            ordinal: 1,
            text: "asst".to_owned(),
            is_user: false,
            theme: default_theme(),
        },
    ];
    state
        .frontend
        .fork_picker
        .set_items(state.frontend.all_fork_entries.clone());

    // When toggling user filter twice.
    handle_toggle_fork_user_filter(&mut state);
    handle_toggle_fork_user_filter(&mut state);

    // Then all entries are restored.
    assert!(state.frontend.fork_show_user);
    assert_eq!(state.frontend.fork_picker.items().len(), 2);
}

// --- Paste ---

#[rstest::rstest]
fn handle_picker_paste_inserts_text_into_filter() {
    use super::intent::handle_picker_paste;

    // Given an active provider picker.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Provider,
    });
    state.provider.provider_picker.set_items(vec![]);

    // When pasting "hello".
    handle_picker_paste(&mut state, "hello");

    // Then the filter contains "hello".
    assert_eq!(state.provider.provider_picker.filter(), "hello");
}

#[rstest::rstest]
fn handle_picker_paste_strips_newlines() {
    use super::intent::handle_picker_paste;

    // Given an active provider picker.
    let mut state = AppState::default();
    state.frontend.scope_stack.push(FocusScope::Picker {
        kind: PickerKind::Provider,
    });
    state.provider.provider_picker.set_items(vec![]);

    // When pasting "hello\nworld".
    handle_picker_paste(&mut state, "hello\nworld");

    // Then newlines are stripped from the filter.
    assert_eq!(state.provider.provider_picker.filter(), "helloworld");
}
