//! Keymap configuration and initialization.
//!
//! Defines the key categories and builds the keymap with all scope bindings.
//! Binds keys to [`Command`](nullslop_protocol::Command) variants. Parameterized on
//! [`nullslop_protocol::KeyEvent`] so the keymap works in both TUI and headless modes.

use crossterm::event::{self, MouseEventKind};
use derive_more::Display;
use nullslop_protocol::chat_input::{InsertChar, SubmitMessage};
use nullslop_protocol::provider_picker::PickerInsertChar;
use nullslop_protocol::picker_kind::PickerKind;
use nullslop_protocol::system::{OpenPicker, SetMode};
use nullslop_protocol::tab::SwitchTab;
use nullslop_protocol::{Command, Key, KeyEvent, Mode, SessionId, TabDirection};
use ratatui_which_key::CrosstermKeymapExt as _;
use ratatui_which_key::Key as WhichKeyKey;
use ratatui_which_key::Keymap;

use crate::scope::Scope;

/// Categories for keybinding grouping in the which-key popup.
///
/// Each variant becomes a section header when displaying available shortcuts.
#[derive(Display, Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCategory {
    /// App-level control: quit, interrupt, help.
    General,
    /// Navigation: scrolling, tab switching, picker movement.
    Navigation,
    /// Model management: model picker, model refresh.
    Model,
    /// Text editing: cursor movement, insertion, deletion, mode entry.
    Input,
    /// Context strategy and prompt template management.
    Context,
}

/// Builds and returns the full keymap with all scope bindings.
#[must_use]
#[rustfmt::skip]
#[expect(clippy::too_many_lines, reason = "exhaustive keymap bindings grow with each scope")]
pub fn init() -> Keymap<KeyEvent, Scope, Command, KeyCategory> {
    let mut keymap = Keymap::new();

    keymap
        // Normal scope: navigation and commands
        .scope(Scope::Normal, |b| {
            b
            // General — app control
            .bind("q", Command::Quit, KeyCategory::General)
            .bind("<c-c>", Command::Quit, KeyCategory::General)
            .bind("?", Command::ToggleWhichKey, KeyCategory::General)
            .bind("<c-p>", Command::OpenPicker { payload: OpenPicker { kind: PickerKind::Keymap } }, KeyCategory::General)
            .bind("<leader>sk", Command::OpenPicker { payload: OpenPicker { kind: PickerKind::Keymap } }, KeyCategory::General)
            // Input — enter input mode
            .bind("i", Command::SetMode { payload: SetMode { mode: Mode::Input } }, KeyCategory::Input)
            // Navigation — scrolling and tab switching
            .bind("k", Command::ScrollLineUp, KeyCategory::Navigation)
            .bind("j", Command::ScrollLineDown, KeyCategory::Navigation)
            .bind("<c-u>", Command::ScrollUp, KeyCategory::Navigation)
            .bind("<c-d>", Command::ScrollDown, KeyCategory::Navigation)
            .bind("<tab>", Command::SwitchTab { payload: SwitchTab { direction: TabDirection::Next } }, KeyCategory::Navigation)
            .bind("<s-tab>", Command::SwitchTab { payload: SwitchTab { direction: TabDirection::Prev } }, KeyCategory::Navigation)
            // Input — external editor
            .bind("<c-e>", Command::EditInput, KeyCategory::Input)
            // g prefix — general commands and model management
            .describe_group_with_category("g", "general", KeyCategory::General)
            .describe_group_with_category("gm", "model", KeyCategory::Model)
            .describe_group_with_category("gc", "context", KeyCategory::Context)
            .bind("gg", Command::ScrollToTop, KeyCategory::Navigation)
            .bind("G", Command::ScrollToBottom, KeyCategory::Navigation)
            .bind("gmp", Command::OpenPicker { payload: OpenPicker { kind: PickerKind::Provider } }, KeyCategory::Model)
            .bind("gmr", Command::RefreshModels, KeyCategory::Model)
            .bind("gcs", Command::OpenPicker { payload: OpenPicker { kind: PickerKind::ContextAssembly } }, KeyCategory::Context)
            .bind("gcr", Command::RescanPromptTemplates, KeyCategory::Context)
            // Pane navigation (only meaningful when workflow pane is visible)
            .bind("<c-h>", Command::WorkflowFocusChat, KeyCategory::Navigation)
            .bind("<c-l>", Command::WorkflowFocusWorkflow, KeyCategory::Navigation)
            // Workflow pane toggle
            .bind("<leader>w", Command::WorkflowTogglePane, KeyCategory::General);
        })
        // Dashboard scope: actor list navigation
        .scope(Scope::Dashboard, |b| {
            b
            // General — app control
            .bind("q", Command::Quit, KeyCategory::General)
            .bind("<c-c>", Command::Quit, KeyCategory::General)
            .bind("?", Command::ToggleWhichKey, KeyCategory::General)
            .bind("<c-p>", Command::OpenPicker { payload: OpenPicker { kind: PickerKind::Keymap } }, KeyCategory::General)
            .bind("<leader>sk", Command::OpenPicker { payload: OpenPicker { kind: PickerKind::Keymap } }, KeyCategory::General)
            // Navigation — actor list
            .bind("j", Command::DashboardSelectDown, KeyCategory::Navigation)
            .bind("k", Command::DashboardSelectUp, KeyCategory::Navigation)
            .bind("gg", Command::DashboardSelectFirst, KeyCategory::Navigation)
            .bind("G", Command::DashboardSelectLast, KeyCategory::Navigation)
            // Tab switching
            .bind("<tab>", Command::SwitchTab { payload: SwitchTab { direction: TabDirection::Next } }, KeyCategory::Navigation)
            .bind("<s-tab>", Command::SwitchTab { payload: SwitchTab { direction: TabDirection::Prev } }, KeyCategory::Navigation)
            // Pane navigation and workflow toggle (no-op outside Chat tab, but prevents confusion)
            .bind("<c-h>", Command::WorkflowFocusChat, KeyCategory::Navigation)
            .bind("<c-l>", Command::WorkflowFocusWorkflow, KeyCategory::Navigation)
            .bind("<leader>w", Command::WorkflowTogglePane, KeyCategory::General);
        })
        // Workflow scope: workflow step list navigation
        .scope(Scope::Workflow, |b| {
            b
            // General — app control
            .bind("q", Command::Quit, KeyCategory::General)
            .bind("<c-c>", Command::Quit, KeyCategory::General)
            .bind("?", Command::ToggleWhichKey, KeyCategory::General)
            // Navigation — step list
            .bind("j", Command::WorkflowSelectDown, KeyCategory::Navigation)
            .bind("k", Command::WorkflowSelectUp, KeyCategory::Navigation)
            .bind("gg", Command::WorkflowSelectFirst, KeyCategory::Navigation)
            .bind("G", Command::WorkflowSelectLast, KeyCategory::Navigation)
            .bind("r", Command::WorkflowRestartStep, KeyCategory::General)
            .bind("a", Command::WorkflowApproveStep, KeyCategory::General)
            .bind("D", Command::WorkflowToggleDetail, KeyCategory::General)
            // Tab switching
            .bind("<tab>", Command::SwitchTab { payload: SwitchTab { direction: TabDirection::Next } }, KeyCategory::Navigation)
            .bind("<s-tab>", Command::SwitchTab { payload: SwitchTab { direction: TabDirection::Prev } }, KeyCategory::Navigation)
            // Pane navigation
            .bind("<c-h>", Command::WorkflowFocusChat, KeyCategory::Navigation)
            .bind("<c-l>", Command::WorkflowFocusWorkflow, KeyCategory::Navigation)
            // Input — external editor
            .bind("<c-e>", Command::EditInput, KeyCategory::Input);
        })
        // Input scope: typing into the input buffer
        .scope(Scope::Input, |b| {
            b.bind("<enter>", Command::SubmitMessage { payload: SubmitMessage { session_id: SessionId::new(), text: String::new() } }, KeyCategory::Input)
            .bind("<s-enter>", Command::InsertChar { payload: InsertChar { ch: '\n' } }, KeyCategory::Input)
            .bind("<c-enter>", Command::InsertChar { payload: InsertChar { ch: '\n' } }, KeyCategory::Input)
            .bind("<esc>", Command::SetMode { payload: SetMode { mode: Mode::Normal } }, KeyCategory::General)
            .bind("<c-c>", Command::Interrupt, KeyCategory::General)
            .bind("<c-e>", Command::EditInput, KeyCategory::Input)
            .bind("<f1>", Command::ToggleWhichKey, KeyCategory::General)
            .bind("<backspace>", Command::DeleteGrapheme, KeyCategory::Input)
            .bind("<left>", Command::MoveCursorLeft, KeyCategory::Input)
            .bind("<right>", Command::MoveCursorRight, KeyCategory::Input)
            .bind("<home>", Command::MoveCursorToStart, KeyCategory::Input)
            .bind("<end>", Command::MoveCursorToEnd, KeyCategory::Input)
            .bind("<delete>", Command::DeleteGraphemeForward, KeyCategory::Input)
            .bind("<c-left>", Command::MoveCursorWordLeft, KeyCategory::Input)
            .bind("<c-right>", Command::MoveCursorWordRight, KeyCategory::Input)
            .bind("<up>", Command::MoveCursorUp, KeyCategory::Input)
            .bind("<down>", Command::MoveCursorDown, KeyCategory::Input)
            .bind("<tab>", Command::AutocompleteConfirm, KeyCategory::Input)
            .bind("<c-u>", Command::ScrollUp, KeyCategory::Navigation)
            .bind("<c-d>", Command::ScrollDown, KeyCategory::Navigation)
            .bind("<c-p>", Command::OpenPicker { payload: OpenPicker { kind: PickerKind::Keymap } }, KeyCategory::General)
            .catch_all(|key: KeyEvent| {
                if let Key::Char(c) = key.key {
                    Some(Command::InsertChar {
                        payload: InsertChar { ch: c },
                    })
                } else {
                    None
                }
            });
        });

    keymap
        .scope(Scope::Picker, |b| {
            b.bind("<esc>", Command::SetMode { payload: SetMode { mode: Mode::Normal } }, KeyCategory::General)
            .bind("<enter>", Command::PickerConfirm, KeyCategory::Model)
            .bind("<up>", Command::PickerMoveUp, KeyCategory::Navigation)
            .bind("<down>", Command::PickerMoveDown, KeyCategory::Navigation)
            .bind("<left>", Command::PickerMoveCursorLeft, KeyCategory::Input)
            .bind("<right>", Command::PickerMoveCursorRight, KeyCategory::Input)
            .bind("<backspace>", Command::PickerBackspace, KeyCategory::Input)
            .bind("<c-r>", Command::RefreshModels, KeyCategory::Model)
            .bind("<c-p>", Command::OpenPicker { payload: OpenPicker { kind: PickerKind::Keymap } }, KeyCategory::General)
            .bind("<c-a>", Command::ToggleKeymapScopeFilter, KeyCategory::General)
            .catch_all(|key: KeyEvent| {
                if let Key::Char(c) = key.key {
                    Some(Command::PickerInsertChar {
                        payload: PickerInsertChar { ch: c },
                    })
                } else {
                    None
                }
            });
        });

    keymap.on_mouse(|mouse: event::MouseEvent, _scope: &Scope| {
        match mouse.kind {
            MouseEventKind::ScrollUp => Some(Command::MouseScrollUp),
            MouseEventKind::ScrollDown => Some(Command::MouseScrollDown),
            _ => None,
        }
    })
}

/// Collects all fully-resolved leaf bindings from the keymap for a given scope.
///
/// Walks the keymap tree recursively, collecting only leaf entries (no prefix-only
/// branch nodes). Each entry includes the full key sequence, description, scope,
/// category, and the command it triggers.
pub fn collect_bindings_for_scope(
    keymap: &Keymap<KeyEvent, Scope, Command, KeyCategory>,
    scope: &Scope,
) -> Vec<nullslop_component::keymap_picker::KeymapEntry> {
    let mut entries = Vec::new();
    collect_leaf_bindings(keymap.bindings(), scope, String::new(), &mut entries);
    entries
}

/// Collects fully-resolved leaf bindings from all scopes.
///
/// Iterates over all known scopes and collects entries from each one.
pub fn collect_all_bindings(
    keymap: &Keymap<KeyEvent, Scope, Command, KeyCategory>,
) -> Vec<nullslop_component::keymap_picker::KeymapEntry> {
    let mut entries = Vec::new();
    for scope in &[Scope::Normal, Scope::Dashboard, Scope::Picker, Scope::Input] {
        collect_leaf_bindings(keymap.bindings(), scope, String::new(), &mut entries);
    }
    entries
}

/// Recursively walks the keybinding tree, collecting fully-resolved leaf entries.
///
/// Only `KeyNode::Leaf` entries are collected — prefix-only branch nodes like `g`
/// (which lead to sub-menus) are not included since they are not actionable.
/// Branch nodes that also have `leaf_entries` for the given scope are included
/// (those represent keys that are both a prefix and a terminal in different scopes).
fn collect_leaf_bindings(
    children: &[ratatui_which_key::KeyChild<KeyEvent, Scope, Command, KeyCategory>],
    scope: &Scope,
    prefix: String,
    out: &mut Vec<nullslop_component::keymap_picker::KeymapEntry>,
) {
    for child in children {
        let key_display = WhichKeyKey::display(&child.key);
        let full_sequence = if prefix.is_empty() {
            key_display.clone()
        } else {
            format!("{prefix}{key_display}")
        };

        match &child.node {
            ratatui_which_key::KeyNode::Leaf(entries) => {
                for entry in entries {
                    if entry.scope == *scope {
                        out.push(nullslop_component::keymap_picker::KeymapEntry {
                            key_sequence: full_sequence.clone(),
                            description: entry.description.clone(),
                            scope: entry.scope.to_string(),
                            category: entry.category.to_string(),
                            command: entry.action.clone(),
                            search_text: format!(
                                "{} {}",
                                full_sequence, entry.description
                            ),
                        });
                    }
                }
            }
            ratatui_which_key::KeyNode::Branch {
                children: branch_children,
                leaf_entries,
                category: branch_category,
                ..
            } => {
                // Collect leaf entries attached to this branch for the given scope.
                // These represent keys that act as both a prefix and a terminal action
                // in different scopes.
                for entry in leaf_entries {
                    if entry.scope == *scope {
                        let cat = branch_category
                            .clone()
                            .unwrap_or_else(|| entry.category.clone());
                        out.push(nullslop_component::keymap_picker::KeymapEntry {
                            key_sequence: full_sequence.clone(),
                            description: entry.description.clone(),
                            scope: entry.scope.to_string(),
                            category: cat.to_string(),
                            command: entry.action.clone(),
                            search_text: format!(
                                "{} {}",
                                full_sequence, entry.description
                            ),
                        });
                    }
                }

                // Recurse into children.
                collect_leaf_bindings(branch_children, scope, full_sequence, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use nullslop_protocol::Modifiers;
    use ratatui_which_key::Key as _;

    use super::*;
    use crate::scope::Scope;

    // --- Normal scope: key sequence resolution ---

    #[test]
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

    #[test]
    fn gmp_produces_open_picker_provider() {
        // Given the keymap.
        let keymap = init();

        // When looking up 'g' then 'm' then 'p'.
        let g_key = KeyEvent {
            key: Key::Char('g'),
            modifiers: Modifiers::none(),
        };
        let m_key = KeyEvent {
            key: Key::Char('m'),
            modifiers: Modifiers::none(),
        };
        let p_key = KeyEvent {
            key: Key::Char('p'),
            modifiers: Modifiers::none(),
        };

        let node = keymap.get_node_at_path(&[g_key, m_key, p_key]);

        // Then it's a leaf with the OpenPicker Provider command.
        assert!(node.is_some());
        if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
            let entry = entries.iter().find(|e| e.scope == Scope::Normal);
            assert!(entry.is_some());
            let cmd = &entry.unwrap().action;
            assert!(
                matches!(cmd, Command::OpenPicker { payload } if payload.kind == PickerKind::Provider),
                "expected OpenPicker Provider, got {cmd:?}"
            );
        } else {
            panic!("Expected leaf node for 'gmp'");
        }
    }

    #[test]
    fn gmr_produces_refresh_models_command() {
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

        // Then it's a leaf with the RefreshModels command.
        assert!(node.is_some());
        if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
            let entry = entries.iter().find(|e| e.scope == Scope::Normal);
            assert!(entry.is_some());
            let cmd = &entry.unwrap().action;
            assert!(
                matches!(cmd, Command::RefreshModels),
                "expected RefreshModels, got {cmd:?}"
            );
        } else {
            panic!("Expected leaf node for 'gmr'");
        }
    }

    // --- New bindings: j/k line scroll, gg/G scroll to top/bottom ---

    #[test]
    fn j_produces_scroll_line_down() {
        // Given the keymap.
        let keymap = init();

        // When looking up 'j'.
        let j_key = KeyEvent {
            key: Key::Char('j'),
            modifiers: Modifiers::none(),
        };
        let node = keymap.get_node_at_path(&[j_key]);

        // Then it's a leaf with ScrollLineDown.
        assert!(node.is_some());
        if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
            let entry = entries.iter().find(|e| e.scope == Scope::Normal);
            assert!(entry.is_some());
            assert!(matches!(entry.unwrap().action, Command::ScrollLineDown));
        } else {
            panic!("Expected leaf node for 'j'");
        }
    }

    #[test]
    fn k_produces_scroll_line_up() {
        // Given the keymap.
        let keymap = init();

        // When looking up 'k'.
        let k_key = KeyEvent {
            key: Key::Char('k'),
            modifiers: Modifiers::none(),
        };
        let node = keymap.get_node_at_path(&[k_key]);

        // Then it's a leaf with ScrollLineUp.
        assert!(node.is_some());
        if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
            let entry = entries.iter().find(|e| e.scope == Scope::Normal);
            assert!(entry.is_some());
            assert!(matches!(entry.unwrap().action, Command::ScrollLineUp));
        } else {
            panic!("Expected leaf node for 'k'");
        }
    }

    #[test]
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
            assert!(matches!(entry.unwrap().action, Command::ScrollToTop));
        } else {
            panic!("Expected leaf node for 'gg'");
        }
    }

    #[test]
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
            assert!(matches!(entry.unwrap().action, Command::ScrollToBottom));
        } else {
            panic!("Expected leaf node for 'G'");
        }
    }

    // --- Tab switching: Tab/Shift+Tab ---

    #[test]
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
                matches!(&entry.unwrap().action, Command::SwitchTab { payload } if payload.direction == TabDirection::Next),
                "expected SwitchTab Next"
            );
        } else {
            panic!("Expected leaf node for '<tab>'");
        }
    }

    #[test]
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
                matches!(&entry.unwrap().action, Command::SwitchTab { payload } if payload.direction == TabDirection::Prev),
                "expected SwitchTab Prev"
            );
        } else {
            panic!("Expected leaf node for '<s-tab>'");
        }
    }

    // --- Category assignments ---

    #[test]
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

    #[test]
    fn normal_scope_mode_category_contains_set_mode_input() {
        // Given the keymap.
        let keymap = init();

        // When getting bindings grouped by category for Normal scope.
        let groups = keymap.bindings_for_scope(Scope::Normal);
        let input = groups.iter().find(|g| g.category == "Input");

        // Then the Input group exists and contains 'i' → set mode input.
        assert!(input.is_some(), "Input category should exist");
        let descs: Vec<&str> = input
            .unwrap()
            .bindings
            .iter()
            .map(|b| b.description.as_str())
            .collect();
        assert!(
            descs.iter().any(|d| d.contains("input")),
            "Input should contain set mode input"
        );
    }

    #[test]
    fn normal_scope_navigation_category_has_scroll_and_tab() {
        // Given the keymap.
        let keymap = init();

        // When getting bindings grouped by category for Normal scope.
        let groups = keymap.bindings_for_scope(Scope::Normal);
        let nav = groups.iter().find(|g| g.category == "Navigation");

        // Then the Navigation group contains scroll and tab bindings.
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
        assert!(
            descs.iter().any(|d| d.contains("tab")),
            "Navigation should contain tab switch"
        );
    }

    #[test]
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

    #[test]
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

    #[test]
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

        // Then it's a leaf with the RescanPromptTemplates command.
        assert!(node.is_some());
        if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
            let entry = entries.iter().find(|e| e.scope == Scope::Normal);
            assert!(entry.is_some());
            let cmd = &entry.unwrap().action;
            assert!(
                matches!(cmd, Command::RescanPromptTemplates),
                "expected RescanPromptTemplates, got {cmd:?}"
            );
        } else {
            panic!("Expected leaf node for 'gcr'");
        }
    }

    #[test]
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

    #[test]
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
            assert!(matches!(
                entry.unwrap().action,
                Command::DashboardSelectDown
            ));
        } else {
            panic!("Expected leaf node for 'j' in Dashboard scope");
        }
    }

    #[test]
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
            assert!(matches!(entry.unwrap().action, Command::DashboardSelectUp));
        } else {
            panic!("Expected leaf node for 'k' in Dashboard scope");
        }
    }

    #[test]
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
                Command::DashboardSelectFirst
            ));
        } else {
            panic!("Expected leaf node for 'gg' in Dashboard scope");
        }
    }

    #[test]
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
            assert!(matches!(
                entry.unwrap().action,
                Command::DashboardSelectLast
            ));
        } else {
            panic!("Expected leaf node for 'G' in Dashboard scope");
        }
    }

    #[test]
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

        // Then it's a leaf with the OpenPicker ContextAssembly command.
        assert!(node.is_some());
        if let Some(ratatui_which_key::KeyNode::Leaf(entries)) = node {
            let entry = entries.iter().find(|e| e.scope == Scope::Normal);
            assert!(entry.is_some());
            let cmd = &entry.unwrap().action;
            assert!(
                matches!(cmd, Command::OpenPicker { payload } if payload.kind == PickerKind::ContextAssembly),
                "expected OpenPicker ContextAssembly, got {cmd:?}"
            );
        } else {
            panic!("Expected leaf node for 'gcs'");
        }
    }

    #[test]
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

    #[test]
    fn input_scope_escape_appears_under_general_category() {
        // Given the keymap.
        let keymap = init();

        // When getting bindings grouped by category for Input scope.
        let groups = keymap.bindings_for_scope(Scope::Input);
        let general = groups.iter().find(|g| g.category == "General");

        // Then the General group contains '<esc>' → set mode normal.
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

    #[test]
    fn collect_bindings_for_scope_finds_single_key_leaf() {
        // Given the keymap.
        let keymap = init();

        // When collecting bindings for Normal scope.
        let entries = super::collect_bindings_for_scope(&keymap, &Scope::Normal);

        // Then the quit binding 'q' is present.
        let q_entry = entries.iter().find(|e| e.key_sequence == "q");
        assert!(q_entry.is_some(), "'q' should be in Normal scope bindings");
        let entry = q_entry.unwrap();
        assert_eq!(entry.description, "quit");
        assert_eq!(entry.scope, "Normal");
        assert!(matches!(entry.command, Command::Quit));
    }

    #[test]
    fn collect_bindings_for_scope_finds_multi_key_sequence() {
        // Given the keymap.
        let keymap = init();

        // When collecting bindings for Normal scope.
        let entries = super::collect_bindings_for_scope(&keymap, &Scope::Normal);

        // Then 'gg' (scroll to top) is present.
        let gg_entry = entries.iter().find(|e| e.key_sequence == "gg");
        assert!(gg_entry.is_some(), "'gg' should be in Normal scope bindings");
        assert_eq!(gg_entry.unwrap().description, "scroll to top");
    }

    #[test]
    fn collect_bindings_for_scope_finds_three_key_sequence() {
        // Given the keymap.
        let keymap = init();

        // When collecting bindings for Normal scope.
        let entries = super::collect_bindings_for_scope(&keymap, &Scope::Normal);

        // Then 'gmp' (open provider picker) is present.
        let gmp_entry = entries.iter().find(|e| e.key_sequence == "gmp");
        assert!(gmp_entry.is_some(), "'gmp' should be in Normal scope bindings");
        let entry = gmp_entry.unwrap();
        assert!(
            matches!(entry.command, Command::OpenPicker { .. }),
            "expected OpenPicker, got {:?}",
            entry.command
        );
    }

    #[test]
    fn collect_bindings_for_scope_excludes_prefix_only_nodes() {
        // Given the keymap.
        let keymap = init();

        // When collecting bindings for Normal scope.
        let entries = super::collect_bindings_for_scope(&keymap, &Scope::Normal);

        // Then plain 'g' is NOT present (it's a prefix, not a leaf).
        let g_only = entries.iter().find(|e| e.key_sequence == "g");
        assert!(
            g_only.is_none(),
            "'g' prefix should not appear as a leaf binding"
        );
    }

    #[test]
    fn collect_bindings_for_scope_includes_category() {
        // Given the keymap.
        let keymap = init();

        // When collecting bindings for Normal scope.
        let entries = super::collect_bindings_for_scope(&keymap, &Scope::Normal);

        // Then 'q' has General category.
        let q_entry = entries.iter().find(|e| e.key_sequence == "q");
        assert!(q_entry.is_some());
        assert_eq!(q_entry.unwrap().category, "General");
    }

    #[test]
    fn collect_bindings_for_scope_separates_scopes() {
        // Given the keymap.
        let keymap = init();

        // When collecting bindings for Dashboard scope.
        let entries = super::collect_bindings_for_scope(&keymap, &Scope::Dashboard);

        // Then 'j' is "dashboard select down" (not "scroll line down").
        let j_entry = entries.iter().find(|e| e.key_sequence == "j");
        assert!(j_entry.is_some(), "'j' should be in Dashboard scope");
        assert_eq!(j_entry.unwrap().description, "dashboard select down");
    }

    #[test]
    fn collect_all_bindings_includes_multiple_scopes() {
        // Given the keymap.
        let keymap = init();

        // When collecting all bindings.
        let entries = super::collect_all_bindings(&keymap);

        // Then entries from multiple scopes are present.
        let normal_entries: Vec<_> = entries.iter().filter(|e| e.scope == "Normal").collect();
        let dashboard_entries: Vec<_> = entries.iter().filter(|e| e.scope == "Dashboard").collect();
        let picker_entries: Vec<_> = entries.iter().filter(|e| e.scope == "Picker").collect();
        let input_entries: Vec<_> = entries.iter().filter(|e| e.scope == "Input").collect();

        assert!(!normal_entries.is_empty(), "should have Normal entries");
        assert!(!dashboard_entries.is_empty(), "should have Dashboard entries");
        assert!(!picker_entries.is_empty(), "should have Picker entries");
        assert!(!input_entries.is_empty(), "should have Input entries");
    }

    // --- Keymap picker keybinding tests ---

    #[test]
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
                matches!(&entry.unwrap().action, Command::OpenPicker { payload } if payload.kind == PickerKind::Keymap),
                "expected OpenPicker Keymap"
            );
        } else {
            panic!("Expected leaf node for '<c-p>'");
        }
    }

    #[test]
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
                matches!(&entry.unwrap().action, Command::OpenPicker { payload } if payload.kind == PickerKind::Keymap),
                "expected OpenPicker Keymap"
            );
        } else {
            panic!("Expected leaf node for '<c-p>'");
        }
    }

    #[test]
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
                matches!(&entry.unwrap().action, Command::OpenPicker { payload } if payload.kind == PickerKind::Keymap),
                "expected OpenPicker Keymap"
            );
        } else {
            panic!("Expected leaf node for '<c-p>'");
        }
    }

    #[test]
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
            assert!(entry.is_some(), "'<c-p>' should be bound in Dashboard scope");
            assert!(
                matches!(&entry.unwrap().action, Command::OpenPicker { payload } if payload.kind == PickerKind::Keymap),
                "expected OpenPicker Keymap"
            );
        } else {
            panic!("Expected leaf node for '<c-p>'");
        }
    }

    #[test]
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
            assert!(entry.is_some(), "'<space>sk' should be bound in Normal scope");
            assert!(
                matches!(&entry.unwrap().action, Command::OpenPicker { payload } if payload.kind == PickerKind::Keymap),
                "expected OpenPicker Keymap"
            );
        } else {
            panic!("Expected leaf node for '<leader>sk'");
        }
    }

    #[test]
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
            assert!(entry.is_some(), "'<space>sk' should be bound in Dashboard scope");
            assert!(
                matches!(&entry.unwrap().action, Command::OpenPicker { payload } if payload.kind == PickerKind::Keymap),
                "expected OpenPicker Keymap"
            );
        } else {
            panic!("Expected leaf node for '<leader>sk'");
        }
    }

    // --- Scope filter toggle binding ---

    #[test]
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
                matches!(entry.unwrap().action, Command::ToggleKeymapScopeFilter),
                "expected ToggleKeymapScopeFilter"
            );
        } else {
            panic!("Expected leaf node for '<c-a>'");
        }
    }
}
