//! Keymap configuration and initialization.
//!
//! Defines the key categories and builds the keymap with all scope bindings.
//! Binds keys to [`Intent`] variants. Parameterized on
//! [`KeyEvent`] so the keymap works in both TUI and headless modes.

use crossterm::event::{self, MouseEventKind};
use derive_more::Display;
use jinn_domain::KernelIntent;
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
#[must_use]
#[rustfmt::skip]
#[expect(clippy::too_many_lines, reason = "declarative keymap table; splitting it would obscure the binding overview")]
pub fn init() -> Keymap<KeyEvent, Scope, KernelIntent, KeyCategory> {
    let mut keymap = Keymap::new();

    keymap
        // Normal scope: navigation and commands
        .scope(Scope::Normal, |b| {
            b
            // General - app control
            .bind("q", KernelIntent::Quit, KeyCategory::General)
            .bind("<c-c>", KernelIntent::Quit, KeyCategory::General)
            .bind("?", KernelIntent::ToggleWhichkey, KeyCategory::General)
            .describe_group_with_category("<leader>s", "search", KeyCategory::General)
            // The model browser is slice-owned, so this binds the
            // provider-selection slice's own open row.
            .bind(
                "<leader>sm",
                KernelIntent::Dynamic(jinn_slices::DynamicIntent::new(
                    jinn_provider_selection_msg::provider_picker_scope(),
                    "open-provider-picker",
                    "choose a model",
                )),
                KeyCategory::General,
            )
            // The session browser and the MCP inspector are slice-owned, so
            // these bind the owning slice's own open row rather than a kernel
            // picker intent.
            .bind(
                "<leader>se",
                KernelIntent::Dynamic(jinn_slices::DynamicIntent::new(
                    jinn_session_store_msg::session_picker_scope(),
                    "open-session-picker",
                    "browse sessions",
                )),
                KeyCategory::General,
            )
            // OpenRouter routing endpoint pin (Single + OpenRouter models only).
            // Input - enter input mode
            .bind("i", KernelIntent::EnterInsertMode, KeyCategory::Input)
            .bind("<c-j>", KernelIntent::EnterInsertMode, KeyCategory::Input)
            // Navigation - scrolling and tab switching
            .bind("k", KernelIntent::ChatEntrySelectPrev, KeyCategory::Navigation)
            .bind("j", KernelIntent::ChatEntrySelectNext, KeyCategory::Navigation)
            .bind("<Tab>", KernelIntent::SwitchTab, KeyCategory::Navigation)

            .bind("<c-u>", KernelIntent::ScrollUp, KeyCategory::Navigation)
            .bind("<c-d>", KernelIntent::ScrollDown, KeyCategory::Navigation)
            // Change CWD - search from session CWD
            .bind("<M-c>", KernelIntent::ChangeCwd { root: CwdRoot::Session }, KeyCategory::Navigation)
            // Change CWD - search from home directory
            .bind("<M-d>", KernelIntent::ChangeCwd { root: CwdRoot::Home }, KeyCategory::Navigation)
            // g prefix - general commands and model management
            .describe_group_with_category("g", "general", KeyCategory::General)
            .describe_group_with_category("gm", "model", KeyCategory::Model)
            .describe_group_with_category("gc", "context", KeyCategory::Context)
            .describe_group_with_category("<leader>c", "change", KeyCategory::General)
            .bind("gg", KernelIntent::ScrollToTop, KeyCategory::Navigation)
            .bind("G", KernelIntent::ScrollToBottom, KeyCategory::Navigation)
            .bind("gmr", KernelIntent::RefreshModels, KeyCategory::Model)
            .bind("gcr", KernelIntent::RescanPromptTemplates, KeyCategory::Context)
            // Isolate selected entry: force-include its tool loop, force-exclude the rest
            .bind("gci", KernelIntent::ChatEntryIsolateSelected, KeyCategory::Context)
            // Minimap navigation
            // Pin selected entry
            .bind("p", KernelIntent::ChatEntryPinSelected, KeyCategory::ChatHistory)
            .bind("x", KernelIntent::ChatEntryIgnoreSelected, KeyCategory::ChatHistory)
            // Reset selected entry to default context
            .bind("r", KernelIntent::ChatEntryResetSelected, KeyCategory::ChatHistory)
            // Expand/collapse tool entry
            .bind("e", KernelIntent::ExpandToolEntry, KeyCategory::ChatHistory)
            // Toggle audit popup for the selected entry
            .bind("a", KernelIntent::ToggleAuditPopup, KeyCategory::ChatHistory)
            // Toggle ignored block visibility
            .bind("h", KernelIntent::ToggleIgnoredBlockVisibility, KeyCategory::ChatHistory)
            // Fork session from selected entry
            .bind("f", KernelIntent::ForkFromEntry, KeyCategory::ChatHistory)
            // New session seeded with selected entry (no inherited history)
            .bind("F", KernelIntent::NewSessionFromEntry, KeyCategory::ChatHistory)
            // Yank (copy) selected entry to clipboard
            .bind("y", KernelIntent::YankSelectedEntry, KeyCategory::ChatHistory)
            // Jump to next/previous compaction summary entry
            .describe_group_with_category("]", "next", KeyCategory::ChatHistory)
            .describe_group_with_category("[", "previous", KeyCategory::ChatHistory)
            .bind("]c", KernelIntent::ChatEntryJumpNextCompaction, KeyCategory::ChatHistory)
            .bind("[c", KernelIntent::ChatEntryJumpPrevCompaction, KeyCategory::ChatHistory)
            .bind("]u", KernelIntent::ChatEntryJumpNextUserEntry, KeyCategory::ChatHistory)
            .bind("[u", KernelIntent::ChatEntryJumpPrevUserEntry, KeyCategory::ChatHistory)
            .bind("]p", KernelIntent::ChatEntryJumpNextPinned, KeyCategory::ChatHistory)
            .bind("[p", KernelIntent::ChatEntryJumpPrevPinned, KeyCategory::ChatHistory)
            // Jump to next/previous Sources (annotation) entry
            .bind("]s", KernelIntent::ChatEntryJumpNextSources, KeyCategory::ChatHistory)
            .bind("[s", KernelIntent::ChatEntryJumpPrevSources, KeyCategory::ChatHistory)
            // Session creation
            .bind("n", KernelIntent::SessionNew, KeyCategory::General)
            // Escape: cancel selection
            .bind("<esc>", KernelIntent::NormalEscape, KeyCategory::General)
            // Unmapped character keys produce NoOp to dismiss confirmation prompts
            .catch_all(|key: KeyEvent| {
                if let Key::Char(_) = key.key {
                    Some(KernelIntent::NoOp)
                } else {
                    None
                }
            });
        })
        // Sidebar - Persona section
        
        // Sidebar - Pins section
        
        // Sidebar - Sessions section
        
        // Sidebar - Task list section
        
        // Sidebar - MCP servers section (read-only in Part 1: nav only).
        
        // Input scope: typing into the input buffer
        .scope(Scope::Input, |b| {
            b.bind("<enter>", KernelIntent::SubmitMessage, KeyCategory::Input)
                .bind("<M-q>", KernelIntent::ToggleInputMode, KeyCategory::Input)
            .bind("<s-enter>", KernelIntent::InsertChar { ch: '\n' }, KeyCategory::Input)
            .bind("<c-enter>", KernelIntent::InsertChar { ch: '\n' }, KeyCategory::Input)
            .bind("<esc>", KernelIntent::EnterNormalMode, KeyCategory::General)
            .bind("<c-k>", KernelIntent::EnterNormalMode, KeyCategory::General)
            .bind("<c-c>", KernelIntent::CtrlClear, KeyCategory::General)
            .bind("<c-e>", KernelIntent::EditInput, KeyCategory::Input)
            // <c-g> consensus one-shot removed (workflow system deprecated)
            // Change CWD - search from session CWD
            .bind("<M-c>", KernelIntent::ChangeCwd { root: CwdRoot::Session }, KeyCategory::Navigation)
            // Change CWD - search from home directory
            .bind("<M-d>", KernelIntent::ChangeCwd { root: CwdRoot::Home }, KeyCategory::Navigation)
            .bind("<f1>", KernelIntent::ToggleWhichkey, KeyCategory::General)
            .bind("<backspace>", KernelIntent::DeleteGrapheme, KeyCategory::Input)
            .bind("<left>", KernelIntent::MoveCursorLeft, KeyCategory::Input)
            .bind("<right>", KernelIntent::MoveCursorRight, KeyCategory::Input)
            .bind("<home>", KernelIntent::MoveCursorToStart, KeyCategory::Input)
            .bind("<end>", KernelIntent::MoveCursorToEnd, KeyCategory::Input)
            .bind("<delete>", KernelIntent::DeleteGraphemeForward, KeyCategory::Input)
            .bind("<c-left>", KernelIntent::MoveCursorWordLeft, KeyCategory::Input)
            .bind("<c-right>", KernelIntent::MoveCursorWordRight, KeyCategory::Input)
            .bind("<up>", KernelIntent::MoveCursorUp, KeyCategory::Input)
            .bind("<down>", KernelIntent::MoveCursorDown, KeyCategory::Input)
            .bind("<tab>", KernelIntent::AutocompleteConfirm, KeyCategory::Input)
            .bind("<c-u>", KernelIntent::ScrollUp, KeyCategory::Navigation)
            .bind("<c-d>", KernelIntent::ScrollDown, KeyCategory::Navigation)

            .bind("<c-j>", KernelIntent::InsertChar { ch: '\n' }, KeyCategory::Input)
            .catch_all(|key: KeyEvent| {
                if let Key::Char(c) = key.key {
                    Some(KernelIntent::InsertChar { ch: c })
                } else {
                    None
                }
            });
        });

    // No global bindings by design: globals survive every scope's catch-all
    // and would pierce slice capture-mode hooks (stranding the control flag
    // on User) and overlay views (popping overlays mid-composition). The
    // would-be globals are per-scope slice rows — see `keymap_gen`.
    keymap.on_mouse(|mouse: event::MouseEvent, _scope: &Scope| {
        match mouse.kind {
            MouseEventKind::ScrollUp => Some(KernelIntent::MouseScrollUp),
            MouseEventKind::ScrollDown => Some(KernelIntent::MouseScrollDown),
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
    /// another scope (the sidebar sessions scope), the leaf must survive the
    /// Leaf→Branch promotion. Before the fix, the library dropped the
    /// existing binding and the catch-all fired instead.
    #[rstest::rstest]
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
            matches!(
                intent,
                Some(jinn_domain::KernelIntent::ChatEntryPinSelected)
            ),
            "'p' in Normal scope should fire ChatEntryPinSelected; got {intent:?}",
        );
    }

    #[rstest::rstest]
    fn the_kernel_defines_no_static_scope_for_the_reasoning_picker() {
        // The reasoning picker is slice-owned: its own scope is a
        // `Dynamic` scope its keys are attached to, not a static one the
        // kernel keymap enumerates. This guard pins the flip side of that
        // migration — the kernel must not have kept a static scope for it.
        assert!(
            "Picker(reasoning-effort)".parse::<Scope>().is_err(),
            "the kernel must not define a static scope for the slice-owned \
             reasoning picker"
        );
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
                matches!(action, KernelIntent::ChatEntryJumpNextCompaction),
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
                matches!(action, KernelIntent::ChatEntryJumpPrevCompaction),
                "[c must resolve to ChatEntryJumpPrevCompaction; got {action:?}",
            ),
            other => panic!("[c must be a leaf, got branch: {other:?}"),
        }
    }

    #[rstest::rstest]
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
            matches!(intent, KernelIntent::InsertChar { ch: ']' }),
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
                matches!(action, KernelIntent::ChatEntryJumpNextPinned),
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
                matches!(action, KernelIntent::ChatEntryJumpPrevPinned),
                "[p must resolve to ChatEntryJumpPrevPinned; got {action:?}",
            ),
            other => panic!("[p must be a leaf, got branch: {other:?}"),
        }
    }

    #[rstest::rstest]
    fn bracket_s_chord_resolves_to_jump_sources_intents() {
        // Given the default keymap.
        use jinn_domain::{Key, KeyEvent, Modifiers};
        use ratatui_which_key::NodeResult;
        let keymap = init();

        // When navigating ]s (next sources) in Normal scope.
        let next_path = [
            KeyEvent {
                key: Key::Char(']'),
                modifiers: Modifiers::none(),
            },
            KeyEvent {
                key: Key::Char('s'),
                modifiers: Modifiers::none(),
            },
        ];
        let next_result = keymap
            .navigate(&next_path, &Scope::Normal)
            .expect("]s path exists");

        // Then it resolves to ChatEntryJumpNextSources.
        match next_result {
            NodeResult::Leaf { action } => assert!(
                matches!(action, KernelIntent::ChatEntryJumpNextSources),
                "]s must resolve to ChatEntryJumpNextSources; got {action:?}",
            ),
            other => panic!("]s must be a leaf, got branch: {other:?}"),
        }

        // When navigating [s (previous sources) in Normal scope.
        let prev_path = [
            KeyEvent {
                key: Key::Char('['),
                modifiers: Modifiers::none(),
            },
            KeyEvent {
                key: Key::Char('s'),
                modifiers: Modifiers::none(),
            },
        ];
        let prev_result = keymap
            .navigate(&prev_path, &Scope::Normal)
            .expect("[s path exists");

        // Then it resolves to ChatEntryJumpPrevSources.
        match prev_result {
            NodeResult::Leaf { action } => assert!(
                matches!(action, KernelIntent::ChatEntryJumpPrevSources),
                "[s must resolve to ChatEntryJumpPrevSources; got {action:?}",
            ),
            other => panic!("[s must be a leaf, got branch: {other:?}"),
        }
    }
}

#[cfg(test)]
mod leak_check {
    #![allow(clippy::expect_used, clippy::panic, reason = "test code")]
    use crate::keymap::init;
    use crate::scope::Scope;
    use ratatui_which_key::Keymap as WKKeymap;

    #[rstest::rstest]
    #[test]
    fn normal_scope_still_shows_chathistory_and_sidebar_groups() {
        // Regression: the library fix must not remove ChatHistory groups from
        // Normal scope where they legitimately belong. The `p` key in Normal
        // scope is a leaf (ChatEntryPinSelected → "pin entry"), not the
        // sessions branch, so we only assert the bracket groups here.
        let keymap: WKKeymap<
            jinn_domain::KeyEvent,
            Scope,
            jinn_domain::KernelIntent,
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

    /// Normal-mode <enter> opens the selected task call's subagent session.
    /// Also guards against accidental rebinding: nothing else may claim
    /// <enter> in the Normal scope.
    #[rstest::rstest]
    #[test]
    fn enter_in_normal_scope_fires_load_subagent_session() {
        use crate::app::WhichKeyInstance;
        use jinn_domain::{Key, Modifiers};

        // Given the default keymap with the sidebar's route rows bound.
        let mut keymap = init();
        let routes = jinn_slices::route::KeyRoutes::new();
        jinn_sidebar::key_routes::attach_sidebar_rows(&routes);
        crate::keymap_gen::bind_route_rows(&routes, &mut keymap);
        let mut wk = WhichKeyInstance::new(keymap, Scope::Normal);

        // When pressing <enter>.
        let enter = jinn_domain::KeyEvent {
            key: Key::Enter,
            modifiers: Modifiers::none(),
        };
        let intent = wk.handle_key(enter);

        // Then it resolves to the sidebar's load-subagent dynamic action.
        assert!(
            matches!(
            intent,
            Some(jinn_domain::KernelIntent::Dynamic(ref dynamic))
                if dynamic.action == "load-subagent"
            ),
            "enter must open the subagent session for the selected task; got {intent:?}",
        );
    }
}
