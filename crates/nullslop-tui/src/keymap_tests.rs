use nullslop_domain::Intent;
use nullslop_domain::{Key, KeyEvent, Modifiers, PickerKind, TabDirection};
use ratatui_which_key::Key as _;

use crate::keymap::{collect_all_bindings, collect_bindings_for_scope, init};
use crate::scope::Scope;

// --- Normal scope: key sequence resolution ---

#[rstest::rstest]
fn g_shows_in_which_key_with_general_description() {
    // Given the keymap.
    let keymap = init();

    // When getting bindings for Normal scope.
    let bindings = keymap.bindings_for_scope(Scope::Normal);

    // Find the 'g' binding across all groups.
    let g_binding = bindings
        .iter()
        .flat_map(|g| g.bindings.iter())
        .find(|b| b.key.display() == "g");

    // Then 'g' is present with description "general".
    assert!(
        g_binding.is_some(),
        "'g' binding should appear in Normal scope"
    );
    assert_eq!(g_binding.unwrap().description, "general");
}

#[rstest::rstest]
fn leader_sm_produces_open_picker_provider() {
    // Given the keymap.
    let keymap = init();

    // When looking up '<leader>' then 's' then 'm'.
    let space_key = KeyEvent {
        key: Key::Char(' '),
        modifiers: Modifiers::none(),
    };
    let s_key = KeyEvent {
        key: Key::Char('s'),
        modifiers: Modifiers::none(),
    };
    let m_key = KeyEvent {
        key: Key::Char('m'),
        modifiers: Modifiers::none(),
    };

    let node = keymap.get_node_at_path(&[space_key, s_key, m_key]);

    // Then it's a leaf with the OpenPicker Provider intent.
    assert!(node.is_some());
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Normal);
        assert!(entry.is_some());
        let cmd = &entry.unwrap().action;
        assert!(
            matches!(cmd, Intent::OpenPicker { kind } if *kind == PickerKind::Provider),
            "expected OpenPicker Provider, got {cmd:?}"
        );
    } else {
        panic!("Expected leaf node for '<leader>sm'");
    }
}

#[rstest::rstest]
fn gmr_produces_refresh_models_intent() {
    // Given the keymap.
    let keymap = init();

    // When looking up 'g' then 'm' then 'r'.
    let g_key = KeyEvent {
        key: Key::Char('g'),
        modifiers: Modifiers::none(),
    };
    let m_key = KeyEvent {
        key: Key::Char('m'),
        modifiers: Modifiers::none(),
    };
    let r_key = KeyEvent {
        key: Key::Char('r'),
        modifiers: Modifiers::none(),
    };

    let node = keymap.get_node_at_path(&[g_key, m_key, r_key]);

    // Then it's a leaf with the RefreshModels intent.
    assert!(node.is_some());
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Normal);
        assert!(entry.is_some());
        let cmd = &entry.unwrap().action;
        assert!(
            matches!(cmd, Intent::RefreshModels),
            "expected RefreshModels, got {cmd:?}"
        );
    } else {
        panic!("Expected leaf node for 'gmr'");
    }
}

// --- New bindings: j/k line scroll, gg/G scroll to top/bottom ---

#[rstest::rstest]
fn j_produces_chat_entry_select_next() {
    // Given the keymap.
    let keymap = init();

    // When looking up 'j'.
    let j_key = KeyEvent {
        key: Key::Char('j'),
        modifiers: Modifiers::none(),
    };
    let node = keymap.get_node_at_path(&[j_key]);

    // Then it's a leaf with ChatEntrySelectNext in Normal scope.
    assert!(node.is_some());
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Normal);
        assert!(entry.is_some());
        assert!(matches!(entry.unwrap().action, Intent::ChatEntrySelectNext));
    } else {
        panic!("Expected leaf node for 'j'");
    }
}

#[rstest::rstest]
fn k_produces_chat_entry_select_prev() {
    // Given the keymap.
    let keymap = init();

    // When looking up 'k'.
    let k_key = KeyEvent {
        key: Key::Char('k'),
        modifiers: Modifiers::none(),
    };
    let node = keymap.get_node_at_path(&[k_key]);

    // Then it's a leaf with ChatEntrySelectPrev in Normal scope.
    assert!(node.is_some());
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Normal);
        assert!(entry.is_some());
        assert!(matches!(entry.unwrap().action, Intent::ChatEntrySelectPrev));
    } else {
        panic!("Expected leaf node for 'k'");
    }
}

#[rstest::rstest]
fn gg_produces_scroll_to_top() {
    // Given the keymap.
    let keymap = init();

    // When looking up 'g' then 'g'.
    let g_key = KeyEvent {
        key: Key::Char('g'),
        modifiers: Modifiers::none(),
    };
    let node = keymap.get_node_at_path(&[g_key.clone(), g_key]);

    // Then it's a leaf with ScrollToTop.
    assert!(node.is_some());
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Normal);
        assert!(entry.is_some());
        assert!(matches!(entry.unwrap().action, Intent::ScrollToTop));
    } else {
        panic!("Expected leaf node for 'gg'");
    }
}

#[rstest::rstest]
fn uppercase_g_produces_scroll_to_bottom() {
    // Given the keymap.
    let keymap = init();

    // When looking up 'G' (uppercase).
    let g_key = KeyEvent {
        key: Key::Char('G'),
        modifiers: Modifiers::none(),
    };
    let node = keymap.get_node_at_path(&[g_key]);

    // Then it's a leaf with ScrollToBottom.
    assert!(node.is_some());
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Normal);
        assert!(entry.is_some());
        assert!(matches!(entry.unwrap().action, Intent::ScrollToBottom));
    } else {
        panic!("Expected leaf node for 'G'");
    }
}

// --- Tab switching: Tab/Shift+Tab ---

#[rstest::rstest]
fn tab_produces_switch_tab_next() {
    // Given the keymap.
    let keymap = init();

    // When looking up '<tab>'.
    let tab_key = KeyEvent {
        key: Key::Tab,
        modifiers: Modifiers::none(),
    };
    let node = keymap.get_node_at_path(&[tab_key]);

    // Then it's a leaf with SwitchTab Next.
    assert!(node.is_some());
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Normal);
        assert!(entry.is_some());
        assert!(
            matches!(&entry.unwrap().action, Intent::SwitchTab { direction } if *direction == TabDirection::Next),
            "expected SwitchTab Next"
        );
    } else {
        panic!("Expected leaf node for '<tab>'");
    }
}

#[rstest::rstest]
fn shift_tab_produces_switch_tab_prev() {
    // Given the keymap.
    let keymap = init();

    // When looking up '<s-tab>'.
    let stab_key = KeyEvent {
        key: Key::Tab,
        modifiers: Modifiers::shift(),
    };
    let node = keymap.get_node_at_path(&[stab_key]);

    // Then it's a leaf with SwitchTab Prev.
    assert!(node.is_some());
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Normal);
        assert!(entry.is_some());
        assert!(
            matches!(&entry.unwrap().action, Intent::SwitchTab { direction } if *direction == TabDirection::Prev),
            "expected SwitchTab Prev"
        );
    } else {
        panic!("Expected leaf node for '<s-tab>'");
    }
}

// --- Category assignments ---

#[rstest::rstest]
fn normal_scope_general_category_has_quit_and_help() {
    // Given the keymap.
    let keymap = init();

    // When getting bindings grouped by category for Normal scope.
    let groups = keymap.bindings_for_scope(Scope::Normal);
    let general = groups.iter().find(|g| g.category == "General");

    // Then the General group contains quit and help bindings.
    assert!(general.is_some(), "General category should exist");
    let descs: Vec<&str> = general
        .unwrap()
        .bindings
        .iter()
        .map(|b| b.description.as_str())
        .collect();
    assert!(descs.contains(&"quit"), "General should contain quit");
    assert!(
        descs.contains(&"toggle which-key"),
        "General should contain toggle which-key"
    );
}

#[rstest::rstest]
fn normal_scope_mode_category_contains_set_mode_input() {
    // Given the keymap.
    let keymap = init();

    // When getting bindings grouped by category for Normal scope.
    let groups = keymap.bindings_for_scope(Scope::Normal);
    let input = groups.iter().find(|g| g.category == "Input");

    // Then the Input group exists and contains 'i' → enter insert mode.
    assert!(input.is_some(), "Input category should exist");
    let descs: Vec<&str> = input
        .unwrap()
        .bindings
        .iter()
        .map(|b| b.description.as_str())
        .collect();
    assert!(
        descs.iter().any(|d| d.contains("insert mode")),
        "Input should contain enter insert mode, found: {descs:?}"
    );
}

#[rstest::rstest]
fn navigation_has_scroll_bindings() {
    // Given the keymap.
    let keymap = init();

    // When getting bindings grouped by category for Normal scope.
    let groups = keymap.bindings_for_scope(Scope::Normal);
    let nav = groups.iter().find(|g| g.category == "Navigation");

    // Then the Navigation group contains scroll bindings.
    assert!(nav.is_some(), "Navigation category should exist");
    let descs: Vec<&str> = nav
        .unwrap()
        .bindings
        .iter()
        .map(|b| b.description.as_str())
        .collect();
    assert!(
        descs.contains(&"scroll up"),
        "Navigation should contain scroll up"
    );
    assert!(
        descs.contains(&"scroll down"),
        "Navigation should contain scroll down"
    );
}

#[rstest::rstest]
fn navigation_has_tab_bindings() {
    // Given the keymap.
    let keymap = init();

    // When getting bindings grouped by category for Normal scope.
    let groups = keymap.bindings_for_scope(Scope::Normal);
    let nav = groups.iter().find(|g| g.category == "Navigation");

    // Then the Navigation group contains tab bindings.
    assert!(nav.is_some(), "Navigation category should exist");
    let descs: Vec<&str> = nav
        .unwrap()
        .bindings
        .iter()
        .map(|b| b.description.as_str())
        .collect();
    assert!(
        descs.iter().any(|d| d.contains("tab")),
        "Navigation should contain tab switch"
    );
}

#[rstest::rstest]
fn gm_prefix_appears_under_model_category() {
    // Given the keymap.
    let keymap = init();

    // When navigating into the 'g' prefix in Normal scope.
    let g_key = KeyEvent {
        key: Key::Char('g'),
        modifiers: Modifiers::none(),
    };
    let children = keymap
        .get_children_at_path(&[g_key], &Scope::Normal)
        .expect("g prefix should have children");

    // Then 'm' is one of the children with description "model".
    let m_child = children.iter().find(|(k, _)| k.display() == "m");
    assert!(m_child.is_some(), "'m' should be a child of 'g'");
    assert_eq!(m_child.unwrap().1, "model");
}

#[rstest::rstest]
fn g_prefix_appears_under_general_category() {
    // Given the keymap.
    let keymap = init();

    // When getting bindings grouped by category for Normal scope.
    let groups = keymap.bindings_for_scope(Scope::Normal);
    let general = groups.iter().find(|g| g.category == "General");

    // Then the General group contains 'g' with description "general".
    assert!(general.is_some(), "General category should exist");
    let g_binding = general
        .unwrap()
        .bindings
        .iter()
        .find(|b| b.key.display() == "g");
    assert!(
        g_binding.is_some(),
        "General category should contain 'g' prefix"
    );
    assert_eq!(g_binding.unwrap().description, "general");
}

#[rstest::rstest]
fn gcr_produces_rescan_prompt_templates() {
    // Given the keymap.
    let keymap = init();

    // When looking up 'g' then 'c' then 'r'.
    let g_key = KeyEvent {
        key: Key::Char('g'),
        modifiers: Modifiers::none(),
    };
    let c_key = KeyEvent {
        key: Key::Char('c'),
        modifiers: Modifiers::none(),
    };
    let r_key = KeyEvent {
        key: Key::Char('r'),
        modifiers: Modifiers::none(),
    };

    let node = keymap.get_node_at_path(&[g_key, c_key, r_key]);

    // Then it's a leaf with the RescanPromptTemplates intent.
    assert!(node.is_some());
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Normal);
        assert!(entry.is_some());
        let cmd = &entry.unwrap().action;
        assert!(
            matches!(cmd, Intent::RescanPromptTemplates),
            "expected RescanPromptTemplates, got {cmd:?}"
        );
    } else {
        panic!("Expected leaf node for 'gcr'");
    }
}

#[rstest::rstest]
fn gc_prefix_appears_under_general_category() {
    // Given the keymap.
    let keymap = init();

    // When navigating into the 'g' prefix in Normal scope.
    let g_key = KeyEvent {
        key: Key::Char('g'),
        modifiers: Modifiers::none(),
    };
    let children = keymap
        .get_children_at_path(&[g_key], &Scope::Normal)
        .expect("g prefix should have children");

    // Then 'c' is one of the children with description "context".
    let c_child = children.iter().find(|(k, _)| k.display() == "c");
    assert!(c_child.is_some(), "'c' should be a child of 'g'");
    assert_eq!(c_child.unwrap().1, "context");
}

#[rstest::rstest]
fn dashboard_j_produces_dashboard_select_down() {
    // Given the keymap.
    let keymap = init();

    // When looking up 'j' in Dashboard scope.
    let j_key = KeyEvent {
        key: Key::Char('j'),
        modifiers: Modifiers::none(),
    };
    let node = keymap.get_node_at_path(&[j_key]);

    // Then it's a leaf with DashboardSelectDown for Dashboard scope.
    assert!(node.is_some());
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Dashboard);
        assert!(entry.is_some());
        assert!(matches!(entry.unwrap().action, Intent::DashboardSelectDown));
    } else {
        panic!("Expected leaf node for 'j' in Dashboard scope");
    }
}

#[rstest::rstest]
fn dashboard_k_produces_dashboard_select_up() {
    // Given the keymap.
    let keymap = init();

    // When looking up 'k' in Dashboard scope.
    let k_key = KeyEvent {
        key: Key::Char('k'),
        modifiers: Modifiers::none(),
    };
    let node = keymap.get_node_at_path(&[k_key]);

    // Then it's a leaf with DashboardSelectUp for Dashboard scope.
    assert!(node.is_some());
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Dashboard);
        assert!(entry.is_some());
        assert!(matches!(entry.unwrap().action, Intent::DashboardSelectUp));
    } else {
        panic!("Expected leaf node for 'k' in Dashboard scope");
    }
}

#[rstest::rstest]
fn dashboard_gg_produces_dashboard_select_first() {
    // Given the keymap.
    let keymap = init();

    // When looking up 'gg' in Dashboard scope.
    let g_key = KeyEvent {
        key: Key::Char('g'),
        modifiers: Modifiers::none(),
    };
    let node = keymap.get_node_at_path(&[g_key.clone(), g_key]);

    // Then it's a leaf with DashboardSelectFirst for Dashboard scope.
    assert!(node.is_some());
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Dashboard);
        assert!(entry.is_some());
        assert!(matches!(
            entry.unwrap().action,
            Intent::DashboardSelectFirst
        ));
    } else {
        panic!("Expected leaf node for 'gg' in Dashboard scope");
    }
}

#[rstest::rstest]
fn dashboard_uppercase_g_produces_dashboard_select_last() {
    // Given the keymap.
    let keymap = init();

    // When looking up 'G' in Dashboard scope.
    let g_key = KeyEvent {
        key: Key::Char('G'),
        modifiers: Modifiers::none(),
    };
    let node = keymap.get_node_at_path(&[g_key]);

    // Then it's a leaf with DashboardSelectLast for Dashboard scope.
    assert!(node.is_some());
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Dashboard);
        assert!(entry.is_some());
        assert!(matches!(entry.unwrap().action, Intent::DashboardSelectLast));
    } else {
        panic!("Expected leaf node for 'G' in Dashboard scope");
    }
}

#[rstest::rstest]
fn gcs_produces_open_picker_context_assembly() {
    // Given the keymap.
    let keymap = init();

    // When looking up 'g' then 'c' then 's'.
    let g_key = KeyEvent {
        key: Key::Char('g'),
        modifiers: Modifiers::none(),
    };
    let c_key = KeyEvent {
        key: Key::Char('c'),
        modifiers: Modifiers::none(),
    };
    let s_key = KeyEvent {
        key: Key::Char('s'),
        modifiers: Modifiers::none(),
    };

    let node = keymap.get_node_at_path(&[g_key, c_key, s_key]);

    // Then it's a leaf with the OpenPicker ContextAssembly intent.
    assert!(node.is_some());
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Normal);
        assert!(entry.is_some());
        let cmd = &entry.unwrap().action;
        assert!(
            matches!(cmd, Intent::OpenPicker { kind } if *kind == PickerKind::ContextAssembly),
            "expected OpenPicker ContextAssembly, got {cmd:?}"
        );
    } else {
        panic!("Expected leaf node for 'gcs'");
    }
}

#[rstest::rstest]
fn gc_prefix_appears_under_context_category() {
    // Given the keymap.
    let keymap = init();

    // When navigating into the 'g' prefix in Normal scope.
    let g_key = KeyEvent {
        key: Key::Char('g'),
        modifiers: Modifiers::none(),
    };
    let children = keymap
        .get_children_at_path(&[g_key], &Scope::Normal)
        .expect("g prefix should have children");

    // Then 'c' is one of the children with description "context".
    let c_child = children.iter().find(|(k, _)| k.display() == "c");
    assert!(c_child.is_some(), "'c' should be a child of 'g'");
    assert_eq!(c_child.unwrap().1, "context");
}

#[rstest::rstest]
fn input_scope_escape_appears_under_general_category() {
    // Given the keymap.
    let keymap = init();

    // When getting bindings grouped by category for Input scope.
    let groups = keymap.bindings_for_scope(Scope::Input);
    let general = groups.iter().find(|g| g.category == "General");

    // Then the General group contains '<esc>' → enter normal mode.
    assert!(general.is_some(), "General category should exist");
    let descs: Vec<&str> = general
        .unwrap()
        .bindings
        .iter()
        .map(|b| b.description.as_str())
        .collect();
    assert!(
        descs.iter().any(|d| d.contains("normal")),
        "General should contain set mode normal, found: {descs:?}"
    );
}

// --- Tree walker tests ---

#[rstest::rstest]
fn quit_binding_present_in_normal_scope() {
    // Given the keymap.
    let keymap = init();

    // When collecting bindings for Normal scope.
    let entries = collect_bindings_for_scope(&keymap, &Scope::Normal);

    // Then the quit binding 'q' is present.
    let q_entry = entries.iter().find(|e| e.key_sequence == "q");
    assert!(q_entry.is_some(), "'q' should be in Normal scope bindings");
}

#[rstest::rstest]
fn quit_binding_has_correct_key() {
    // Given the keymap.
    let keymap = init();

    // When collecting bindings for Normal scope.
    let entries = collect_bindings_for_scope(&keymap, &Scope::Normal);

    // Then the quit binding has correct description, scope, and intent.
    let q_entry = entries.iter().find(|e| e.key_sequence == "q");
    assert!(q_entry.is_some(), "'q' should be in Normal scope bindings");
    let entry = q_entry.unwrap();
    assert_eq!(entry.description, "quit");
    assert_eq!(entry.scope, "Normal");
    assert!(matches!(entry.command, Intent::Quit));
}

#[rstest::rstest]
fn collect_bindings_for_scope_finds_multi_key_sequence() {
    // Given the keymap.
    let keymap = init();

    // When collecting bindings for Normal scope.
    let entries = collect_bindings_for_scope(&keymap, &Scope::Normal);

    // Then 'gg' (scroll to top) is present.
    let gg_entry = entries.iter().find(|e| e.key_sequence == "gg");
    assert!(
        gg_entry.is_some(),
        "'gg' should be in Normal scope bindings"
    );
    assert_eq!(gg_entry.unwrap().description, "scroll to top");
}

#[rstest::rstest]
fn collect_bindings_for_scope_finds_leader_key_sequence() {
    // Given the keymap.
    let keymap = init();

    // When collecting bindings for Normal scope.
    let entries = collect_bindings_for_scope(&keymap, &Scope::Normal);

    // Then 'Spacesm' (search models) is present.
    let sm_entry = entries.iter().find(|e| e.key_sequence == "Spacesm");
    assert!(
        sm_entry.is_some(),
        "'Spacesm' should be in Normal scope bindings"
    );
    let entry = sm_entry.unwrap();
    assert!(
        matches!(entry.command, Intent::OpenPicker { .. }),
        "expected OpenPicker, got {:?}",
        entry.command
    );
}

#[rstest::rstest]
fn collect_bindings_for_scope_excludes_prefix_only_nodes() {
    // Given the keymap.
    let keymap = init();

    // When collecting bindings for Normal scope.
    let entries = collect_bindings_for_scope(&keymap, &Scope::Normal);

    // Then plain 'g' is NOT present (it's a prefix, not a leaf).
    let g_only = entries.iter().find(|e| e.key_sequence == "g");
    assert!(
        g_only.is_none(),
        "'g' prefix should not appear as a leaf binding"
    );
}

#[rstest::rstest]
fn collect_bindings_for_scope_includes_category() {
    // Given the keymap.
    let keymap = init();

    // When collecting bindings for Normal scope.
    let entries = collect_bindings_for_scope(&keymap, &Scope::Normal);

    // Then 'q' has General category.
    let q_entry = entries.iter().find(|e| e.key_sequence == "q");
    assert!(q_entry.is_some());
    assert_eq!(q_entry.unwrap().category, "General");
}

#[rstest::rstest]
fn collect_bindings_for_scope_separates_scopes() {
    // Given the keymap.
    let keymap = init();

    // When collecting bindings for Dashboard scope.
    let entries = collect_bindings_for_scope(&keymap, &Scope::Dashboard);

    // Then 'j' is "dashboard select down" (not "scroll line down").
    let j_entry = entries.iter().find(|e| e.key_sequence == "j");
    assert!(j_entry.is_some(), "'j' should be in Dashboard scope");
    assert_eq!(j_entry.unwrap().description, "dashboard select down");
}

#[rstest::rstest]
fn bindings_include_normal_scope() {
    // Given the keymap.
    let keymap = init();

    // When collecting all bindings.
    let entries = collect_all_bindings(&keymap);

    // Then entries from Normal and Dashboard scopes are present.
    let normal_entries: Vec<_> = entries.iter().filter(|e| e.scope == "Normal").collect();
    let dashboard_entries: Vec<_> = entries.iter().filter(|e| e.scope == "Dashboard").collect();

    assert!(!normal_entries.is_empty(), "should have Normal entries");
    assert!(
        !dashboard_entries.is_empty(),
        "should have Dashboard entries"
    );
}

#[rstest::rstest]
fn bindings_include_input_scope() {
    // Given the keymap.
    let keymap = init();

    // When collecting all bindings.
    let entries = collect_all_bindings(&keymap);

    // Then entries from Picker and Input scopes are present.
    let picker_entries: Vec<_> = entries.iter().filter(|e| e.scope == "Picker").collect();
    let input_entries: Vec<_> = entries.iter().filter(|e| e.scope == "Input").collect();

    assert!(!picker_entries.is_empty(), "should have Picker entries");
    assert!(!input_entries.is_empty(), "should have Input entries");
}

// --- Keymap picker keybinding tests ---

#[rstest::rstest]
fn ctrl_p_produces_open_picker_keymap() {
    // Given the keymap.
    let keymap = init();

    // When looking up '<c-p>' in Normal scope.
    let ctrl_p = KeyEvent {
        key: Key::Char('p'),
        modifiers: Modifiers::ctrl(),
    };
    let node = keymap.get_node_at_path(&[ctrl_p]);

    // Then it's a leaf with OpenPicker Keymap for Normal scope.
    assert!(node.is_some());
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Normal);
        assert!(entry.is_some());
        assert!(
            matches!(&entry.unwrap().action, Intent::OpenPicker { kind } if *kind == PickerKind::Keymap),
            "expected OpenPicker Keymap"
        );
    } else {
        panic!("Expected leaf node for '<c-p>'");
    }
}

#[rstest::rstest]
fn ctrl_p_produces_open_picker_keymap_in_input_scope() {
    // Given the keymap.
    let keymap = init();

    // When looking up '<c-p>' in Input scope.
    let ctrl_p = KeyEvent {
        key: Key::Char('p'),
        modifiers: Modifiers::ctrl(),
    };
    let node = keymap.get_node_at_path(&[ctrl_p]);

    // Then it's a leaf with OpenPicker Keymap for Input scope.
    assert!(node.is_some());
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Input);
        assert!(entry.is_some(), "'<c-p>' should be bound in Input scope");
        assert!(
            matches!(&entry.unwrap().action, Intent::OpenPicker { kind } if *kind == PickerKind::Keymap),
            "expected OpenPicker Keymap"
        );
    } else {
        panic!("Expected leaf node for '<c-p>'");
    }
}

#[rstest::rstest]
fn ctrl_p_produces_open_picker_keymap_in_picker_scope() {
    // Given the keymap.
    let keymap = init();

    // When looking up '<c-p>' in Picker scope.
    let ctrl_p = KeyEvent {
        key: Key::Char('p'),
        modifiers: Modifiers::ctrl(),
    };
    let node = keymap.get_node_at_path(&[ctrl_p]);

    // Then it's a leaf with OpenPicker Keymap for Picker scope.
    assert!(node.is_some());
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Picker);
        assert!(entry.is_some(), "'<c-p>' should be bound in Picker scope");
        assert!(
            matches!(&entry.unwrap().action, Intent::OpenPicker { kind } if *kind == PickerKind::Keymap),
            "expected OpenPicker Keymap"
        );
    } else {
        panic!("Expected leaf node for '<c-p>'");
    }
}

#[rstest::rstest]
fn ctrl_p_produces_open_picker_keymap_in_dashboard_scope() {
    // Given the keymap.
    let keymap = init();

    // When looking up '<c-p>' in Dashboard scope.
    let ctrl_p = KeyEvent {
        key: Key::Char('p'),
        modifiers: Modifiers::ctrl(),
    };
    let node = keymap.get_node_at_path(&[ctrl_p]);

    // Then it's a leaf with OpenPicker Keymap for Dashboard scope.
    assert!(node.is_some());
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Dashboard);
        assert!(
            entry.is_some(),
            "'<c-p>' should be bound in Dashboard scope"
        );
        assert!(
            matches!(&entry.unwrap().action, Intent::OpenPicker { kind } if *kind == PickerKind::Keymap),
            "expected OpenPicker Keymap"
        );
    } else {
        panic!("Expected leaf node for '<c-p>'");
    }
}

#[rstest::rstest]
fn leader_sk_produces_open_picker_keymap() {
    // Given the keymap.
    let keymap = init();

    // When looking up '<space>' then 's' then 'k' in Normal scope.
    let space_key = KeyEvent {
        key: Key::Char(' '),
        modifiers: Modifiers::none(),
    };
    let s_key = KeyEvent {
        key: Key::Char('s'),
        modifiers: Modifiers::none(),
    };
    let k_key = KeyEvent {
        key: Key::Char('k'),
        modifiers: Modifiers::none(),
    };

    let node = keymap.get_node_at_path(&[space_key, s_key, k_key]);

    // Then it's a leaf with OpenPicker Keymap for Normal scope.
    assert!(node.is_some(), "'<space>sk' should resolve");
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Normal);
        assert!(
            entry.is_some(),
            "'<space>sk' should be bound in Normal scope"
        );
        assert!(
            matches!(&entry.unwrap().action, Intent::OpenPicker { kind } if *kind == PickerKind::Keymap),
            "expected OpenPicker Keymap"
        );
    } else {
        panic!("Expected leaf node for '<leader>sk'");
    }
}

#[rstest::rstest]
fn leader_sk_produces_open_picker_keymap_in_dashboard() {
    // Given the keymap.
    let keymap = init();

    // When looking up '<space>' then 's' then 'k' in Dashboard scope.
    let space_key = KeyEvent {
        key: Key::Char(' '),
        modifiers: Modifiers::none(),
    };
    let s_key = KeyEvent {
        key: Key::Char('s'),
        modifiers: Modifiers::none(),
    };
    let k_key = KeyEvent {
        key: Key::Char('k'),
        modifiers: Modifiers::none(),
    };

    let node = keymap.get_node_at_path(&[space_key, s_key, k_key]);

    // Then it's a leaf with OpenPicker Keymap for Dashboard scope.
    assert!(node.is_some(), "'<space>sk' should resolve");
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Dashboard);
        assert!(
            entry.is_some(),
            "'<space>sk' should be bound in Dashboard scope"
        );
        assert!(
            matches!(&entry.unwrap().action, Intent::OpenPicker { kind } if *kind == PickerKind::Keymap),
            "expected OpenPicker Keymap"
        );
    } else {
        panic!("Expected leaf node for '<leader>sk'");
    }
}

// --- Scope filter toggle binding ---

#[rstest::rstest]
fn ctrl_a_produces_toggle_keymap_scope_filter() {
    // Given the keymap.
    let keymap = init();

    // When looking up '<c-a>' in Picker scope.
    let ctrl_a = KeyEvent {
        key: Key::Char('a'),
        modifiers: Modifiers::ctrl(),
    };
    let node = keymap.get_node_at_path(&[ctrl_a]);

    // Then it's a leaf with ToggleKeymapScopeFilter for Picker scope.
    assert!(node.is_some());
    if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
        let entry = entries.iter().find(|e| e.scope == Scope::Picker);
        assert!(entry.is_some(), "'<c-a>' should be bound in Picker scope");
        assert!(
            matches!(entry.unwrap().action, Intent::ToggleKeymapScopeFilter),
            "expected ToggleKeymapScopeFilter"
        );
    } else {
        panic!("Expected leaf node for '<c-a>'");
    }
}
