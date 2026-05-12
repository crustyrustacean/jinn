//! The [`Intent`] enum — one variant per user-initiated action.

use crate::protocol::{Command, PickerKind, SessionId, TabDirection};

/// A user-initiated action.
///
/// Every keymap binding and mouse event produces exactly one [`Intent`] variant.
/// The keymap decides the intent; the `IntentHandler` decides what to do with it.
#[derive(Debug, Clone)]
pub enum Intent {
    // --- Chat Input ---
    /// Insert a character at the cursor position.
    InsertChar {
        /// The character to insert.
        ch: char,
    },
    /// Delete the grapheme before the cursor.
    DeleteGrapheme,
    /// Delete the grapheme after the cursor (forward delete).
    DeleteGraphemeForward,
    /// Submit the current input as a user message.
    SubmitMessage,
    /// Move the cursor one grapheme left.
    MoveCursorLeft,
    /// Move the cursor one grapheme right.
    MoveCursorRight,
    /// Move the cursor to the beginning of the input.
    MoveCursorToStart,
    /// Move the cursor to the end of the input.
    MoveCursorToEnd,
    /// Move the cursor one word left.
    MoveCursorWordLeft,
    /// Move the cursor one word right.
    MoveCursorWordRight,
    /// Move the cursor up one visual line.
    MoveCursorUp,
    /// Move the cursor down one visual line.
    MoveCursorDown,
    /// Confirm the autocomplete selection (Tab in Input scope).
    AutocompleteConfirm,

    // --- Navigation ---
    /// Scroll the chat log up.
    ScrollUp,
    /// Scroll the chat log down.
    ScrollDown,
    /// Mouse scroll up.
    MouseScrollUp,
    /// Mouse scroll down.
    MouseScrollDown,
    /// Scroll to the very top.
    ScrollToTop,
    /// Scroll to the very bottom.
    ScrollToBottom,
    /// Switch to the next/previous tab.
    SwitchTab {
        /// Which direction to cycle.
        direction: TabDirection,
    },
    /// Open the input in an external editor.
    EditInput,

    // --- Mode & App ---
    /// Quit the application.
    Quit,
    /// Context-sensitive interrupt: clear input or cancel stream.
    ///
    /// When `session_id` is `None`, applies to the active session (smart behavior).
    /// When `session_id` is `Some(id)`, targets a specific session for cancel only.
    Interrupt {
        /// The session to target, or `None` for the active session.
        session_id: Option<SessionId>,
    },
    /// Enter Insert (Input) mode — the chat input box is active.
    EnterInsertMode,
    /// Enter Normal mode — cancel streams, clear picker, return to neutral.
    EnterNormalMode,
    /// Toggle the which-key popup.
    ToggleWhichkey,
    /// Escape key in Normal mode: cancel selection.
    NormalEscape,

    // --- Picker ---
    /// Open a picker of the specified kind.
    OpenPicker {
        /// Which picker to open.
        kind: PickerKind,
    },
    /// Insert a character into the picker filter.
    PickerInsertChar {
        /// The character to insert.
        ch: char,
    },
    /// Delete the last character from the picker filter.
    PickerBackspace,
    /// Confirm the current picker selection.
    PickerConfirm,
    /// Move the picker selection up.
    PickerMoveUp,
    /// Move the picker selection down.
    PickerMoveDown,
    /// Move the picker filter cursor left.
    PickerMoveCursorLeft,
    /// Move the picker filter cursor right.
    PickerMoveCursorRight,
    /// Toggle the keymap picker scope filter.
    ToggleKeymapScopeFilter,
    /// Create a new session.
    SessionNew,
    /// Refresh the model list from all providers.
    RefreshModels,
    /// Rescan the prompt templates directory.
    RescanPromptTemplates,

    // --- Dashboard ---
    /// Move the dashboard selection down.
    DashboardSelectDown,
    /// Move the dashboard selection up.
    DashboardSelectUp,
    /// Move the dashboard selection to the first entry.
    DashboardSelectFirst,
    /// Move the dashboard selection to the last entry.
    DashboardSelectLast,

    // --- Pinned Panel ---
    /// Toggle the pinned context panel visibility.
    PinnedPanelToggle,
    /// Open the pinned context panel.
    PinnedPanelOpen,
    /// Close the pinned context panel.
    PinnedPanelClose,
    /// Move the pinned panel selection down.
    PinnedPanelSelectDown,
    /// Move the pinned panel selection up.
    PinnedPanelSelectUp,
    /// Unpin the selected pinned entry.
    PinnedPanelUnpin,
    /// Set the selected pinned entry's position to TOP.
    PinnedPanelPinTop,
    /// Set the selected pinned entry's position to BOTTOM.
    PinnedPanelPinBottom,
    /// Set the selected pinned entry's position to RELATIVE.
    PinnedPanelPinRelative,
    /// Cycle the selected pinned entry's position.
    PinnedPanelPinCycle,

    // --- Chat Entry Selection ---
    /// Select the next chat entry.
    ChatEntrySelectNext,
    /// Select the previous chat entry.
    ChatEntrySelectPrev,
    /// Pin the currently selected chat entry.
    ChatEntryPinSelected,
}

impl std::fmt::Display for Intent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Intent::InsertChar { ch } => write!(f, "insert '{ch}'"),
            Intent::DeleteGrapheme => write!(f, "delete"),
            Intent::DeleteGraphemeForward => write!(f, "forward delete"),
            Intent::SubmitMessage => write!(f, "submit message"),
            Intent::MoveCursorLeft => write!(f, "cursor left"),
            Intent::MoveCursorRight => write!(f, "cursor right"),
            Intent::MoveCursorToStart => write!(f, "cursor home"),
            Intent::MoveCursorToEnd => write!(f, "cursor end"),
            Intent::MoveCursorWordLeft => write!(f, "cursor word left"),
            Intent::MoveCursorWordRight => write!(f, "cursor word right"),
            Intent::MoveCursorUp => write!(f, "cursor up"),
            Intent::MoveCursorDown => write!(f, "cursor down"),
            Intent::AutocompleteConfirm => write!(f, "autocomplete confirm"),
            Intent::ScrollUp => write!(f, "scroll up"),
            Intent::ScrollDown => write!(f, "scroll down"),
            Intent::MouseScrollUp => write!(f, "mouse scroll up"),
            Intent::MouseScrollDown => write!(f, "mouse scroll down"),
            Intent::ScrollToTop => write!(f, "scroll to top"),
            Intent::ScrollToBottom => write!(f, "scroll to bottom"),
            Intent::SwitchTab { direction } => write!(f, "switch tab {direction}"),
            Intent::EditInput => write!(f, "edit in $EDITOR"),
            Intent::Quit => write!(f, "quit"),
            Intent::Interrupt { .. } => write!(f, "interrupt"),
            Intent::EnterInsertMode => write!(f, "enter insert mode"),
            Intent::EnterNormalMode => write!(f, "enter normal mode"),
            Intent::ToggleWhichkey => write!(f, "toggle which-key"),
            Intent::NormalEscape => write!(f, "escape"),
            Intent::OpenPicker { kind } => write!(f, "search {kind}"),
            Intent::PickerInsertChar { ch } => write!(f, "picker insert '{ch}'"),
            Intent::PickerBackspace => write!(f, "picker backspace"),
            Intent::PickerConfirm => write!(f, "picker confirm"),
            Intent::PickerMoveUp => write!(f, "picker move up"),
            Intent::PickerMoveDown => write!(f, "picker move down"),
            Intent::PickerMoveCursorLeft => write!(f, "picker cursor left"),
            Intent::PickerMoveCursorRight => write!(f, "picker cursor right"),
            Intent::ToggleKeymapScopeFilter => write!(f, "toggle keymap scope filter"),
            Intent::SessionNew => write!(f, "session new"),
            Intent::RefreshModels => write!(f, "refresh models"),
            Intent::RescanPromptTemplates => write!(f, "rescan prompt templates"),
            Intent::DashboardSelectDown => write!(f, "dashboard select down"),
            Intent::DashboardSelectUp => write!(f, "dashboard select up"),
            Intent::DashboardSelectFirst => write!(f, "dashboard select first"),
            Intent::DashboardSelectLast => write!(f, "dashboard select last"),
            Intent::PinnedPanelToggle => write!(f, "toggle pinned panel"),
            Intent::PinnedPanelOpen => write!(f, "open pinned panel"),
            Intent::PinnedPanelClose => write!(f, "close pinned panel"),
            Intent::PinnedPanelSelectDown => write!(f, "pinned panel select down"),
            Intent::PinnedPanelSelectUp => write!(f, "pinned panel select up"),
            Intent::PinnedPanelUnpin => write!(f, "pinned panel unpin"),
            Intent::PinnedPanelPinTop => write!(f, "pinned panel pin top"),
            Intent::PinnedPanelPinBottom => write!(f, "pinned panel pin bottom"),
            Intent::PinnedPanelPinRelative => write!(f, "pinned panel pin relative"),
            Intent::PinnedPanelPinCycle => write!(f, "pinned panel pin cycle"),
            Intent::ChatEntrySelectNext => write!(f, "select next entry"),
            Intent::ChatEntrySelectPrev => write!(f, "select prev entry"),
            Intent::ChatEntryPinSelected => write!(f, "pin selected entry"),
        }
    }
}

/// What an intent handler returns after processing an intent.
///
/// Carries commands to be dispatched to the actor system.
#[derive(Debug)]
pub struct IntentResult {
    /// Commands to send to the actor system.
    pub commands: Vec<Command>,
}

impl IntentResult {
    /// An empty result with no commands.
    #[must_use]
    pub fn empty() -> Self {
        Self { commands: vec![] }
    }

    /// A result with commands.
    #[must_use]
    pub fn with_commands(commands: Vec<Command>) -> Self {
        Self { commands }
    }
}
