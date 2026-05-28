//! The [`Intent`] enum — one variant per user-initiated action.
use crate::protocol::{Command, PickerKind, SessionId};

/// The search root for the directory picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CwdRoot {
    /// Search from the active session's current CWD.
    Session,
    /// Search from the user's home directory.
    Home,
}

impl std::fmt::Display for CwdRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CwdRoot::Session => write!(f, "session"),
            CwdRoot::Home => write!(f, "home"),
        }
    }
}

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
    /// Paste text from the clipboard (bracketed paste).
    PasteText {
        /// The pasted text content.
        text: String,
    },

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
    /// Toggle the selected tool's enabled/disabled state in the tool picker.
    ToolToggleSelected,
    /// Toggle the selected skill's enabled/disabled state in the skill picker.
    SkillToggleSelected,
    /// Create a new session.
    SessionNew,
    /// Refresh the model list from all providers.
    RefreshModels,
    /// Rescan the prompt templates directory.
    RescanPromptTemplates,

    // --- Sidebar ---
    /// Enter the sidebar scope.
    SidebarFocus,
    /// Jump directly to the Sessions sidebar section from any scope.
    SidebarFocusSessions,
    /// Leave the sidebar, returning to origin scope.
    SidebarLeave,
    /// Move selection down in the sidebar.
    SidebarMoveDown,
    /// Move selection up in the sidebar.
    SidebarMoveUp,
    /// Jump to the next sidebar section.
    SidebarSectionNext,
    /// Jump to the previous sidebar section.
    SidebarSectionPrev,
    /// Activate the selected session (switch to it).
    SidebarConfirm,
    /// Unpin the selected pinned entry.
    PinsUnpin,
    /// Set the selected pinned entry's position to TOP.
    PinsPinTop,
    /// Set the selected pinned entry's position to BOTTOM.
    PinsPinBottom,
    /// Set the selected pinned entry's position to RELATIVE.
    PinsPinRelative,
    /// Cycle the selected pinned entry's pin position.
    PinsPinCycle,
    /// Close the selected open session from the sidebar.
    SidebarSessionClose,
    /// Re-run teardown for the selected session without closing it.
    SidebarSessionTeardown,
    /// Archive the selected session without running teardown.
    SidebarSessionArchive,
    /// Open the persona picker from the sidebar.
    SidebarPersonaEdit,
    /// Open the session lifecycle picker from the sidebar sessions section.
    SessionNewWithLifecycle,
    /// Queue a "Continue" user message to the session under the sidebar cursor.
    SidebarSessionContinue,

    /// Toggle the attached/detached state of the judge session under the sidebar cursor.
    ToggleJudgeAttached,
    /// Toggle the per-session auto-reset override for the judge session under the sidebar cursor.
    ToggleJudgeAutoReset,

    /// Reset the judge session under the sidebar cursor — truncate history to pinned entries only.
    ResetJudge,

    // --- Chat Entry Selection ---
    /// Select the next chat entry.
    ChatEntrySelectNext,
    /// Select the previous chat entry.
    ChatEntrySelectPrev,
    /// Pin the currently selected chat entry.
    ChatEntryPinSelected,
    /// Toggle expand/collapse of the selected tool entry (tool call or tool result).
    ExpandToolEntry,
    /// Toggle visibility of the ignored entry block at the cursor.
    ToggleIgnoredBlockVisibility,
    /// Fork the session at the currently selected chat entry.
    ForkFromEntry,
    /// Yank (copy) the currently selected chat entry to the system clipboard.
    YankSelectedEntry,
    /// Toggle the `ignored` flag on the currently selected chat entry.
    ChatEntryIgnoreSelected,

    // --- Session Lifecycle ---
    /// Run a lifecycle setup command to create a new session.
    SessionLifecycleSetup {
        /// The lifecycle name (e.g., "fossil branch").
        lifecycle_name: String,
        /// Resolved positional arguments.
        args: Vec<String>,
    },
    /// Close the active session, running teardown if applicable.
    SessionClose,
    /// Confirm the arg input and trigger lifecycle setup.
    ArgInputConfirm,

    // --- Sidebar Resize ---
    /// Enter sidebar resize mode.
    SidebarResizeEnter,
    /// Expand the sidebar (move border left).
    SidebarResizeExpand,
    /// Contract the sidebar (move border right).
    SidebarResizeContract,
    /// Exit sidebar resize mode, returning to Normal scope.
    SidebarResizeLeave,

    // --- Rename Session Input ---
    /// Open the rename session input popup.
    SidebarRenameSession,
    /// Confirm the rename session input and apply.
    RenameSessionConfirm,
    /// Cancel the rename session input popup.
    RenameSessionLeave,
    /// Insert a character into the rename session input.
    RenameInsertChar {
        /// The character to insert.
        ch: char,
    },
    /// Move cursor left in the rename session input.
    RenameCursorLeft,
    /// Move cursor right in the rename session input.
    RenameCursorRight,
    /// Delete the grapheme before the cursor in rename input.
    RenameDeleteGrapheme,
    /// Delete the grapheme after the cursor in rename input.
    RenameDeleteForward,
    /// Switch to the next tab (Chat → Workflow → Chat).
    SwitchTab,

    // --- Workflow Navigation ---
    /// Select the nearest node to the left (spatial).
    WorkflowNodeLeft,
    /// Select the nearest node downward (spatial).
    WorkflowNodeDown,
    /// Select the nearest node upward (spatial).
    WorkflowNodeUp,
    /// Select the nearest node to the right (spatial).
    WorkflowNodeRight,
    /// Toggle the sticky node inspector popup.
    WorkflowInspectToggle,
    /// Scroll the inspector popup up one line.
    WorkflowInspectScrollUp,
    /// Scroll the inspector popup down one line.
    WorkflowInspectScrollDown,
    /// ESC in workflow scope: first press shows cancel prompt, second confirms cancel.
    WorkflowEscape,
    /// Re-run the workflow from the currently selected node.
    WorkflowRerunNode,
    /// Run the loaded workflow (or re-run a completed/failed workflow).
    WorkflowRun,
    /// Pan the workflow viewport left.
    WorkflowPanLeft,
    /// Pan the workflow viewport down.
    WorkflowPanDown,
    /// Pan the workflow viewport up.
    WorkflowPanUp,
    /// Pan the workflow viewport right.
    WorkflowPanRight,

    // --- Workflow Input Editing ---
    /// Enter editing mode on the selected workflow source node.
    WorkflowEditNode,
    /// Submit the workflow input buffer (write to node output).
    WorkflowInputSubmit,
    /// Cancel workflow input editing (discard changes).
    WorkflowInputCancel,
    /// Insert a character into the workflow input buffer.
    WorkflowInputInsertChar {
        /// The character to insert.
        ch: char,
    },
    /// Delete grapheme before cursor in workflow input buffer.
    WorkflowInputDeleteGrapheme,
    /// Delete grapheme after cursor (forward delete) in workflow input buffer.
    WorkflowInputDeleteGraphemeForward,
    /// Paste text into the workflow input buffer.
    WorkflowInputPasteText {
        /// The pasted text content.
        text: String,
    },
    /// Move cursor left in workflow input buffer.
    WorkflowInputCursorLeft,
    /// Move cursor right in workflow input buffer.
    WorkflowInputCursorRight,
    /// Move cursor to start of workflow input buffer.
    WorkflowInputCursorToStart,
    /// Move cursor to end of workflow input buffer.
    WorkflowInputCursorToEnd,
    /// Move cursor one word left in workflow input buffer.
    WorkflowInputCursorWordLeft,
    /// Move cursor one word right in workflow input buffer.
    WorkflowInputCursorWordRight,
    /// Move cursor up one visual line in workflow input buffer.
    WorkflowInputCursorUp,
    /// Move cursor down one visual line in workflow input buffer.
    WorkflowInputCursorDown,

    // --- CWD Picker ---
    /// Change the session's working directory via an external picker.
    ChangeCwd {
        /// Where to search from.
        root: CwdRoot,
    },
}

impl std::fmt::Display for Intent {
    #[allow(clippy::too_many_lines)]
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
            Intent::PasteText { text } => {
                let line_count = text.lines().count();
                write!(f, "paste ({line_count} lines)")
            }
            Intent::ScrollUp => write!(f, "scroll up"),
            Intent::ScrollDown => write!(f, "scroll down"),
            Intent::MouseScrollUp => write!(f, "mouse scroll up"),
            Intent::MouseScrollDown => write!(f, "mouse scroll down"),
            Intent::ScrollToTop => write!(f, "scroll to top"),
            Intent::ScrollToBottom => write!(f, "scroll to bottom"),
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
            Intent::ToolToggleSelected => write!(f, "toggle tool"),
            Intent::SkillToggleSelected => write!(f, "toggle skill"),
            Intent::SessionNew => write!(f, "session new"),
            Intent::RefreshModels => write!(f, "refresh models"),
            Intent::RescanPromptTemplates => write!(f, "rescan prompt templates"),
            Intent::SidebarFocus => write!(f, "sidebar focus"),
            Intent::SidebarFocusSessions => write!(f, "sidebar focus sessions"),
            Intent::SidebarLeave => write!(f, "sidebar leave"),
            Intent::SidebarMoveDown => write!(f, "sidebar move down"),
            Intent::SidebarMoveUp => write!(f, "sidebar move up"),
            Intent::SidebarSectionNext => write!(f, "sidebar section next"),
            Intent::SidebarSectionPrev => write!(f, "sidebar section prev"),
            Intent::SidebarConfirm => write!(f, "sidebar confirm"),
            Intent::PinsUnpin => write!(f, "pins unpin"),
            Intent::PinsPinTop => write!(f, "pins pin top"),
            Intent::PinsPinBottom => write!(f, "pins pin bottom"),
            Intent::PinsPinRelative => write!(f, "pins pin relative"),
            Intent::PinsPinCycle => write!(f, "pins pin cycle"),
            Intent::SidebarSessionClose => write!(f, "sidebar session close"),
            Intent::SidebarSessionTeardown => write!(f, "sidebar session teardown"),
            Intent::SidebarSessionArchive => write!(f, "sidebar session archive"),
            Intent::SidebarPersonaEdit => write!(f, "edit persona"),
            Intent::SessionNewWithLifecycle => write!(f, "new session with lifecycle"),
            Intent::SidebarSessionContinue => write!(f, "session continue"),
            Intent::ToggleJudgeAttached => write!(f, "toggle judge attached"),
            Intent::ToggleJudgeAutoReset => write!(f, "toggle judge auto-reset"),
            Intent::ResetJudge => write!(f, "reset judge"),
            Intent::ChatEntrySelectNext => write!(f, "select next entry"),
            Intent::ChatEntrySelectPrev => write!(f, "select prev entry"),
            Intent::ChatEntryPinSelected => write!(f, "pin selected entry"),
            Intent::ExpandToolEntry => write!(f, "expand tool entry"),
            Intent::ToggleIgnoredBlockVisibility => write!(f, "toggle ignored block visibility"),
            Intent::ForkFromEntry => write!(f, "fork from entry"),
            Intent::YankSelectedEntry => write!(f, "yank selected entry"),
            Intent::ChatEntryIgnoreSelected => write!(f, "ignore selected entry"),

            Intent::SessionLifecycleSetup { lifecycle_name, .. } => {
                write!(f, "session lifecycle setup: {lifecycle_name}")
            }
            Intent::SessionClose => write!(f, "session close"),
            Intent::ArgInputConfirm => write!(f, "arg input confirm"),
            Intent::SidebarResizeEnter => write!(f, "sidebar resize enter"),
            Intent::SidebarResizeExpand => write!(f, "sidebar resize expand"),
            Intent::SidebarResizeContract => write!(f, "sidebar resize contract"),
            Intent::SidebarResizeLeave => write!(f, "sidebar resize leave"),
            Intent::SidebarRenameSession => write!(f, "rename session"),
            Intent::RenameSessionConfirm => write!(f, "rename session confirm"),
            Intent::RenameSessionLeave => write!(f, "rename session leave"),
            Intent::RenameInsertChar { ch } => write!(f, "rename insert '{ch}'"),
            Intent::RenameCursorLeft => write!(f, "rename cursor left"),
            Intent::RenameCursorRight => write!(f, "rename cursor right"),
            Intent::RenameDeleteGrapheme => write!(f, "rename delete"),
            Intent::RenameDeleteForward => write!(f, "rename forward delete"),
            Intent::SwitchTab => write!(f, "switch tab"),

            // --- Workflow Navigation ---
            Intent::WorkflowNodeLeft => write!(f, "workflow node left"),
            Intent::WorkflowNodeDown => write!(f, "workflow node down"),
            Intent::WorkflowNodeUp => write!(f, "workflow node up"),
            Intent::WorkflowNodeRight => write!(f, "workflow node right"),
            Intent::WorkflowInspectToggle => write!(f, "workflow inspect toggle"),
            Intent::WorkflowInspectScrollUp => write!(f, "workflow inspect scroll up"),
            Intent::WorkflowInspectScrollDown => write!(f, "workflow inspect scroll down"),
            Intent::WorkflowEscape => write!(f, "workflow escape"),
            Intent::WorkflowRerunNode => write!(f, "workflow rerun node"),
            Intent::WorkflowRun => write!(f, "workflow run"),
            Intent::WorkflowPanLeft => write!(f, "workflow pan left"),
            Intent::WorkflowPanDown => write!(f, "workflow pan down"),
            Intent::WorkflowPanUp => write!(f, "workflow pan up"),
            Intent::WorkflowPanRight => write!(f, "workflow pan right"),

            // --- Workflow Input Editing ---
            Intent::WorkflowEditNode => write!(f, "workflow edit node"),
            Intent::WorkflowInputSubmit => write!(f, "workflow input submit"),
            Intent::WorkflowInputCancel => write!(f, "workflow input cancel"),
            Intent::WorkflowInputInsertChar { ch } => write!(f, "workflow input insert '{ch}'"),
            Intent::WorkflowInputDeleteGrapheme => write!(f, "workflow input delete"),
            Intent::WorkflowInputDeleteGraphemeForward => {
                write!(f, "workflow input forward delete")
            }
            Intent::WorkflowInputPasteText { text } => {
                let line_count = text.lines().count();
                write!(f, "workflow input paste ({line_count} lines)")
            }
            Intent::WorkflowInputCursorLeft => write!(f, "workflow input cursor left"),
            Intent::WorkflowInputCursorRight => write!(f, "workflow input cursor right"),
            Intent::WorkflowInputCursorToStart => write!(f, "workflow input cursor home"),
            Intent::WorkflowInputCursorToEnd => write!(f, "workflow input cursor end"),
            Intent::WorkflowInputCursorWordLeft => write!(f, "workflow input cursor word left"),
            Intent::WorkflowInputCursorWordRight => write!(f, "workflow input cursor word right"),
            Intent::WorkflowInputCursorUp => write!(f, "workflow input cursor up"),
            Intent::WorkflowInputCursorDown => write!(f, "workflow input cursor down"),
            Intent::ChangeCwd { root } => write!(f, "change cwd ({root})"),
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
