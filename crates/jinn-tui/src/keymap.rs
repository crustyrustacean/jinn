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
        // Input - external editor
        .bind("<c-e>", Intent::EditInput, KeyCategory::Input)
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
            .bind("<leader>sp", Intent::OpenPicker { kind: PickerKind::Persona }, KeyCategory::General)
            .bind("<leader>st", Intent::OpenPicker { kind: PickerKind::Tool }, KeyCategory::General)
            .bind("<leader>sk", Intent::OpenPicker { kind: PickerKind::Skill }, KeyCategory::General)
            .bind("<leader>sh", Intent::OpenPicker { kind: PickerKind::Theme }, KeyCategory::General)
            .bind("<leader>sw", Intent::OpenPicker { kind: PickerKind::Workflow }, KeyCategory::General)
            // Input - enter input mode
            .bind("i", Intent::EnterInsertMode, KeyCategory::Input)
            .bind("<c-j>", Intent::EnterInsertMode, KeyCategory::Input)
            // Navigation - scrolling and tab switching
            .bind("k", Intent::ChatEntrySelectPrev, KeyCategory::Navigation)
            .bind("j", Intent::ChatEntrySelectNext, KeyCategory::Navigation)

            .bind("<c-u>", Intent::ScrollUp, KeyCategory::Navigation)
            .bind("<c-d>", Intent::ScrollDown, KeyCategory::Navigation)
            // Input - external editor
            .bind("<c-e>", Intent::EditInput, KeyCategory::Input)
            // Change CWD - search from session CWD
            .bind("<M-c>", Intent::ChangeCwd { root: CwdRoot::Session }, KeyCategory::Navigation)
            // Change CWD - search from home directory
            .bind("<M-d>", Intent::ChangeCwd { root: CwdRoot::Home }, KeyCategory::Navigation)
            // g prefix - general commands and model management
            .describe_group_with_category("g", "general", KeyCategory::General)
            .describe_group_with_category("gm", "model", KeyCategory::Model)
            .describe_group_with_category("gc", "context", KeyCategory::Context)
            .bind("<leader>sl", Intent::OpenPicker { kind: PickerKind::SessionLifecycle }, KeyCategory::General)
            .bind("<leader>sc", Intent::OpenPicker { kind: PickerKind::CompactionModel }, KeyCategory::Model)
            .bind("gg", Intent::ScrollToTop, KeyCategory::Navigation)
            .bind("G", Intent::ScrollToBottom, KeyCategory::Navigation)
            .bind("gmr", Intent::RefreshModels, KeyCategory::Model)
            .bind("gcr", Intent::RescanPromptTemplates, KeyCategory::Context)
            .bind("<c-l>", Intent::SidebarFocus, KeyCategory::Navigation)
            .bind("<M-s>", Intent::SidebarFocusSessions, KeyCategory::Navigation)
            // Sidebar resize
            .bind("<c-w>", Intent::SidebarResizeEnter, KeyCategory::Navigation)
            // Minimap navigation
            // Pin selected entry
            .bind("p", Intent::ChatEntryPinSelected, KeyCategory::Context)
            // Ignore/un-ignore selected entry
            .bind("x", Intent::ChatEntryIgnoreSelected, KeyCategory::Context)
            // Expand/collapse tool entry
            .bind("e", Intent::ExpandToolEntry, KeyCategory::Navigation)
            // Toggle ignored block visibility
            .bind("h", Intent::ToggleIgnoredBlockVisibility, KeyCategory::Navigation)
            // Fork session from selected entry
            .bind("f", Intent::ForkFromEntry, KeyCategory::General)
            // Yank (copy) selected entry to clipboard
            .bind("y", Intent::YankSelectedEntry, KeyCategory::Navigation)
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
            .bind("c", Intent::SidebarPersonaEdit, KeyCategory::Context);
        })
        // Sidebar - Pins section
        .scope(Scope::SidebarPins, |b| {
            add_sidebar_base(b);
            b
            // Pin management actions
            .bind("u", Intent::PinsUnpin, KeyCategory::Context)
            .bind("t", Intent::PinsPinTop, KeyCategory::Context)
            .bind("b", Intent::PinsPinBottom, KeyCategory::Context)
            .bind("r", Intent::PinsPinRelative, KeyCategory::Context)
            .bind("m", Intent::PinsPinCycle, KeyCategory::Context);
        })
        // Sidebar - Sessions section
        .scope(Scope::SidebarSessions, |b| {
            add_sidebar_base(b);
            b
            // Session management actions
            .bind("x", Intent::SidebarSessionClose, KeyCategory::General)
            .bind("t", Intent::SidebarSessionTeardown, KeyCategory::General)
            .bind("<enter>", Intent::SidebarConfirm, KeyCategory::General)
            .bind("n", Intent::SessionNew, KeyCategory::General)
            .bind("N", Intent::SessionNewWithLifecycle, KeyCategory::General)
            .bind("r", Intent::SidebarRenameSession, KeyCategory::General)
            .bind("a", Intent::SidebarSessionArchive, KeyCategory::General)
            .bind("c", Intent::SidebarSessionContinue, KeyCategory::General)

            // i activates session and enters insert mode (same as enter)
            .bind("i", Intent::SidebarConfirm, KeyCategory::Input)
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
        })
        // Input scope: typing into the input buffer
        .scope(Scope::Input, |b| {
            b.bind("<enter>", Intent::SubmitMessage, KeyCategory::Input)
            .bind("<s-enter>", Intent::InsertChar { ch: '\n' }, KeyCategory::Input)
            .bind("<c-enter>", Intent::InsertChar { ch: '\n' }, KeyCategory::Input)
            .bind("<esc>", Intent::EnterNormalMode, KeyCategory::General)
            .bind("<c-k>", Intent::EnterNormalMode, KeyCategory::General)
            .bind("<c-c>", Intent::Interrupt { session_id: None }, KeyCategory::General)
            .bind("<c-e>", Intent::EditInput, KeyCategory::Input)
            .bind("<c-g>", Intent::ToggleOneShot { kind: jinn_domain::feat::workflow::attached_workflow::OneShotKind::Consensus }, KeyCategory::Input)
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
            .bind("<M-s>", Intent::SidebarFocusSessions, KeyCategory::Navigation)
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
        .scope(Scope::PickerTool, |b| {
            add_picker_base(b);
            b.bind("<Tab>", Intent::ToolToggleSelected, KeyCategory::General);
        })
        .scope(Scope::PickerSkill, |b| {
            add_picker_base(b);
            b.bind("<Tab>", Intent::SkillToggleSelected, KeyCategory::General)
             .bind("<pgup>", Intent::PreviewScrollUp, KeyCategory::Navigation)
             .bind("<pgdn>", Intent::PreviewScrollDown, KeyCategory::Navigation);
        })
        .scope(Scope::PickerWorkflow, |b| {
            add_picker_base(b);
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
        .bind("h", Intent::SidebarResizeExpand, KeyCategory::Navigation)
        .bind("l", Intent::SidebarResizeContract, KeyCategory::Navigation)
        .bind("<esc>", Intent::SidebarResizeLeave, KeyCategory::General)
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
        .catch_all(|key: KeyEvent| {
            if let Key::Char(c) = key.key {
                Some(Intent::RenameInsertChar { ch: c })
            } else {
                None
            }
        });
    });

    // RenameWorkflowInput scope - editing a workflow label.
    keymap.scope(Scope::RenameWorkflowInput, |b| {
        b
        .bind("<esc>", Intent::RenameWorkflowLeave, KeyCategory::General)
        .bind("<enter>", Intent::RenameWorkflowConfirm, KeyCategory::Input)
        .bind("<left>", Intent::RenameWorkflowCursorLeft, KeyCategory::Input)
        .bind("<right>", Intent::RenameWorkflowCursorRight, KeyCategory::Input)
        .bind("<backspace>", Intent::RenameWorkflowDeleteGrapheme, KeyCategory::Input)
        .bind("<delete>", Intent::RenameWorkflowDeleteForward, KeyCategory::Input)
        .bind("<c-j>", Intent::RenameWorkflowInsertChar { ch: '\n' }, KeyCategory::Input)
        .catch_all(|key: KeyEvent| {
            if let Key::Char(c) = key.key {
                Some(Intent::RenameWorkflowInsertChar { ch: c })
            } else {
                None
            }
        });
    });
    // Workflow scope - workflow tab browsing.
    keymap.scope(Scope::Workflow, |b| {
        b

        .bind("q", Intent::Quit, KeyCategory::General)
        .bind("<c-c>", Intent::Quit, KeyCategory::General)
        .bind("?", Intent::ToggleWhichkey, KeyCategory::General)
        // Spatial node navigation
        .bind("h", Intent::WorkflowNodeLeft, KeyCategory::Navigation)
        .bind("j", Intent::WorkflowNodeDown, KeyCategory::Navigation)
        .bind("k", Intent::WorkflowNodeUp, KeyCategory::Navigation)
        .bind("l", Intent::WorkflowNodeRight, KeyCategory::Navigation)
        // Viewport panning (HJKL)
        .bind("H", Intent::WorkflowPanLeft, KeyCategory::Navigation)
        .bind("J", Intent::WorkflowPanDown, KeyCategory::Navigation)
        .bind("K", Intent::WorkflowPanUp, KeyCategory::Navigation)
        .bind("L", Intent::WorkflowPanRight, KeyCategory::Navigation)
        // Inspector
        .bind("i", Intent::WorkflowInspectToggle, KeyCategory::Navigation)
        .bind("<down>", Intent::WorkflowInspectScrollDown, KeyCategory::Navigation)
        .bind("<up>", Intent::WorkflowInspectScrollUp, KeyCategory::Navigation)
        // Cancel
        .bind("<esc>", Intent::WorkflowEscape, KeyCategory::General)
        // Run / re-run workflow
        .bind("<enter>", Intent::WorkflowRun, KeyCategory::General)
        // Edit source node data
        .bind("e", Intent::WorkflowEditNode, KeyCategory::Input)
        // Re-run from node
        .bind("r", Intent::WorkflowRerunNode, KeyCategory::General)
        // Sidebar
        .bind("<c-l>", Intent::SidebarFocus, KeyCategory::Navigation)
        .bind("<M-s>", Intent::SidebarFocusSessions, KeyCategory::Navigation)
        .bind("<c-w>", Intent::SidebarResizeEnter, KeyCategory::Navigation);

    });

    // WorkflowInput scope - editing source node data.
    keymap.scope(Scope::WorkflowInput, |b| {
        b
        .bind("<esc>", Intent::WorkflowInputCancel, KeyCategory::General)
        .bind("<enter>", Intent::WorkflowInputSubmit, KeyCategory::Input)
        .bind("<s-enter>", Intent::WorkflowInputInsertChar { ch: '\n' }, KeyCategory::Input)
        .bind("<backspace>", Intent::WorkflowInputDeleteGrapheme, KeyCategory::Input)
        .bind("<delete>", Intent::WorkflowInputDeleteGraphemeForward, KeyCategory::Input)
        .bind("<left>", Intent::WorkflowInputCursorLeft, KeyCategory::Input)
        .bind("<right>", Intent::WorkflowInputCursorRight, KeyCategory::Input)
        .bind("<home>", Intent::WorkflowInputCursorToStart, KeyCategory::Input)
        .bind("<end>", Intent::WorkflowInputCursorToEnd, KeyCategory::Input)
        .bind("<c-left>", Intent::WorkflowInputCursorWordLeft, KeyCategory::Input)
        .bind("<c-right>", Intent::WorkflowInputCursorWordRight, KeyCategory::Input)
        .bind("<up>", Intent::WorkflowInputCursorUp, KeyCategory::Input)
        .bind("<down>", Intent::WorkflowInputCursorDown, KeyCategory::Input)
        .bind("<c-j>", Intent::WorkflowInputInsertChar { ch: '\n' }, KeyCategory::Input)
        .catch_all(|key: KeyEvent| {
            if let Key::Char(c) = key.key {
                Some(Intent::WorkflowInputInsertChar { ch: c })
            } else {
                None
            }
        });
    });

    keymap.on_mouse(|mouse: event::MouseEvent, _scope: &Scope| {
        match mouse.kind {
            MouseEventKind::ScrollUp => Some(Intent::MouseScrollUp),
            MouseEventKind::ScrollDown => Some(Intent::MouseScrollDown),
            _ => None,
        }
    })
}
