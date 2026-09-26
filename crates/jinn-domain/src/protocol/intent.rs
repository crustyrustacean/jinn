//! The [`Intent`] enum - one variant per user-initiated action.
use std::sync::Arc;

use crate::protocol::PickerKind;
use jinn_core_types::SessionId;

/// The search root for the directory picker (shared vocabulary from
/// `jinn-slices`; the scope-focus cell carries it in `TuiSignals`).
pub use jinn_slices::cwd_root::CwdRoot;

/// A user-initiated action.
///
/// Every keymap binding and mouse event produces exactly one [`Intent`] variant.
/// The keymap decides the intent; the `IntentHandler` decides what to do with it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum KernelIntent {
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
    /// Toggle the input submission mode between Queue and Steer.
    ToggleInputMode,

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
    /// Paste text from the clipboard (bracketed paste).
    PasteText {
        /// The pasted text content.
        text: String,
    },

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
    /// Open the input in an external editor.
    EditInput,

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
    /// Universal ctrl-c clear/leave: clears the active text input; if the input
    /// is empty, leaves the active popup scope (equivalent to `<esc>` for popups).
    CtrlClear,
    /// Enter Insert (Input) mode - the chat input box is active.
    EnterInsertMode,
    /// Enter Normal mode - cancel streams, clear picker, return to neutral.
    EnterNormalMode,
    /// Toggle the which-key popup.
    ToggleWhichkey,
    /// Escape key in Normal mode: cancel selection.
    NormalEscape,
    /// No-op intent produced by unmapped keys in scopes with confirmation prompts.
    /// Dismisses any active confirmation prompt via the pre-match interceptors.
    NoOp,

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
    /// Run a spec-driven picker's declared bind action.
    ///
    /// One data-carried intent covers every picker's binds: `picker` is the
    /// spec's registry id, `action` the bind row's notation. Resolved
    /// through the picker's own bind table.
    PickerAction {
        /// The picker spec's registry id (e.g. `"skill"`).
        picker: String,
        /// The bind row's action (e.g. `"<tab>"`).
        action: String,
    },
    /// Move the picker selection up.
    PickerMoveUp,
    /// Move the picker selection down.
    PickerMoveDown,
    /// Page the picker selection up by half the visible window.
    PickerPageUp,
    /// Page the picker selection down by half the visible window.
    PickerPageDown,
    /// Move the picker filter cursor left.
    PickerMoveCursorLeft,
    /// Move the picker filter cursor right.
    PickerMoveCursorRight,
    /// Create a new session.
    SessionNew,
    /// Refresh the model list from all providers.
    RefreshModels,
    /// Rescan the prompt templates directory.
    RescanPromptTemplates,
    /// Open the session lifecycle picker from the sidebar sessions section.

    /// Select the next chat entry.
    ChatEntrySelectNext,
    /// Select the previous chat entry.
    ChatEntrySelectPrev,
    /// Jump the cursor to the next (newer) compaction summary entry.
    ChatEntryJumpNextCompaction,
    /// Jump the cursor to the previous (older) compaction summary entry.
    ChatEntryJumpPrevCompaction,
    /// Jump the cursor to the next (newer) user message.
    ChatEntryJumpNextUserEntry,
    /// Jump the cursor to the previous (older) user message.
    ChatEntryJumpPrevUserEntry,
    /// Jump the cursor to the next (newer) pinned entry.
    ChatEntryJumpNextPinned,
    /// Jump the cursor to the previous (older) pinned entry.
    ChatEntryJumpPrevPinned,
    /// Jump the cursor to the next (newer) Sources (annotation) entry.
    ChatEntryJumpNextSources,
    /// Jump the cursor to the previous (older) Sources (annotation) entry.
    ChatEntryJumpPrevSources,
    /// Pin the currently selected chat entry.
    ChatEntryPinSelected,
    /// Toggle expand/collapse of the selected tool entry (tool call, tool result, or annotation).
    ExpandToolEntry,
    /// Toggle visibility of the audit popup for the currently selected chat entry.
    ToggleAuditPopup,
    /// Toggle visibility of the ignored entry block at the cursor.
    ToggleIgnoredBlockVisibility,
    /// Fork the session at the currently selected chat entry.
    ForkFromEntry,
    /// Create a new empty session seeded with the selected entry's text.
    ///
    /// Unlike [`ForkFromEntry`], the new session carries no inherited history;
    /// only the selected entry is copied in (kind preserved) as the sole
    /// history entry. Restricted to User and Assistant entries.
    NewSessionFromEntry,
    /// Yank (copy) the currently selected chat entry to the system clipboard.
    YankSelectedEntry,
    /// Toggle the `ignored` flag on the currently selected chat entry.
    ChatEntryIgnoreSelected,
    /// Reset the currently selected chat entry's context override to `Default`.
    ChatEntryResetSelected,
    /// Isolate the selected chat entry in context: force-include it and
    /// force-exclude all other non-pinned entries.
    ChatEntryIsolateSelected,

    /// Run a lifecycle setup command to create a new session.
    SessionLifecycleSetup {
        /// The lifecycle name (e.g., "fossil branch").
        lifecycle_name: String,
        /// Resolved positional arguments.
        args: Vec<String>,
    },
    /// Close the active session, running teardown if applicable.
    SessionClose,

    /// Change the session's working directory via an external picker.
    ChangeCwd {
        /// Where to search from.
        root: CwdRoot,
    },

    /// A dynamically-registered slice's action.
    ///
    /// Dispatched exclusively through the feature route table
    /// ([`KeyRoutes`](jinn_slices::route::KeyRoutes)):
    /// a slice that never registered a row for this intent is inert by
    /// construction. Carries its identity as data, so slices never edit
    /// this enum.
    Dynamic(jinn_slices::DynamicIntent),

    /// Switch between Chat and the registered dynamic tabs.
    SwitchTab,
}

impl jinn_slices::BusMessage for KernelIntent {}

impl trouper::schema::Schema for KernelIntent {
    fn schema_def() -> trouper::schema::SchemaDef {
        trouper::schema::SchemaDef {
            name: "KernelIntent".to_owned(),
            kind: trouper::schema::SchemaKind::Command,
            fields: vec![],
            description: Some(
                "A user-initiated action produced by the keymap (dispatched dynamically between slices)."
                    .to_owned(),
            ),
        }
    }
}

impl trouper::envelope::PayloadValue for KernelIntent {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn field(&self, _name: &str) -> Option<String> {
        None
    }

    fn to_json_bytes(&self) -> Arc<[u8]> {
        trouper::envelope::payload_value_json_bytes(self)
    }

    fn clone_value(&self) -> Box<dyn trouper::envelope::PayloadValue> {
        Box::new(self.clone())
    }
}

impl std::fmt::Display for KernelIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KernelIntent::InsertChar { ch } => write!(f, "insert '{ch}'"),
            KernelIntent::DeleteGrapheme => write!(f, "delete"),
            KernelIntent::DeleteGraphemeForward => write!(f, "forward delete"),
            KernelIntent::SubmitMessage => write!(f, "submit message"),
            KernelIntent::ToggleInputMode => write!(f, "toggle input mode"),
            KernelIntent::MoveCursorLeft => write!(f, "cursor left"),
            KernelIntent::MoveCursorRight => write!(f, "cursor right"),
            KernelIntent::MoveCursorToStart => write!(f, "cursor home"),
            KernelIntent::MoveCursorToEnd => write!(f, "cursor end"),
            KernelIntent::MoveCursorWordLeft => write!(f, "cursor word left"),
            KernelIntent::MoveCursorWordRight => write!(f, "cursor word right"),
            KernelIntent::MoveCursorUp => write!(f, "cursor up"),
            KernelIntent::MoveCursorDown => write!(f, "cursor down"),
            KernelIntent::AutocompleteConfirm => write!(f, "autocomplete confirm"),
            KernelIntent::PasteText { text } => {
                let line_count = text.lines().count();
                write!(f, "paste ({line_count} lines)")
            }
            KernelIntent::ScrollUp => write!(f, "scroll up"),
            KernelIntent::ScrollDown => write!(f, "scroll down"),
            KernelIntent::MouseScrollUp => write!(f, "mouse scroll up"),
            KernelIntent::MouseScrollDown => write!(f, "mouse scroll down"),
            KernelIntent::ScrollToTop => write!(f, "scroll to top"),
            KernelIntent::ScrollToBottom => write!(f, "scroll to bottom"),
            KernelIntent::EditInput => write!(f, "edit in $EDITOR"),
            KernelIntent::Quit => write!(f, "quit"),
            KernelIntent::Interrupt { .. } => write!(f, "interrupt"),
            KernelIntent::CtrlClear => write!(f, "ctrl-c clear"),
            KernelIntent::EnterInsertMode => write!(f, "enter insert mode"),
            KernelIntent::EnterNormalMode => write!(f, "enter normal mode"),
            KernelIntent::ToggleWhichkey => write!(f, "toggle which-key"),
            KernelIntent::NormalEscape => write!(f, "escape"),
            KernelIntent::NoOp => write!(f, "no-op"),
            KernelIntent::OpenPicker { kind } => write!(f, "search {kind}"),
            KernelIntent::PickerInsertChar { ch } => write!(f, "picker insert '{ch}'"),
            KernelIntent::PickerBackspace => write!(f, "picker backspace"),
            KernelIntent::PickerConfirm => write!(f, "picker confirm"),
            KernelIntent::PickerAction { picker, action } => {
                write!(f, "picker action {action} ({picker})")
            }
            KernelIntent::PickerMoveUp => write!(f, "picker move up"),
            KernelIntent::PickerMoveDown => write!(f, "picker move down"),
            KernelIntent::PickerPageUp => write!(f, "picker page up"),
            KernelIntent::PickerPageDown => write!(f, "picker page down"),
            KernelIntent::PickerMoveCursorLeft => write!(f, "picker cursor left"),
            KernelIntent::PickerMoveCursorRight => write!(f, "picker cursor right"),
            KernelIntent::SessionNew => write!(f, "new session"),
            KernelIntent::RefreshModels => write!(f, "refresh models"),
            KernelIntent::RescanPromptTemplates => write!(f, "rescan prompt templates"),

            KernelIntent::ChatEntrySelectNext => write!(f, "select next entry"),
            KernelIntent::ChatEntrySelectPrev => write!(f, "select prev entry"),
            KernelIntent::ChatEntryJumpNextCompaction => write!(f, "next compaction"),
            KernelIntent::ChatEntryJumpPrevCompaction => write!(f, "previous compaction"),
            KernelIntent::ChatEntryJumpNextUserEntry => write!(f, "next user message"),
            KernelIntent::ChatEntryJumpPrevUserEntry => write!(f, "previous user message"),
            KernelIntent::ChatEntryJumpNextPinned => write!(f, "next pinned entry"),
            KernelIntent::ChatEntryJumpPrevPinned => write!(f, "previous pinned entry"),
            KernelIntent::ChatEntryJumpNextSources => write!(f, "next sources entry"),
            KernelIntent::ChatEntryJumpPrevSources => write!(f, "previous sources entry"),
            KernelIntent::ChatEntryPinSelected => write!(f, "pin entry"),
            KernelIntent::ExpandToolEntry => write!(f, "expand tool entry"),
            KernelIntent::ToggleAuditPopup => write!(f, "toggle audit popup"),
            KernelIntent::ToggleIgnoredBlockVisibility => {
                write!(f, "toggle ignored block visibility")
            }
            KernelIntent::ForkFromEntry => write!(f, "fork from entry"),
            KernelIntent::NewSessionFromEntry => write!(f, "new session from entry"),
            KernelIntent::YankSelectedEntry => write!(f, "yank entry"),
            KernelIntent::ChatEntryIgnoreSelected => write!(f, "toggle entry in/out of context"),
            KernelIntent::ChatEntryResetSelected => write!(f, "reset entry to default context"),
            KernelIntent::ChatEntryIsolateSelected => {
                write!(f, "isolate selected entry in context")
            }

            KernelIntent::SessionLifecycleSetup { lifecycle_name, .. } => {
                write!(f, "session lifecycle setup: {lifecycle_name}")
            }
            KernelIntent::SessionClose => write!(f, "session close"),

            KernelIntent::ChangeCwd { root } => write!(f, "change cwd from '{root}'"),

            KernelIntent::Dynamic(dynamic) => write!(f, "{dynamic}"),
            KernelIntent::SwitchTab => write!(f, "switch tab"),
        }
    }
}

/// What an intent handler returns after processing an intent.
///
/// A type alias for the slice-level [`RouteResult`]: the route
/// mechanics (and this result type) live in `jinn-slices` so slice
/// crates can produce outcomes without depending on the kernel. The
/// publish closures are identical — `RouteResult::new_message` and
/// `Bridge::publish_closure` spawn the same `bus.tell(Publish(..))` —
/// so behavior is unchanged; only the definition's home moved.
///
/// Carries typed message closures to be dispatched to the actor system
/// onto the message fabric, plus an optional scope transition. The
/// scope signal is applied by the handler (an exempt scope-stack
/// writer) *before* the messages publish, so a slice that opens itself
/// pushes its scope before any bus message a subscriber could observe.
pub use jinn_slices::RouteResult as IntentResult;

/// A scope-stack transition requested by a route action.
///
/// Slices declare their transitions as data; the composition-side
/// handler applies them. Ownership stays single-writer: only the
/// handler mutates the scope stack, and it does so only on these signals.
pub use jinn_slices::ScopeSignal;
