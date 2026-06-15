//! The [`Intent`] enum - one variant per user-initiated action.
use crate::Bridge;
use crate::common::bridge::BridgeClosure;
use crate::common::bus::BusMessage;
use crate::protocol::{PickerKind, SessionId};

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
    /// Toggle the selected model's selected state for multi-select alloy building.
    ModelToggleSelected,
    /// Toggle the provider picker between single-model and alloy-selection modes.
    ///
    /// No-op unless the provider picker is active.
    ToggleAlloyMode,
    /// Scroll the preview pane up one page.
    PreviewScrollUp,
    /// Scroll the preview pane down one page.
    PreviewScrollDown,
    /// Create a new session.
    SessionNew,
    /// Refresh the model list from all providers.
    RefreshModels,
    /// Rescan the prompt templates directory.
    RescanPromptTemplates,
    /// Rescan the agent skills directory and reload the skill picker.
    RefreshSkills,
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
    SidebarSessionConfirm,
    /// Activate the selected session and enter Insert mode.
    SidebarConfirmInsert,
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
    /// Re-run the lifecycle setup command for the sidebar-selected session.
    /// Only valid when the session's lifecycle_script_state is NothingRan.
    SidebarSessionRerunSetup,
    /// Toggle the enabled/disabled state of the sidebar-selected plugin entry.
    /// Only valid when the cursor is on a `SessionEntryKind::Plugin` entry.
    SidebarTogglePlugin,

    // --- Chat Entry Selection ---
    /// Select the next chat entry.
    ChatEntrySelectNext,
    /// Select the previous chat entry.
    ChatEntrySelectPrev,
    /// Pin the currently selected chat entry.
    ChatEntryPinSelected,
    /// Toggle expand/collapse of the selected tool entry (tool call or tool result).
    ExpandToolEntry,
    /// Toggle visibility of the audit popup for the currently selected chat entry.
    ToggleAuditPopup,
    /// Toggle visibility of the ignored entry block at the cursor.
    ToggleIgnoredBlockVisibility,
    /// Fork the session at the currently selected chat entry.
    ForkFromEntry,
    /// Yank (copy) the currently selected chat entry to the system clipboard.
    YankSelectedEntry,
    /// Toggle the `ignored` flag on the currently selected chat entry.
    ChatEntryIgnoreSelected,
    /// Reset the currently selected chat entry's context override to `Default`.
    ChatEntryResetSelected,

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

    // --- Pruner Accumulation Input (set threshold) ---
    /// Open the pruner accumulation threshold input popup.
    OpenPrunerAccumulationInput,
    /// Confirm the pruner accumulation input and persist.
    PrunerAccumulationConfirm,
    /// Cancel the pruner accumulation input popup.
    PrunerAccumulationLeave,
    /// Insert a character into the pruner accumulation input.
    PrunerAccumulationInsertChar {
        /// The character to insert.
        ch: char,
    },
    /// Move cursor left in the pruner accumulation input.
    PrunerAccumulationCursorLeft,
    /// Move cursor right in the pruner accumulation input.
    PrunerAccumulationCursorRight,
    /// Delete the grapheme before the cursor in pruner accumulation input.
    PrunerAccumulationDeleteGrapheme,
    /// Delete the grapheme after the cursor in pruner accumulation input.
    PrunerAccumulationDeleteForward,

    // --- CWD Input (type a path) ---
    /// Open the cwd input popup (type a directory path).
    OpenCwdInput,
    /// Confirm the cwd input - resolve, validate, and apply.
    CwdInputConfirm,
    /// Cancel the cwd input popup.
    CwdInputLeave,

    // --- CWD Selection ---
    /// Change the session's working directory via an external picker.
    ChangeCwd {
        /// Where to search from.
        root: CwdRoot,
    },

    // --- Plugin ---
    /// Trigger a plugin-declared action via a registered keybind.
    ///
    /// The `description` is rendered by ratatui-which-key via the `Display` impl.
    /// `session_id` is `None` for global plugins (resolved to the active session at fire time).
    TriggerPlugin {
        /// Name of the plugin that declared the keybind.
        plugin_name: String,
        /// Action name the plugin registered for this keybind.
        action: String,
        /// Human-readable description shown in which-key help.
        description: String,
        /// Optional per-session scope; `None` means active session at fire time.
        session_id: Option<SessionId>,
    },
}

impl std::fmt::Display for Intent {
    #[expect(
        clippy::too_many_lines,
        clippy::match_same_arms,
        reason = "handler reads best as a single unit"
    )]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Intent::InsertChar { ch } => write!(f, "insert '{ch}'"),
            Intent::DeleteGrapheme => write!(f, "delete"),
            Intent::DeleteGraphemeForward => write!(f, "forward delete"),
            Intent::SubmitMessage => write!(f, "submit message"),
            Intent::ToggleInputMode => write!(f, "toggle input mode"),
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
            Intent::CtrlClear => write!(f, "ctrl-c clear"),
            Intent::EnterInsertMode => write!(f, "enter insert mode"),
            Intent::EnterNormalMode => write!(f, "enter normal mode"),
            Intent::ToggleWhichkey => write!(f, "toggle which-key"),
            Intent::NormalEscape => write!(f, "escape"),
            Intent::NoOp => write!(f, "no-op"),
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
            Intent::ModelToggleSelected => write!(f, "toggle model"),
            Intent::ToggleAlloyMode => write!(f, "toggle alloy mode"),
            Intent::PreviewScrollUp => write!(f, "preview scroll up"),
            Intent::PreviewScrollDown => write!(f, "preview scroll down"),
            Intent::SessionNew => write!(f, "new session"),
            Intent::RefreshModels => write!(f, "refresh models"),
            Intent::RescanPromptTemplates => write!(f, "rescan prompt templates"),
            Intent::RefreshSkills => write!(f, "refresh skills"),
            Intent::SidebarFocus => write!(f, "focus sidebar"),
            Intent::SidebarFocusSessions => write!(f, "focus session list"),
            Intent::SidebarLeave => write!(f, "return to normal mode"),
            Intent::SidebarMoveDown => write!(f, "cursor down"),
            Intent::SidebarMoveUp => write!(f, "cursor up"),
            Intent::SidebarSectionNext => write!(f, "cursor to next section"),
            Intent::SidebarSectionPrev => write!(f, "cursor to previous section"),
            Intent::SidebarSessionConfirm => write!(f, "activate session"),
            Intent::SidebarConfirmInsert => write!(f, "activate session -> insert mode"),
            Intent::PinsUnpin => write!(f, "unpin entry"),
            Intent::PinsPinTop => write!(f, "pin to top position"),
            Intent::PinsPinBottom => write!(f, "pin to bottom position"),
            Intent::PinsPinRelative => write!(f, "pin relative position"),
            Intent::PinsPinCycle => write!(f, "cycle pin position"),
            Intent::SidebarSessionClose => write!(f, "close session (w/teardown)"),
            Intent::SidebarSessionTeardown => write!(f, "run teardown script"),
            Intent::SidebarSessionArchive => write!(f, "archive session"),
            Intent::SidebarPersonaEdit => write!(f, "change persona"),
            Intent::SessionNewWithLifecycle => write!(f, "new session with lifecycle"),
            Intent::SidebarSessionContinue => write!(f, "continue session"),
            Intent::SidebarSessionRerunSetup => write!(f, "rerun session setup"),
            Intent::SidebarTogglePlugin => write!(f, "toggle plugin"),

            Intent::ChatEntrySelectNext => write!(f, "select next entry"),
            Intent::ChatEntrySelectPrev => write!(f, "select prev entry"),
            Intent::ChatEntryPinSelected => write!(f, "pin entry"),
            Intent::ExpandToolEntry => write!(f, "expand tool entry"),
            Intent::ToggleAuditPopup => write!(f, "toggle audit popup"),
            Intent::ToggleIgnoredBlockVisibility => write!(f, "toggle ignored block visibility"),
            Intent::ForkFromEntry => write!(f, "fork from entry"),
            Intent::YankSelectedEntry => write!(f, "yank entry"),
            Intent::ChatEntryIgnoreSelected => write!(f, "toggle entry in/out of context"),
            Intent::ChatEntryResetSelected => write!(f, "reset entry to default context"),

            Intent::SessionLifecycleSetup { lifecycle_name, .. } => {
                write!(f, "session lifecycle setup: {lifecycle_name}")
            }
            Intent::SessionClose => write!(f, "session close"),
            Intent::ArgInputConfirm => write!(f, "arg input confirm"),
            Intent::SidebarResizeEnter => write!(f, "enter 'resize sidebar' mode"),
            Intent::SidebarResizeExpand => write!(f, "expand sidebar"),
            Intent::SidebarResizeContract => write!(f, "contract sidebar"),
            Intent::SidebarResizeLeave => write!(f, "exist resize sidebar mode"),
            Intent::SidebarRenameSession => write!(f, "rename session"),
            Intent::RenameSessionConfirm => write!(f, "rename session confirm"),
            Intent::RenameSessionLeave => write!(f, "rename session leave"),
            Intent::RenameInsertChar { ch } => write!(f, "rename insert '{ch}'"),
            Intent::RenameCursorLeft => write!(f, "rename cursor left"),
            Intent::RenameCursorRight => write!(f, "rename cursor right"),
            Intent::RenameDeleteGrapheme => write!(f, "rename delete"),
            Intent::RenameDeleteForward => write!(f, "rename forward delete"),
            Intent::OpenPrunerAccumulationInput => write!(f, "set pruner accumulation threshold"),
            Intent::PrunerAccumulationConfirm => write!(f, "pruner accumulation confirm"),
            Intent::PrunerAccumulationLeave => write!(f, "pruner accumulation leave"),
            Intent::PrunerAccumulationInsertChar { ch } => {
                write!(f, "pruner accumulation insert '{ch}'")
            }
            Intent::PrunerAccumulationCursorLeft => write!(f, "pruner accumulation cursor left"),
            Intent::PrunerAccumulationCursorRight => write!(f, "pruner accumulation cursor right"),
            Intent::PrunerAccumulationDeleteGrapheme => write!(f, "pruner accumulation delete"),
            Intent::PrunerAccumulationDeleteForward => {
                write!(f, "pruner accumulation forward delete")
            }
            Intent::OpenCwdInput => write!(f, "change cwd"),
            Intent::CwdInputConfirm => write!(f, "cwd input confirm"),
            Intent::CwdInputLeave => write!(f, "cwd input leave"),

            Intent::ChangeCwd { root } => write!(f, "change cwd from '{root}'"),
            Intent::TriggerPlugin { description, .. } => write!(f, "{description}"),
        }
    }
}

/// What an intent handler returns after processing an intent.
///
/// Carries typed message closures to be dispatched to the actor system
/// via the kameo message bus.
pub struct IntentResult {
    /// Typed message closures to publish to the kameo bus.
    pub messages: Vec<BridgeClosure>,
    /// Type names of messages, for test inspection.
    pub message_names: Vec<&'static str>,
}

impl IntentResult {
    /// An empty result with no messages.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            messages: vec![],
            message_names: vec![],
        }
    }

    /// A result with a single typed message to publish to the bus.
    ///
    /// The message is wrapped in a closure that calls
    /// `bus.tell(Publish(msg)).await` when the bridge drain task processes it.
    #[must_use]
    pub fn with_message<M>(msg: M) -> Self
    where
        M: BusMessage,
    {
        Self {
            messages: vec![crate::common::bridge::Bridge::publish_closure(msg)],
            message_names: vec![std::any::type_name::<M>()],
        }
    }

    /// Append multiple messages of one type at the same time.
    #[must_use]
    pub fn with_messages<I, M>(mut self, msgs: I) -> Self
    where
        M: BusMessage,
        I: IntoIterator<Item = M>,
    {
        for msg in msgs {
            self.messages.push(Bridge::publish_closure(msg));
            self.message_names.push(std::any::type_name::<M>());
        }
        self
    }

    /// Append a typed message and return self for chaining.
    #[must_use]
    pub fn message<M: BusMessage>(mut self, msg: M) -> Self {
        self.messages.push(Bridge::publish_closure(msg));
        self.message_names.push(std::any::type_name::<M>());
        self
    }

    /// Merge another IntentResult's messages into this one.
    #[must_use]
    pub fn merge(mut self, other: IntentResult) -> Self {
        self.messages.extend(other.messages);
        self.message_names.extend(other.message_names);
        self
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::panic,
        clippy::unreachable,
        clippy::string_slice,
        clippy::uninlined_format_args,
        reason = "test code"
    )]
    use super::*;

    #[test]
    fn trigger_plugin_display_returns_description() {
        let intent = Intent::TriggerPlugin {
            plugin_name: "prompt_enrichment".into(),
            action: "on_enrich".into(),
            description: "enrich prompt".into(),
            session_id: None,
        };
        assert_eq!(intent.to_string(), "enrich prompt");
    }
}
