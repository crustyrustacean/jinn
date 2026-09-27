//! The [`Intent`] enum - one variant per user-initiated action.
use std::sync::Arc;

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
    /// Toggle the which-key popup.
    ToggleWhichkey,
    /// Escape key in Normal mode: cancel selection.
    NormalEscape,
    /// No-op intent produced by unmapped keys in scopes with confirmation prompts.
    /// Dismisses any active confirmation prompt via the pre-match interceptors.
    NoOp,

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
            KernelIntent::ToggleWhichkey => write!(f, "toggle which-key"),
            KernelIntent::NormalEscape => write!(f, "escape"),
            KernelIntent::NoOp => write!(f, "no-op"),
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
