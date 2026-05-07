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
    collect_leaf_bindings(keymap.bindings(), *scope, "", &mut entries);
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
        collect_leaf_bindings(keymap.bindings(), *scope, "", &mut entries);
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
    scope: Scope,
    prefix: &str,
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
                    if entry.scope == scope {
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
                    if entry.scope == scope {
                        let cat = (*branch_category)
                            .unwrap_or(entry.category);
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
                collect_leaf_bindings(branch_children, scope, &full_sequence, out);
            }
        }
    }
}


