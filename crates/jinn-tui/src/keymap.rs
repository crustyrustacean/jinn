//! Keymap configuration and initialization.
//!
//! Defines the key categories and builds the keymap with all scope bindings.
//! Binds keys to [`Intent`] variants. Parameterized on
//! [`KeyEvent`] so the keymap works in both TUI and headless modes.

use crossterm::event::{self, MouseEventKind};
use derive_more::Display;
use jinn_domain::Intent;
use jinn_domain::PickerKind;
use jinn_domain::protocol::CwdRoot;
use jinn_domain::{Key, KeyEvent};
use ratatui_which_key::CrosstermKeymapExt as _;
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
    /// Sidebar sections
    Sidebar,
    /// Chat history
    ChatHistory,
}

/// Builds and returns the full keymap with all scope bindings.
/// Adds shared sidebar keybindings common to all sidebar section scopes.
///
/// Includes: quit, help, navigation (j/k/J/K), escape, tab switching,
/// pane navigation, sidebar resize, and input mode entry.
fn add_sidebar_base(b: &mut ratatui_which_key::ScopeBuilder<KeyEvent, Scope, Intent, KeyCategory>) {
    b
        // General - app control
        .bind("q", Intent::Quit, KeyCategory::General)
        .bind("<c-c>", Intent::Quit, KeyCategory::General)
        .bind("?", Intent::ToggleWhichkey, KeyCategory::General)
        // Navigation - within section and between sections
        .bind("j", Intent::SidebarMoveDown, KeyCategory::Navigation)
        .bind("k", Intent::SidebarMoveUp, KeyCategory::Navigation)
        .bind("J", Intent::SidebarSectionNext, KeyCategory::Navigation)
        .bind("K", Intent::SidebarSectionPrev, KeyCategory::Navigation)
        .bind("<esc>", Intent::SidebarLeave, KeyCategory::General)
        // Pane navigation - focus back to chat
        .bind("<c-h>", Intent::SidebarLeave, KeyCategory::Navigation)
        // Sidebar resize
        .bind("<c-w>", Intent::SidebarResizeEnter, KeyCategory::Navigation)
        // Input - enter input mode
        .bind("i", Intent::EnterInsertMode, KeyCategory::Input)
        // Direct jump to Sessions section
        .bind(
            "<M-s>",
            Intent::SidebarFocusSessions,
            KeyCategory::Navigation,
        );
}

/// Adds shared picker keybindings common to all picker scopes.
///
/// Includes: escape, confirm, navigation (up/down), cursor (left/right),
/// backspace, new session, and catch-all char input.
fn add_picker_base(b: &mut ratatui_which_key::ScopeBuilder<KeyEvent, Scope, Intent, KeyCategory>) {
    b.bind("<esc>", Intent::EnterNormalMode, KeyCategory::General)
        .bind("<enter>", Intent::PickerConfirm, KeyCategory::Model)
        .bind("<up>", Intent::PickerMoveUp, KeyCategory::Navigation)
        .bind("<down>", Intent::PickerMoveDown, KeyCategory::Navigation)
        .bind("<left>", Intent::PickerMoveCursorLeft, KeyCategory::Input)
        .bind("<right>", Intent::PickerMoveCursorRight, KeyCategory::Input)
        .bind("<backspace>", Intent::PickerBackspace, KeyCategory::Input)
        .bind("<c-n>", Intent::SessionNew, KeyCategory::General)
        .bind("<c-c>", Intent::CtrlClear, KeyCategory::General)
        .catch_all(|key: KeyEvent| {
            if let Key::Char(c) = key.key {
                Some(Intent::PickerInsertChar { ch: c })
            } else {
                None
            }
        });
}

/// Builds and returns the full keymap with all scope bindings.
#[must_use]
#[rustfmt::skip]
#[expect(clippy::too_many_lines, reason = "exhaustive keymap bindings grow with each scope")]
pub fn init() -> Keymap<KeyEvent, Scope, Intent, KeyCategory> {
    let mut keymap = Keymap::new();

    keymap
        // Normal scope: navigation and commands
        .scope(Scope::Normal, |b| {
            b
            // General - app control
            .bind("q", Intent::Quit, KeyCategory::General)
            .bind("<c-c>", Intent::Quit, KeyCategory::General)
            .bind("?", Intent::ToggleWhichkey, KeyCategory::General)
            .describe_group_with_category("<leader>s", "search", KeyCategory::General)
            .bind("<leader>sm", Intent::OpenPicker { kind: PickerKind::Provider }, KeyCategory::General)
            .bind("<leader>ss", Intent::OpenPicker { kind: PickerKind::Session }, KeyCategory::General)
            .bind("<leader>se", Intent::OpenPicker { kind: PickerKind::Persona }, KeyCategory::General)
            .bind("<leader>st", Intent::OpenPicker { kind: PickerKind::Tool }, KeyCategory::General)
            .bind("<leader>sk", Intent::OpenPicker { kind: PickerKind::Skill }, KeyCategory::General)
            .bind("<leader>sh", Intent::OpenPicker { kind: PickerKind::Theme }, KeyCategory::General)
            .bind("<leader>sr", Intent::OpenPicker { kind: PickerKind::ReasoningEffort }, KeyCategory::General)
            // Projects - curated directory list for quick session creation
            .bind("<leader>so", Intent::OpenPicker { kind: PickerKind::Project }, KeyCategory::General)
            // Input - enter input mode
            .bind("i", Intent::EnterInsertMode, KeyCategory::Input)
            .bind("<c-j>", Intent::EnterInsertMode, KeyCategory::Input)
            // Navigation - scrolling and tab switching
            .bind("k", Intent::ChatEntrySelectPrev, KeyCategory::Navigation)
            .bind("j", Intent::ChatEntrySelectNext, KeyCategory::Navigation)
            .bind("<Tab>", Intent::SwitchTab, KeyCategory::Navigation)

            .bind("<c-u>", Intent::ScrollUp, KeyCategory::Navigation)
            .bind("<c-d>", Intent::ScrollDown, KeyCategory::Navigation)
            // Change CWD - search from session CWD
            .bind("<M-c>", Intent::ChangeCwd { root: CwdRoot::Session }, KeyCategory::Navigation)
            // Change CWD - search from home directory
            .bind("<M-d>", Intent::ChangeCwd { root: CwdRoot::Home }, KeyCategory::Navigation)
            // g prefix - general commands and model management
            .describe_group_with_category("g", "general", KeyCategory::General)
            .describe_group_with_category("gm", "model", KeyCategory::Model)
            .describe_group_with_category("gc", "context", KeyCategory::Context)
            .describe_group_with_category("gd", "discord", KeyCategory::General)
            .bind("<leader>sl", Intent::OpenPicker { kind: PickerKind::SessionLifecycle }, KeyCategory::General)
            .bind("<leader>sc", Intent::OpenPicker { kind: PickerKind::CompactionModel }, KeyCategory::Model)
            .describe_group_with_category("<leader>c", "change", KeyCategory::General)
            .bind("<leader>cd", Intent::OpenCwdInput, KeyCategory::General)
            .bind("gg", Intent::ScrollToTop, KeyCategory::Navigation)
            .bind("G", Intent::ScrollToBottom, KeyCategory::Navigation)
            .bind("gmr", Intent::RefreshModels, KeyCategory::Model)
            .bind("gcr", Intent::RescanPromptTemplates, KeyCategory::Context)
            .bind("gcp", Intent::OpenPrunerAccumulationInput, KeyCategory::Context)
            .bind("gdc", Intent::ToDiscordThread, KeyCategory::General)
            .bind("<c-l>", Intent::SidebarFocus, KeyCategory::Navigation)
            .bind("<M-s>", Intent::SidebarFocusSessions, KeyCategory::Navigation)
            // Sidebar resize
            .bind("<c-w>", Intent::SidebarResizeEnter, KeyCategory::Navigation)
            // Minimap navigation
            // Pin selected entry
            .bind("p", Intent::ChatEntryPinSelected, KeyCategory::ChatHistory)
            .bind("x", Intent::ChatEntryIgnoreSelected, KeyCategory::ChatHistory)
            // Reset selected entry to default context
            .bind("r", Intent::ChatEntryResetSelected, KeyCategory::ChatHistory)
            // Expand/collapse tool entry
            .bind("e", Intent::ExpandToolEntry, KeyCategory::ChatHistory)
            // Toggle audit popup for the selected entry
            .bind("a", Intent::ToggleAuditPopup, KeyCategory::ChatHistory)
            // Toggle ignored block visibility
            .bind("h", Intent::ToggleIgnoredBlockVisibility, KeyCategory::ChatHistory)
            // Fork session from selected entry
            .bind("f", Intent::ForkFromEntry, KeyCategory::ChatHistory)
            // Yank (copy) selected entry to clipboard
            .bind("y", Intent::YankSelectedEntry, KeyCategory::ChatHistory)
            // Jump to next/previous compaction summary entry
            .describe_group_with_category("]", "next", KeyCategory::ChatHistory)
            .describe_group_with_category("[", "previous", KeyCategory::ChatHistory)
            .bind("]c", Intent::ChatEntryJumpNextCompaction, KeyCategory::ChatHistory)
            .bind("[c", Intent::ChatEntryJumpPrevCompaction, KeyCategory::ChatHistory)
            .bind("]u", Intent::ChatEntryJumpNextUserEntry, KeyCategory::ChatHistory)
            .bind("[u", Intent::ChatEntryJumpPrevUserEntry, KeyCategory::ChatHistory)
            .bind("]p", Intent::ChatEntryJumpNextPinned, KeyCategory::ChatHistory)
            .bind("[p", Intent::ChatEntryJumpPrevPinned, KeyCategory::ChatHistory)
            // Session creation
            .bind("n", Intent::SessionNew, KeyCategory::General)
            .bind("N", Intent::SessionNewWithLifecycle, KeyCategory::General)
            // Escape: cancel selection
            .bind("<esc>", Intent::NormalEscape, KeyCategory::General)
            // Unmapped character keys produce NoOp to dismiss confirmation prompts
            .catch_all(|key: KeyEvent| {
                if let Key::Char(_) = key.key {
                    Some(Intent::NoOp)
                } else {
                    None
                }
            });
        })
        // Sidebar - Persona section
        .scope(Scope::SidebarPersona, |b| {
            add_sidebar_base(b);
            b
            // Persona-specific actions
            .bind("c", Intent::SidebarPersonaEdit, KeyCategory::Sidebar);
        })
        // Sidebar - Pins section
        .scope(Scope::SidebarPins, |b| {
            add_sidebar_base(b);
            b
            // Pin management actions
            .bind("u", Intent::PinsUnpin, KeyCategory::Sidebar)
            .bind("t", Intent::PinsPinTop, KeyCategory::Sidebar)
            .bind("b", Intent::PinsPinBottom, KeyCategory::Sidebar)
            .bind("r", Intent::PinsPinRelative, KeyCategory::Sidebar)
            .bind("m", Intent::PinsPinCycle, KeyCategory::Sidebar)
            // Leave sidebar to Normal at the pin's position (same as <c-h>/<esc>).
            .bind("<enter>", Intent::SidebarLeave, KeyCategory::General);
        })
        // Sidebar - Sessions section
        .scope(Scope::SidebarSessions, |b| {
            add_sidebar_base(b);
            b
            // Session management actions
            .bind("x", Intent::SidebarSessionClose, KeyCategory::Sidebar)
            .bind("t", Intent::SidebarSessionTeardown, KeyCategory::Sidebar)
            .describe_group_with_category("p", "sessions", KeyCategory::Sidebar)
            .bind("<enter>", Intent::SidebarSessionConfirm, KeyCategory::Sidebar)
            .bind("n", Intent::SessionNew, KeyCategory::Sidebar)
            .bind("N", Intent::SessionNewWithLifecycle, KeyCategory::Sidebar)
            .bind("r", Intent::SidebarRenameSession, KeyCategory::Sidebar)
            .bind("a", Intent::SidebarSessionArchive, KeyCategory::Sidebar)
            .bind("c", Intent::SidebarSessionContinue, KeyCategory::Sidebar)
            .bind("s", Intent::SidebarSessionRerunSetup, KeyCategory::Sidebar)

            // i activates session and enters insert mode
            .bind("i", Intent::SidebarConfirmInsert, KeyCategory::Sidebar)
            // Unmapped character keys produce NoOp to dismiss confirmation prompts
            .catch_all(|key: KeyEvent| {
                if let Key::Char(_) = key.key {
                    Some(Intent::NoOp)
                } else {
                    None
                }
            });
        })
        // Sidebar - Task list section
        .scope(Scope::SidebarTaskList, |b| {
            add_sidebar_base(b);
            // Open full-screen task list browser
            b.bind(
                "s",
                Intent::OpenPicker { kind: jinn_domain::feat::picker::PickerKind::TaskList },
                KeyCategory::Sidebar,
            )
            // Scroll the task list preview popup (left of the sidebar).
            .bind(
                "<pgup>",
                Intent::TaskListPreviewScrollUp,
                KeyCategory::Navigation,
            )
            .bind(
                "<pgdn>",
                Intent::TaskListPreviewScrollDown,
                KeyCategory::Navigation,
            );
            b.bind(
                "s",
                Intent::OpenPicker { kind: jinn_domain::feat::picker::PickerKind::TaskList },
                KeyCategory::Sidebar,
            );
        })
        // Input scope: typing into the input buffer
        .scope(Scope::Input, |b| {
            b.bind("<enter>", Intent::SubmitMessage, KeyCategory::Input)
                .bind("<M-q>", Intent::ToggleInputMode, KeyCategory::Input)
                .bind("<M-s>", Intent::SidebarFocusSessions, KeyCategory::Navigation)
            .bind("<s-enter>", Intent::InsertChar { ch: '\n' }, KeyCategory::Input)
            .bind("<c-enter>", Intent::InsertChar { ch: '\n' }, KeyCategory::Input)
            .bind("<esc>", Intent::EnterNormalMode, KeyCategory::General)
            .bind("<c-k>", Intent::EnterNormalMode, KeyCategory::General)
            .bind("<c-c>", Intent::CtrlClear, KeyCategory::General)
            .bind("<c-e>", Intent::EditInput, KeyCategory::Input)
            // <c-g> consensus one-shot removed (workflow system deprecated)
            // Change CWD - search from session CWD
            .bind("<M-c>", Intent::ChangeCwd { root: CwdRoot::Session }, KeyCategory::Navigation)
            // Change CWD - search from home directory
            .bind("<M-d>", Intent::ChangeCwd { root: CwdRoot::Home }, KeyCategory::Navigation)
            .bind("<f1>", Intent::ToggleWhichkey, KeyCategory::General)
            .bind("<backspace>", Intent::DeleteGrapheme, KeyCategory::Input)
            .bind("<left>", Intent::MoveCursorLeft, KeyCategory::Input)
            .bind("<right>", Intent::MoveCursorRight, KeyCategory::Input)
            .bind("<home>", Intent::MoveCursorToStart, KeyCategory::Input)
            .bind("<end>", Intent::MoveCursorToEnd, KeyCategory::Input)
            .bind("<delete>", Intent::DeleteGraphemeForward, KeyCategory::Input)
            .bind("<c-left>", Intent::MoveCursorWordLeft, KeyCategory::Input)
            .bind("<c-right>", Intent::MoveCursorWordRight, KeyCategory::Input)
            .bind("<up>", Intent::MoveCursorUp, KeyCategory::Input)
            .bind("<down>", Intent::MoveCursorDown, KeyCategory::Input)
            .bind("<tab>", Intent::AutocompleteConfirm, KeyCategory::Input)
            .bind("<c-u>", Intent::ScrollUp, KeyCategory::Navigation)
            .bind("<c-d>", Intent::ScrollDown, KeyCategory::Navigation)
            .bind("<c-l>", Intent::SidebarFocus, KeyCategory::Navigation)

            .bind("<c-j>", Intent::InsertChar { ch: '\n' }, KeyCategory::Input)
            .catch_all(|key: KeyEvent| {
                if let Key::Char(c) = key.key {
                    Some(Intent::InsertChar { ch: c })
                } else {
                    None
                }
            });
        });

    // Picker scopes - each picker kind has its own scope for kind-specific bindings.
    // Shared bindings (navigation, confirm, escape, char input) are in add_picker_base.
    keymap
        .scope(Scope::PickerProvider, |b| {
            add_picker_base(b);
            b.bind("<Tab>", Intent::ModelToggleSelected, KeyCategory::General);
            b.bind("<c-a>", Intent::ToggleAlloyMode, KeyCategory::Model);
            b.bind("<c-r>", Intent::RefreshModels, KeyCategory::Model);
        })
        .scope(Scope::PickerSession, |b| {
            add_picker_base(b);
        })
        .scope(Scope::PickerPersona, |b| {
            add_picker_base(b);
        })
        .scope(Scope::PickerTheme, |b| {
            add_picker_base(b);
        })
        .scope(Scope::PickerLifecycle, |b| {
            add_picker_base(b);
        })

        .scope(Scope::PickerCompactionModel, |b| {
            add_picker_base(b);
        })

        .scope(Scope::PickerReasoningEffort, |b| {
            add_picker_base(b);
        })
        .scope(Scope::PickerTool, |b| {
            add_picker_base(b);
            b.bind("<Tab>", Intent::ToolToggleSelected, KeyCategory::General);
        })
        .scope(Scope::PickerSkill, |b| {
            add_picker_base(b);
            b.bind("<Tab>", Intent::SkillToggleSelected, KeyCategory::General)
             .bind("<pgup>", Intent::PreviewScrollUp, KeyCategory::Navigation)
             .bind("<pgdn>", Intent::PreviewScrollDown, KeyCategory::Navigation)
             .bind("<c-r>", Intent::RefreshSkills, KeyCategory::General);
        })
        .scope(Scope::PickerTaskList, |b| {
            add_picker_base(b);
        })
        .scope(Scope::PickerProject, |b| {
            add_picker_base(b);
            b.bind("<c-enter>", Intent::ProjectNewAtHighlightedWithLifecycle, KeyCategory::General)
             .bind("<c-n>", Intent::OpenProjectAddInput, KeyCategory::General)
             .bind("<c-d>", Intent::ProjectRemoveHighlighted, KeyCategory::General);
        });

    // Dashboard scope - service status overview.
    keymap.scope(Scope::Dashboard, |b| {
        b
        .bind("<Tab>", Intent::SwitchTab, KeyCategory::General)
        .bind("j", Intent::DashboardSelectDown, KeyCategory::Navigation)
        .bind("k", Intent::DashboardSelectUp, KeyCategory::Navigation)
        .bind("g", Intent::DashboardSelectFirst, KeyCategory::Navigation)
        .bind("G", Intent::DashboardSelectLast, KeyCategory::Navigation)
        .bind("q", Intent::Quit, KeyCategory::General)
        .bind("<esc>", Intent::SwitchTab, KeyCategory::General)
        .bind("?", Intent::ToggleWhichkey, KeyCategory::General);
    });

    // ArgInput scope - typing positional args for a lifecycle command.
    keymap.scope(Scope::ArgInput, |b| {
        b.bind("<esc>", Intent::EnterNormalMode, KeyCategory::General)
        .bind("<enter>", Intent::ArgInputConfirm, KeyCategory::Input)
        .bind("<left>", Intent::MoveCursorLeft, KeyCategory::Input)
        .bind("<right>", Intent::MoveCursorRight, KeyCategory::Input)
        .bind("<backspace>", Intent::DeleteGrapheme, KeyCategory::Input)
        .bind("<delete>", Intent::DeleteGraphemeForward, KeyCategory::Input)
        .bind("<c-j>", Intent::InsertChar { ch: '\n' }, KeyCategory::Input)
        .bind("<c-c>", Intent::CtrlClear, KeyCategory::General)
        .catch_all(|key: KeyEvent| {
            if let Key::Char(c) = key.key {
                Some(Intent::InsertChar { ch: c })
            } else {
                None
            }
        });
    });

    // SidebarResize scope - adjusting sidebar width.
    keymap.scope(Scope::SidebarResize, |b| {
        b
        .bind("h", Intent::SidebarResizeExpand, KeyCategory::Sidebar)
        .bind("l", Intent::SidebarResizeContract, KeyCategory::Sidebar)
        .bind("<esc>", Intent::SidebarResizeLeave, KeyCategory::Sidebar)
        .bind("<c-c>", Intent::Quit, KeyCategory::General);
    });

    // RenameSessionInput scope - editing a session title.
    keymap.scope(Scope::RenameSessionInput, |b| {
        b
        .bind("<esc>", Intent::RenameSessionLeave, KeyCategory::General)
        .bind("<enter>", Intent::RenameSessionConfirm, KeyCategory::Input)
        .bind("<left>", Intent::RenameCursorLeft, KeyCategory::Input)
        .bind("<right>", Intent::RenameCursorRight, KeyCategory::Input)
        .bind("<backspace>", Intent::RenameDeleteGrapheme, KeyCategory::Input)
        .bind("<delete>", Intent::RenameDeleteForward, KeyCategory::Input)
        .bind("<c-j>", Intent::RenameInsertChar { ch: '\n' }, KeyCategory::Input)
        .bind("<c-c>", Intent::CtrlClear, KeyCategory::General)
        .catch_all(|key: KeyEvent| {
            if let Key::Char(c) = key.key {
                Some(Intent::RenameInsertChar { ch: c })
            } else {
                None
            }
        });
    });

    // PrunerAccumulationInput scope — numeric-only threshold input.
    keymap.scope(Scope::PrunerAccumulationInput, |b| {
        b
        .bind("<esc>", Intent::PrunerAccumulationLeave, KeyCategory::General)
        .bind("<enter>", Intent::PrunerAccumulationConfirm, KeyCategory::Input)
        .bind("<left>", Intent::PrunerAccumulationCursorLeft, KeyCategory::Input)
        .bind("<right>", Intent::PrunerAccumulationCursorRight, KeyCategory::Input)
        .bind("<backspace>", Intent::PrunerAccumulationDeleteGrapheme, KeyCategory::Input)
        .bind("<delete>", Intent::PrunerAccumulationDeleteForward, KeyCategory::Input)
        .bind("<c-c>", Intent::CtrlClear, KeyCategory::General)
        .catch_all(|key: KeyEvent| {
            if let Key::Char(c) = key.key {
                Some(Intent::PrunerAccumulationInsertChar { ch: c })
            } else {
                None
            }
        });
    });

    // CwdInput scope - typing a directory path (mirrors ArgInput).
    keymap.scope(Scope::CwdInput, |b| {
        b.bind("<esc>", Intent::CwdInputLeave, KeyCategory::General)
            .bind("<enter>", Intent::CwdInputConfirm, KeyCategory::Input)
            .bind("<left>", Intent::MoveCursorLeft, KeyCategory::Input)
            .bind("<right>", Intent::MoveCursorRight, KeyCategory::Input)
            .bind("<backspace>", Intent::DeleteGrapheme, KeyCategory::Input)
            .bind("<delete>", Intent::DeleteGraphemeForward, KeyCategory::Input)
            .bind("<c-j>", Intent::InsertChar { ch: '\n' }, KeyCategory::Input)
            .bind("<c-c>", Intent::CtrlClear, KeyCategory::General)
            .catch_all(|key: KeyEvent| {
                if let Key::Char(c) = key.key {
                    Some(Intent::InsertChar { ch: c })
                } else {
                    None
                }
            });
    });

    // ProjectAddInput scope - clone of CwdInput, specialized for registering
    // a new project directory from inside the project picker (<c-n>).
    keymap.scope(Scope::ProjectAddInput, |b| {
        b.bind("<esc>", Intent::ProjectAddInputLeave, KeyCategory::General)
            .bind("<enter>", Intent::ProjectAddInputConfirm, KeyCategory::Input)
            .bind("<left>", Intent::MoveCursorLeft, KeyCategory::Input)
            .bind("<right>", Intent::MoveCursorRight, KeyCategory::Input)
            .bind("<backspace>", Intent::DeleteGrapheme, KeyCategory::Input)
            .bind("<delete>", Intent::DeleteGraphemeForward, KeyCategory::Input)
            .bind("<c-j>", Intent::InsertChar { ch: '\n' }, KeyCategory::Input)
            .bind("<c-c>", Intent::CtrlClear, KeyCategory::General)
            .catch_all(|key: KeyEvent| {
                if let Key::Char(c) = key.key {
                    Some(Intent::InsertChar { ch: c })
                } else {
                    None
                }
            });
    });

    // Quake Bar scope — the global overlay console. Captures every keystroke;
    // only <esc> dismisses it. The opening <M-`> keybind is registered as a
    // global binding (top-level, below) so it survives every scope — including
    // the Input scope, whose catch-all would otherwise swallow it as a backtick.
    keymap.scope(Scope::QuakeBar, |b| {
        b.bind("<esc>", Intent::CloseQuakeBar, KeyCategory::General)
            .bind("<M-`>", Intent::CloseQuakeBar, KeyCategory::General)
            .bind("<enter>", Intent::SubmitQuakeBar, KeyCategory::Input)
            .bind("<pgup>", Intent::QuakeBarScrollUp, KeyCategory::Navigation)
            .bind("<pgdn>", Intent::QuakeBarScrollDown, KeyCategory::Navigation)
            .bind("<backspace>", Intent::DeleteGrapheme, KeyCategory::Input)
            .bind("<delete>", Intent::DeleteGraphemeForward, KeyCategory::Input)
            .bind("<left>", Intent::MoveCursorLeft, KeyCategory::Input)
            .bind("<right>", Intent::MoveCursorRight, KeyCategory::Input)
            .bind("<home>", Intent::MoveCursorToStart, KeyCategory::Input)
            .bind("<end>", Intent::MoveCursorToEnd, KeyCategory::Input)
            .bind("<c-c>", Intent::CtrlClear, KeyCategory::General)
            .catch_all(|key: KeyEvent| {
                if let Key::Char(c) = key.key {
                    Some(Intent::InsertChar { ch: c })
                } else {
                    None
                }
            });
    });

    // Global bindings — apply in every scope, with specific-scope-wins precedence.
    keymap.bind_global("<M-`>", Intent::OpenQuakeBar, KeyCategory::General);

    keymap.on_mouse(|mouse: event::MouseEvent, _scope: &Scope| {
        match mouse.kind {
            MouseEventKind::ScrollUp => Some(Intent::MouseScrollUp),
            MouseEventKind::ScrollDown => Some(Intent::MouseScrollDown),
            _ => None,
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use super::*;

    /// Regression test for ratatui-which-key v0.12.1: when a key is bound as a
    /// leaf in one scope (Normal) and used as a describe_group prefix in
    /// another scope (SidebarSessions), the leaf must survive the
    /// Leaf→Branch promotion. Before the fix, the library dropped the
    /// existing binding and the catch-all fired instead.
    #[test]
    fn p_prefix_group_in_sidebar_does_not_drop_normal_pin_binding() {
        use crate::app::WhichKeyInstance;
        use jinn_domain::{Key, Modifiers};

        // Given a fresh keymap with no custom bindings.
        let keymap = init();
        let mut wk = WhichKeyInstance::new(keymap, Scope::Normal);

        // When pressing 'p' alone.
        let intent = wk.handle_key(jinn_domain::KeyEvent {
            key: Key::Char('p'),
            modifiers: Modifiers::none(),
        });

        // Then it fires ChatEntryPinSelected (not a chord prefix).
        assert!(
            matches!(intent, Some(jinn_domain::Intent::ChatEntryPinSelected)),
            "'p' in Normal scope should fire ChatEntryPinSelected; got {intent:?}",
        );
    }

    #[test]
    fn backtick_in_input_scope_does_not_insert_literal_backtick() {
        // Given a keymap with the global <M-`> binding, queried in Input scope.
        use crate::app::WhichKeyInstance;
        use jinn_domain::{Key, KeyEvent, Modifiers};

        let keymap = init();
        let mut wk = WhichKeyInstance::new(keymap, Scope::Input);

        // When pressing <M-`> (Alt+backtick).
        let alt_backtick = KeyEvent {
            key: Key::Char('`'),
            modifiers: Modifiers {
                ctrl: false,
                alt: true,
                shift: false,
            },
        };
        let intent = wk.handle_key(alt_backtick);

        // Then it resolves to OpenQuakeBar, NOT a literal InsertChar('`').
        let intent = intent.expect(
            "<M-`> in Input scope must fire an intent; got None (global binding not found, catch-all regression)",
        );
        assert!(
            matches!(intent, Intent::OpenQuakeBar),
            "<M-`> must resolve to OpenQuakeBar, not InsertChar; got {intent:?}",
        );
    }

    #[test]
    fn quake_bar_scope_esc_fires_close_quake_bar() {
        // Given a keymap queried in QuakeBar scope.
        use crate::app::WhichKeyInstance;
        use jinn_domain::{Key, KeyEvent, Modifiers};

        let keymap = init();
        let mut wk = WhichKeyInstance::new(keymap, Scope::QuakeBar);

        // When pressing ESC.
        let esc = KeyEvent {
            key: Key::Esc,
            modifiers: Modifiers {
                ctrl: false,
                alt: false,
                shift: false,
            },
        };
        let intent = wk.handle_key(esc);

        // Then it resolves to CloseQuakeBar (which pops the quake bar scope).
        let intent = intent.expect("ESC in QuakeBar scope must fire an intent");
        assert!(
            matches!(intent, Intent::CloseQuakeBar),
            "ESC must resolve to CloseQuakeBar; got {intent:?}",
        );
    }

    #[test]
    fn quake_bar_scope_meta_backtick_fires_close_quake_bar() {
        // Given a keymap queried in QuakeBar scope.
        use crate::app::WhichKeyInstance;
        use jinn_domain::{Key, KeyEvent, Modifiers};

        let keymap = init();
        let mut wk = WhichKeyInstance::new(keymap, Scope::QuakeBar);

        // When pressing <M-`> (the scoped close binding, overriding the global opener).
        let meta_backtick = KeyEvent {
            key: Key::Char('`'),
            modifiers: Modifiers {
                ctrl: false,
                alt: true,
                shift: false,
            },
        };
        let intent = wk.handle_key(meta_backtick);

        // Then it resolves to CloseQuakeBar, making <M-`> a toggle (specific-scope-wins
        // over the global OpenQuakeBar).
        let intent = intent.expect("<M-`> in QuakeBar scope must fire an intent");
        assert!(
            matches!(intent, Intent::CloseQuakeBar),
            "<M-`> in QuakeBar scope must resolve to CloseQuakeBar (toggle); got {intent:?}",
        );
    }

    #[test]
    fn quake_bar_scope_printable_char_routes_to_insert_char() {
        // Given a keymap queried in QuakeBar scope.
        use crate::app::WhichKeyInstance;
        use jinn_domain::{Key, KeyEvent, Modifiers};

        let keymap = init();
        let mut wk = WhichKeyInstance::new(keymap, Scope::QuakeBar);

        // When pressing a plain printable char.
        let key_x = KeyEvent {
            key: Key::Char('x'),
            modifiers: Modifiers {
                ctrl: false,
                alt: false,
                shift: false,
            },
        };
        let intent = wk.handle_key(key_x);

        // Then it resolves to InsertChar('x') (full keystroke capture).
        let intent = intent.expect("printable char in QuakeBar scope must fire an intent");
        assert!(
            matches!(intent, Intent::InsertChar { ch: 'x' }),
            "printable char must route to InsertChar; got {intent:?}",
        );
    }

    #[test]
    fn quake_bar_scope_pgup_fires_scroll_up() {
        // Given a keymap queried in QuakeBar scope.
        use crate::app::WhichKeyInstance;
        use jinn_domain::{Key, KeyEvent, Modifiers};

        let keymap = init();
        let mut wk = WhichKeyInstance::new(keymap, Scope::QuakeBar);

        // When pressing PageUp.
        let pgup = KeyEvent {
            key: Key::PageUp,
            modifiers: Modifiers {
                ctrl: false,
                alt: false,
                shift: false,
            },
        };
        let intent = wk.handle_key(pgup);

        // Then it resolves to QuakeBarScrollUp (so the log actually scrolls).
        let intent = intent.expect("PageUp in QuakeBar scope must fire an intent");
        assert!(
            matches!(intent, Intent::QuakeBarScrollUp),
            "PageUp must resolve to QuakeBarScrollUp; got {intent:?}",
        );
    }

    #[test]
    fn task_list_scope_pgup_fires_preview_scroll_up() {
        // Given a keymap queried in SidebarTaskList scope.
        use crate::app::WhichKeyInstance;
        use jinn_domain::{Key, KeyEvent, Modifiers};

        let keymap = init();
        let mut wk = WhichKeyInstance::new(keymap, Scope::SidebarTaskList);

        // When pressing PageUp.
        let pgup = KeyEvent {
            key: Key::PageUp,
            modifiers: Modifiers {
                ctrl: false,
                alt: false,
                shift: false,
            },
        };
        let intent = wk.handle_key(pgup);

        // Then it resolves to TaskListPreviewScrollUp.
        let intent = intent.expect("PageUp in SidebarTaskList scope must fire an intent");
        assert!(
            matches!(intent, Intent::TaskListPreviewScrollUp),
            "PageUp must resolve to TaskListPreviewScrollUp; got {intent:?}",
        );
    }

    #[test]
    fn task_list_scope_pgdn_fires_preview_scroll_down() {
        // Given a keymap queried in SidebarTaskList scope.
        use crate::app::WhichKeyInstance;
        use jinn_domain::{Key, KeyEvent, Modifiers};

        let keymap = init();
        let mut wk = WhichKeyInstance::new(keymap, Scope::SidebarTaskList);

        // When pressing PageDown.
        let pgdn = KeyEvent {
            key: Key::PageDown,
            modifiers: Modifiers {
                ctrl: false,
                alt: false,
                shift: false,
            },
        };
        let intent = wk.handle_key(pgdn);

        // Then it resolves to TaskListPreviewScrollDown.
        let intent = intent.expect("PageDown in SidebarTaskList scope must fire an intent");
        assert!(
            matches!(intent, Intent::TaskListPreviewScrollDown),
            "PageDown must resolve to TaskListPreviewScrollDown; got {intent:?}",
        );
    }

    #[test]
    fn ctrl_d_in_project_picker_removes_highlighted() {
        use crate::app::WhichKeyInstance;
        use jinn_domain::{Key, Modifiers};

        // Given a fresh keymap.
        let keymap = init();
        let mut wk = WhichKeyInstance::new(keymap, Scope::PickerProject);

        // When pressing Ctrl+D.
        let intent = wk.handle_key(jinn_domain::KeyEvent {
            key: Key::Char('d'),
            modifiers: Modifiers::ctrl(),
        });

        // Then it fires ProjectRemoveHighlighted.
        assert!(
            matches!(intent, Some(jinn_domain::Intent::ProjectRemoveHighlighted)),
            "<c-d> in PickerProject should fire ProjectRemoveHighlighted; got {intent:?}",
        );
    }

    #[test]
    fn bare_d_in_project_picker_types_into_filter() {
        use crate::app::WhichKeyInstance;
        use jinn_domain::{Key, Modifiers};

        // Given a fresh keymap.
        let keymap = init();
        let mut wk = WhichKeyInstance::new(keymap, Scope::PickerProject);

        // When pressing bare 'd'.
        let intent = wk.handle_key(jinn_domain::KeyEvent {
            key: Key::Char('d'),
            modifiers: Modifiers::none(),
        });

        // Then it falls through to the catch-all and types into the filter.
        assert!(
            matches!(
                intent,
                Some(jinn_domain::Intent::PickerInsertChar { ch: 'd' })
            ),
            "bare 'd' in PickerProject should type into the filter; got {intent:?}",
        );
    }

    #[test]
    fn bare_a_in_project_picker_types_into_filter_not_add_cwd() {
        use crate::app::WhichKeyInstance;
        use jinn_domain::{Key, Modifiers};

        // Given a fresh keymap.
        let keymap = init();
        let mut wk = WhichKeyInstance::new(keymap, Scope::PickerProject);

        // When pressing bare 'a'.
        let intent = wk.handle_key(jinn_domain::KeyEvent {
            key: Key::Char('a'),
            modifiers: Modifiers::none(),
        });

        // Then it types into the filter (the unapproved 'a' add-cwd bind is gone).
        assert!(
            matches!(
                intent,
                Some(jinn_domain::Intent::PickerInsertChar { ch: 'a' })
            ),
            "bare 'a' in PickerProject should type into the filter, not add cwd; got {intent:?}",
        );
    }

    #[rstest::rstest]
    fn leader_sr_resolves_to_reasoning_effort_picker() {
        // Given the default keymap.
        use jinn_domain::{Key, KeyEvent, Modifiers};
        use ratatui_which_key::NodeResult;
        let keymap = init();
        let leader = KeyEvent {
            key: Key::Char(' '),
            modifiers: Modifiers::none(),
        };

        // When navigating the <leader>sr sequence.
        let path = [
            leader,
            KeyEvent {
                key: Key::Char('s'),
                modifiers: Modifiers::none(),
            },
            KeyEvent {
                key: Key::Char('r'),
                modifiers: Modifiers::none(),
            },
        ];
        let result = keymap.navigate(&path, &Scope::Normal).expect("path exists");

        // Then it resolves to OpenPicker{ReasoningEffort}.
        match result {
            NodeResult::Leaf { action } => assert!(
                matches!(
                    action,
                    Intent::OpenPicker {
                        kind: PickerKind::ReasoningEffort
                    }
                ),
                "<leader>sr must resolve to OpenPicker{{ReasoningEffort}}; got {action:?}",
            ),
            other => panic!("<leader>sr must be a leaf, got branch: {other:?}"),
        }
    }
    #[rstest::rstest]
    fn gdc_resolves_to_to_discord_thread() {
        // Given the default keymap.
        use jinn_domain::{Key, KeyEvent, Modifiers};
        use ratatui_which_key::NodeResult;
        let keymap = init();

        // When navigating the gdc sequence (g → d → c).
        let path = [
            KeyEvent {
                key: Key::Char('g'),
                modifiers: Modifiers::none(),
            },
            KeyEvent {
                key: Key::Char('d'),
                modifiers: Modifiers::none(),
            },
            KeyEvent {
                key: Key::Char('c'),
                modifiers: Modifiers::none(),
            },
        ];
        let result = keymap.navigate(&path, &Scope::Normal).expect("path exists");

        // Then it resolves to Intent::ToDiscordThread.
        match result {
            NodeResult::Leaf { action } => assert!(
                matches!(action, Intent::ToDiscordThread),
                "gdc must resolve to ToDiscordThread; got {action:?}",
            ),
            other => panic!("gdc must be a leaf, got branch: {other:?}"),
        }
    }

    #[rstest::rstest]
    fn reasoning_effort_picker_scope_binds_base_intents() {
        // Given the default keymap.
        use jinn_domain::{Key, KeyEvent, Modifiers};
        use ratatui_which_key::NodeResult;
        let keymap = init();
        let esc = KeyEvent {
            key: Key::Esc,
            modifiers: Modifiers::none(),
        };
        let enter = KeyEvent {
            key: Key::Enter,
            modifiers: Modifiers::none(),
        };

        // When navigating the two explicit base keys within the ReasoningEffort picker scope.
        let esc_res = keymap
            .navigate(&[esc], &Scope::PickerReasoningEffort)
            .expect("esc bound");
        let enter_res = keymap
            .navigate(&[enter], &Scope::PickerReasoningEffort)
            .expect("enter bound");

        // Then each resolves to a real picker base intent (the bug: scope had no bindings).
        let NodeResult::Leaf { action: esc_action } = esc_res else {
            panic!("esc must be a leaf");
        };
        assert!(
            matches!(esc_action, Intent::EnterNormalMode),
            "esc must resolve to EnterNormalMode, got {esc_action:?}"
        );

        let NodeResult::Leaf {
            action: enter_action,
        } = enter_res
        else {
            panic!("enter must be a leaf");
        };
        assert!(
            matches!(enter_action, Intent::PickerConfirm),
            "enter must resolve to PickerConfirm, got {enter_action:?}"
        );
    }

    #[rstest::rstest]
    fn leader_se_resolves_to_persona_picker() {
        // Given the default keymap.
        use jinn_domain::{Key, KeyEvent, Modifiers};
        use ratatui_which_key::NodeResult;
        let keymap = init();
        let path = [
            KeyEvent {
                key: Key::Char(' '),
                modifiers: Modifiers::none(),
            },
            KeyEvent {
                key: Key::Char('s'),
                modifiers: Modifiers::none(),
            },
            KeyEvent {
                key: Key::Char('e'),
                modifiers: Modifiers::none(),
            },
        ];

        // When navigating the <leader>se sequence.
        let result = keymap.navigate(&path, &Scope::Normal).expect("path exists");

        // Then it resolves to OpenPicker{Persona} (rebound from <leader>sp).
        match result {
            NodeResult::Leaf { action } => assert!(
                matches!(
                    action,
                    Intent::OpenPicker {
                        kind: PickerKind::Persona
                    }
                ),
                "<leader>se must resolve to OpenPicker{{Persona}}; got {action:?}",
            ),
            other => panic!("<leader>se must be a leaf, got branch: {other:?}"),
        }
    }

    #[rstest::rstest]
    fn bracket_c_chord_resolves_to_jump_compaction_intents() {
        // Given the default keymap.
        use jinn_domain::{Key, KeyEvent, Modifiers};
        use ratatui_which_key::NodeResult;
        let keymap = init();

        // When navigating ]c (next compaction) in Normal scope.
        let next_path = [
            KeyEvent {
                key: Key::Char(']'),
                modifiers: Modifiers::none(),
            },
            KeyEvent {
                key: Key::Char('c'),
                modifiers: Modifiers::none(),
            },
        ];
        let next_result = keymap
            .navigate(&next_path, &Scope::Normal)
            .expect("]c path exists");

        // Then it resolves to ChatEntryJumpNextCompaction.
        match next_result {
            NodeResult::Leaf { action } => assert!(
                matches!(action, Intent::ChatEntryJumpNextCompaction),
                "]c must resolve to ChatEntryJumpNextCompaction; got {action:?}",
            ),
            other => panic!("]c must be a leaf, got branch: {other:?}"),
        }

        // When navigating [c (previous compaction) in Normal scope.
        let prev_path = [
            KeyEvent {
                key: Key::Char('['),
                modifiers: Modifiers::none(),
            },
            KeyEvent {
                key: Key::Char('c'),
                modifiers: Modifiers::none(),
            },
        ];
        let prev_result = keymap
            .navigate(&prev_path, &Scope::Normal)
            .expect("[c path exists");

        // Then it resolves to ChatEntryJumpPrevCompaction.
        match prev_result {
            NodeResult::Leaf { action } => assert!(
                matches!(action, Intent::ChatEntryJumpPrevCompaction),
                "[c must resolve to ChatEntryJumpPrevCompaction; got {action:?}",
            ),
            other => panic!("[c must be a leaf, got branch: {other:?}"),
        }
    }

    #[test]
    fn bracket_c_chord_does_not_resolve_in_input_scope() {
        // Given the default keymap queried in Input scope.
        // Input scope has a catch-all that turns every Char into InsertChar,
        // so the `]c` / `[c` jump chords (bound only in Normal) must never fire here.
        use crate::app::WhichKeyInstance;
        use jinn_domain::{Key, KeyEvent, Modifiers};

        let keymap = init();
        let mut wk = WhichKeyInstance::new(keymap, Scope::Input);

        let bracket = KeyEvent {
            key: Key::Char(']'),
            modifiers: Modifiers::none(),
        };

        // When pressing `]` in Input scope.
        let intent = wk.handle_key(bracket);

        // Then it resolves to a literal InsertChar(']'), not the jump chord prefix.
        // The `]c` jump intents are therefore unreachable in Input scope.
        let intent = intent.expect("] in Input scope must fire an intent (catch-all)");
        assert!(
            matches!(intent, Intent::InsertChar { ch: ']' }),
            "] in Input scope must insert a literal ], not start the jump chord; got {intent:?}",
        );
    }

    #[rstest::rstest]
    fn bracket_p_chord_resolves_to_jump_pinned_intents() {
        // Given the default keymap.
        use jinn_domain::{Key, KeyEvent, Modifiers};
        use ratatui_which_key::NodeResult;
        let keymap = init();

        // When navigating ]p (next pinned) in Normal scope.
        let next_path = [
            KeyEvent {
                key: Key::Char(']'),
                modifiers: Modifiers::none(),
            },
            KeyEvent {
                key: Key::Char('p'),
                modifiers: Modifiers::none(),
            },
        ];
        let next_result = keymap
            .navigate(&next_path, &Scope::Normal)
            .expect("]p path exists");

        // Then it resolves to ChatEntryJumpNextPinned.
        match next_result {
            NodeResult::Leaf { action } => assert!(
                matches!(action, Intent::ChatEntryJumpNextPinned),
                "]p must resolve to ChatEntryJumpNextPinned; got {action:?}",
            ),
            other => panic!("]p must be a leaf, got branch: {other:?}"),
        }

        // When navigating [p (previous pinned) in Normal scope.
        let prev_path = [
            KeyEvent {
                key: Key::Char('['),
                modifiers: Modifiers::none(),
            },
            KeyEvent {
                key: Key::Char('p'),
                modifiers: Modifiers::none(),
            },
        ];
        let prev_result = keymap
            .navigate(&prev_path, &Scope::Normal)
            .expect("[p path exists");

        // Then it resolves to ChatEntryJumpPrevPinned.
        match prev_result {
            NodeResult::Leaf { action } => assert!(
                matches!(action, Intent::ChatEntryJumpPrevPinned),
                "[p must resolve to ChatEntryJumpPrevPinned; got {action:?}",
            ),
            other => panic!("[p must be a leaf, got branch: {other:?}"),
        }
    }
}

#[cfg(test)]
mod leak_check {
    use crate::keymap::init;
    use crate::scope::Scope;
    use ratatui_which_key::Keymap as WKKeymap;

    #[test]
    fn dashboard_scope_has_no_chathistory_or_sidebar_bindings() {
        let keymap: WKKeymap<
            jinn_domain::KeyEvent,
            Scope,
            jinn_domain::Intent,
            crate::keymap::KeyCategory,
        > = init();
        let groups = keymap.bindings_for_scope(Scope::Dashboard);
        let all_desc: Vec<&str> = groups
            .iter()
            .flat_map(|g| g.bindings.iter().map(|b| b.description.as_str()))
            .collect();
        assert!(
            !all_desc
                .iter()
                .any(|d| d.contains("next") || d.contains("previous")),
            "ChatHistory groups leaked into Dashboard: {all_desc:?}"
        );
    }
    #[test]
    fn normal_scope_still_shows_chathistory_and_sidebar_groups() {
        // Regression: the library fix must not remove ChatHistory groups from
        // Normal scope where they legitimately belong. The `p` key in Normal
        // scope is a leaf (ChatEntryPinSelected → "pin entry"), not the
        // sessions branch, so we only assert the bracket groups here.
        let keymap: WKKeymap<
            jinn_domain::KeyEvent,
            Scope,
            jinn_domain::Intent,
            crate::keymap::KeyCategory,
        > = init();
        let groups = keymap.bindings_for_scope(Scope::Normal);
        let all_desc: Vec<&str> = groups
            .iter()
            .flat_map(|g| g.bindings.iter().map(|b| b.description.as_str()))
            .collect();
        assert!(
            all_desc.iter().any(|d| d.contains("next")),
            "next group should appear in Normal scope; got {all_desc:?}"
        );
        assert!(
            all_desc.iter().any(|d| d.contains("previous")),
            "previous group should appear in Normal scope; got {all_desc:?}"
        );
    }
}
