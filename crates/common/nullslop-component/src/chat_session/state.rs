//! State for a single chat session — history, input box, streaming progress, and subsystem state.

mod queue;
mod scroll;
mod selection;
mod streaming;

use std::collections::VecDeque;
use std::sync::atomic::AtomicU16;

use nullslop_protocol::{ChatEntry, ChatEntryId, PinPosition};
use serde_json::Value as JsonValue;

use nsslice_chat_input_box_protocol::ChatInputBoxState;

/// Core session state — owned by session-actor and context-actor.
///
/// IntentHandler is exempt and may read/write any field.
/// No other actor should mutate these fields.
#[derive(Debug)]
pub struct SessionCore {
    /// All messages in this conversation.
    /// OWNER: session-actor (creates/removes entries, restores history)
    pub(super) history: Vec<ChatEntry>,
    /// Index into `history` for the entry currently receiving stream tokens.
    /// OWNER: session-actor
    pub(super) streaming_entry_index: Option<usize>,
    /// Whether an LLM stream is actively producing tokens.
    /// OWNER: session-actor
    pub(super) is_streaming: bool,
    /// Messages waiting to be sent to the LLM, one at a time.
    /// OWNER: session-actor
    pub(super) message_queue: VecDeque<String>,
    /// Whether a message has been dispatched to the LLM but no tokens have arrived yet.
    /// OWNER: session-actor
    pub(super) is_sending: bool,
    /// Whether a prompt assembly request is in progress.
    /// OWNER: session-actor
    pub(super) is_assembling: bool,
    /// The active prompt strategy for this session.
    /// OWNER: context-actor (via SwitchPromptStrategy command)
    pub(super) active_strategy: nullslop_protocol::PromptStrategyId,
    /// Maps stream tool call index to history index for in-progress tool calls.
    /// OWNER: session-actor
    pub(super) streaming_tool_call_indices: std::collections::HashMap<usize, usize>,
    /// Persisted strategy state blob for the active strategy.
    /// OWNER: context-actor (via RestoreStrategyState command)
    pub(super) strategy_state: Option<JsonValue>,
}

impl Default for SessionCore {
    fn default() -> Self {
        Self {
            history: Vec::new(),
            streaming_entry_index: None,
            is_streaming: false,
            message_queue: VecDeque::new(),
            is_sending: false,
            is_assembling: false,
            active_strategy: nullslop_protocol::PromptStrategyId::passthrough(),
            streaming_tool_call_indices: std::collections::HashMap::new(),
            strategy_state: None,
        }
    }
}

/// UI state for a session — owned by IntentHandler (exempt from ownership restrictions).
///
/// These fields control visual presentation: scroll position, selection, input text.
#[derive(Debug)]
pub struct SessionUi {
    /// The user's in-progress message for this session.
    pub(super) chat_input: ChatInputBoxState,
    /// Number of lines to skip from the top when rendering (ratatui scroll offset).
    ///
    /// `None` means "show the bottom of the conversation" (auto-scroll).
    /// `Some(n)` means the user has manually scrolled to offset `n`.
    pub(super) scroll_offset: Option<u16>,
    /// The index of the currently selected chat entry, if any.
    ///
    /// Used by j/k navigation in Normal mode for targeting entries
    /// for actions like pinning. `None` means no entry is selected.
    pub(super) selected_entry_index: Option<usize>,
    /// The maximum scroll offset computed during the last render.
    ///
    /// Used by scroll handlers to resolve the "at bottom" sentinel into
    /// a concrete offset so `scroll_up` / `scroll_down` work correctly.
    /// Uses `AtomicU16` for interior mutability since the element receives `&self`.
    pub(super) last_max_offset: AtomicU16,
}

impl Default for SessionUi {
    fn default() -> Self {
        Self {
            chat_input: ChatInputBoxState::new(),
            scroll_offset: None,
            selected_entry_index: None,
            last_max_offset: AtomicU16::new(0),
        }
    }
}

/// The state of a single chat session.
///
/// Owns the conversation history and tracks whether an LLM response is
/// currently streaming in. The streaming entry is an in-progress `Assistant`
/// entry at a known index — tokens are appended to it until the stream
/// completes or is cancelled.
///
/// Fields are grouped into [`SessionCore`] (session-actor / context-actor)
/// and [`SessionUi`] (IntentHandler) sub-structs to make cross-boundary
/// writes visually obvious during code review.
#[derive(Debug)]
pub struct ChatSessionState {
    /// Core domain state managed by session-actor and context-actor.
    pub(super) core: SessionCore,
    /// UI state managed by IntentHandler.
    pub(super) ui: SessionUi,
}

impl ChatSessionState {
    /// Create a new session with empty history and no active stream.
    #[must_use]
    pub fn new() -> Self {
        Self {
            core: SessionCore::default(),
            ui: SessionUi::default(),
        }
    }

    /// Create a new session with a specific prompt strategy.
    #[must_use]
    pub fn new_with_strategy(strategy_id: nullslop_protocol::PromptStrategyId) -> Self {
        Self {
            core: SessionCore {
                active_strategy: strategy_id,
                ..SessionCore::default()
            },
            ui: SessionUi::default(),
        }
    }

    /// Read-only access to this session's input box state.
    pub fn chat_input(&self) -> &ChatInputBoxState {
        &self.ui.chat_input
    }

    /// Mutable access to this session's input box state.
    pub fn chat_input_mut(&mut self) -> &mut ChatInputBoxState {
        &mut self.ui.chat_input
    }

    /// Read-only access to the conversation history.
    pub fn history(&self) -> &[ChatEntry] {
        &self.core.history
    }

    /// Append an entry to the history and return its index.
    ///
    /// Resets scroll to the bottom so new messages are visible.
    pub fn push_entry(&mut self, entry: ChatEntry) -> usize {
        let index = self.core.history.len();
        self.core.history.push(entry);
        self.reset_scroll();
        self.clear_selection();
        index
    }

    // --- Assembling ---

    /// Mark the session as having a prompt assembly in progress.
    ///
    /// # Panics
    ///
    /// Panics if already sending, streaming, or assembling.
    pub fn begin_assembling(&mut self) {
        assert!(
            !self.core.is_sending && !self.core.is_streaming && !self.core.is_assembling,
            "begin_assembling called while already busy"
        );
        self.core.is_assembling = true;
    }

    /// Clear the assembling flag (called when prompt assembly completes).
    ///
    /// # Panics
    ///
    /// Panics if called while not in the assembling state.
    pub fn finish_assembling(&mut self) {
        assert!(
            self.core.is_assembling,
            "finish_assembling called while not assembling"
        );
        self.core.is_assembling = false;
    }

    /// Whether a prompt assembly is in progress.
    pub fn is_assembling(&self) -> bool {
        self.core.is_assembling
    }

    /// Switch the active prompt strategy for this session.
    pub fn switch_strategy(&mut self, strategy_id: nullslop_protocol::PromptStrategyId) {
        self.core.active_strategy = strategy_id;
    }

    /// The currently active prompt strategy.
    pub fn active_strategy(&self) -> &nullslop_protocol::PromptStrategyId {
        &self.core.active_strategy
    }

    // --- Sending ---

    /// Mark the session as having dispatched a message to the LLM.
    ///
    /// # Panics
    ///
    /// Panics if already sending or streaming. This is a programming error —
    /// the caller must ensure the session is idle before dispatching.
    pub fn begin_sending(&mut self) {
        assert!(
            !self.core.is_sending && !self.core.is_streaming,
            "begin_sending called while already sending or streaming"
        );
        self.core.is_sending = true;
    }

    /// Clear the sending flag (called when the first stream token arrives).
    ///
    /// # Panics
    ///
    /// Panics if not currently sending.
    pub fn finish_sending(&mut self) {
        assert!(
            self.core.is_sending,
            "finish_sending called while not sending"
        );
        self.core.is_sending = false;
    }

    /// Whether a message has been dispatched but no tokens have arrived yet.
    pub fn is_sending(&self) -> bool {
        self.core.is_sending
    }

    // --- Combined status ---

    /// Whether the session is completely idle (not sending, not streaming, not assembling).
    pub fn is_idle(&self) -> bool {
        !self.core.is_sending && !self.core.is_streaming && !self.core.is_assembling
    }

    // --- History restoration ---

    /// Restore conversation history from a persisted snapshot.
    ///
    /// Replaces the current history with the given entries. Used by session
    /// persistence to rehydrate a session from disk.
    pub fn restore_history(&mut self, entries: Vec<ChatEntry>) {
        self.core.history = entries;
        self.clear_selection();
    }

    // --- Pinning ---

    /// Pin an entry by ID, setting its pin position.
    ///
    /// If no entry with the given ID exists, this is a no-op.
    pub fn pin_entry(&mut self, id: &ChatEntryId, position: PinPosition) {
        if let Some(entry) = self.core.history.iter_mut().find(|e| e.id == *id) {
            entry.pin_position = Some(position);
        }
    }

    /// Unpin an entry by ID, clearing its pin position.
    ///
    /// If no entry with the given ID exists, this is a no-op.
    pub fn unpin_entry(&mut self, id: &ChatEntryId) {
        if let Some(entry) = self.core.history.iter_mut().find(|e| e.id == *id) {
            entry.pin_position = None;
        }
    }

    /// Returns all pinned entries in history order.
    pub fn pinned_entries(&self) -> Vec<&ChatEntry> {
        self.core.history.iter().filter(|e| e.is_pinned()).collect()
    }

    // --- Strategy state ---

    /// Read-only access to the persisted strategy state blob.
    pub fn strategy_state(&self) -> Option<&JsonValue> {
        self.core.strategy_state.as_ref()
    }

    /// Update the strategy state blob.
    pub fn set_strategy_state(&mut self, blob: JsonValue) {
        self.core.strategy_state = Some(blob);
    }
}

impl Default for ChatSessionState {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for constructing [`ChatSessionState`] in tests.
///
/// Replays operations sequentially on `build()`. Example:
///
/// ```ignore
/// let mut session = ChatSessionState::builder()
///     .with_user_entry("hello")
///     .begin_streaming()
///     .build();
/// session.append_stream_token("world");
/// ```
#[cfg(test)]
#[derive(Debug, Default)]
pub struct ChatSessionStateBuilder {
    ops: Vec<BuilderOp>,
}

#[cfg(test)]
#[derive(Debug)]
enum BuilderOp {
    PushEntry(ChatEntry),
    BeginStreaming,
    BeginSending,
    PinLast(PinPosition),
}

#[cfg(test)]
impl ChatSessionStateBuilder {
    /// Push a user entry onto the history.
    pub fn with_user_entry(mut self, text: &str) -> Self {
        self.ops.push(BuilderOp::PushEntry(ChatEntry::user(text)));
        self
    }

    /// Push any entry onto the history.
    pub fn with_entry(mut self, entry: ChatEntry) -> Self {
        self.ops.push(BuilderOp::PushEntry(entry));
        self
    }

    /// Begin streaming (creates an empty Assistant entry and sets `is_streaming`).
    pub fn begin_streaming(mut self) -> Self {
        self.ops.push(BuilderOp::BeginStreaming);
        self
    }

    /// Mark the session as sending.
    pub fn begin_sending(mut self) -> Self {
        self.ops.push(BuilderOp::BeginSending);
        self
    }

    /// Pin the most recently pushed entry at the given position.
    pub fn with_pin(mut self, position: PinPosition) -> Self {
        self.ops.push(BuilderOp::PinLast(position));
        self
    }

    /// Build the session by replaying all stored operations.
    pub fn build(self) -> ChatSessionState {
        let mut session = ChatSessionState::new();
        let mut last_id: Option<ChatEntryId> = None;
        for op in self.ops {
            match op {
                BuilderOp::PushEntry(entry) => {
                    let id = entry.id.clone();
                    session.push_entry(entry);
                    last_id = Some(id);
                }
                BuilderOp::BeginStreaming => {
                    session.begin_streaming();
                }
                BuilderOp::BeginSending => {
                    session.begin_sending();
                }
                BuilderOp::PinLast(position) => {
                    if let Some(ref id) = last_id {
                        session.pin_entry(id, position);
                    }
                }
            }
        }
        session
    }
}

#[cfg(test)]
impl ChatSessionState {
    /// Create a test builder.
    pub fn builder() -> ChatSessionStateBuilder {
        ChatSessionStateBuilder::default()
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod state_tests;
