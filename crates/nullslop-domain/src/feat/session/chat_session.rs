//! Chat session protocol — state types for a single conversation.
//!
//! [`ChatSessionState`] owns the history and streaming state for one chat session.
//! Multiple sessions can exist concurrently in the application, each identified
//! by a [`SessionId`](crate::protocol::SessionId).
//!
//! Fields are grouped into [`SessionCore`] (session-actor / context-actor)
//! and [`SessionUi`] (IntentHandler) sub-structs to make cross-boundary
//! writes visually obvious during code review.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU16, Ordering};

use serde_json::Value as JsonValue;

use crate::feat::chat_input::ChatInputBoxState;
use crate::feat::session::profile::SessionProfile;
use crate::feat::session::token_stats::TokenRecord;
use crate::protocol::{
    ChatEntry, ChatEntryId, ChatEntryKind, PinPosition, PromptStrategyId, SessionId,
};

/// Core session state — owned by session-actor and context-actor.
///
/// IntentHandler is exempt and may read/write any field.
/// No other actor should mutate these fields.
#[derive(Debug)]
pub struct SessionCore {
    /// All messages in this conversation.
    /// OWNER: session-actor (creates/removes entries, restores history)
    history: Vec<ChatEntry>,
    /// Index into `history` for the entry currently receiving stream tokens.
    /// OWNER: session-actor
    streaming_entry_index: Option<usize>,
    /// Whether an LLM stream is actively producing tokens.
    /// OWNER: session-actor
    is_streaming: bool,
    /// Messages waiting to be sent to the LLM, one at a time.
    /// OWNER: session-actor
    message_queue: VecDeque<String>,
    /// Whether a message has been dispatched to the LLM but no tokens have arrived yet.
    /// OWNER: session-actor
    is_sending: bool,
    /// Whether a prompt assembly request is in progress.
    /// OWNER: session-actor
    is_assembling: bool,
    /// Per-session model and strategy selection.
    /// OWNER: provider-actor (model), context-actor (strategy via SwitchPromptStrategy command)
    profile: SessionProfile,
    /// Maps stream tool call index to history index for in-progress tool calls.
    /// OWNER: session-actor
    streaming_tool_call_indices: HashMap<usize, usize>,
    /// Persisted strategy state blob for the active strategy.
    /// OWNER: context-actor (via RestoreStrategyState command)
    strategy_state: Option<JsonValue>,
    /// Working directory for tool execution in this session.
    /// OWNER: IntentHandler (set on session creation and cd commands)
    cwd: std::path::PathBuf,
    /// Token usage ledger — one immutable record per request/response pair.
    /// OWNER: session-actor (records tokens on PromptAssembled and StreamCompleted).
    token_ledger: Vec<TokenRecord>,
    /// Parent session ID, if this session was forked from another.
    /// `None` means this is a root session.
    /// OWNER: session-actor (set at session creation).
    parent_session: Option<SessionId>,
    /// Cached context size in tokens (assembled prompt size).
    /// Updated when PromptAssembled fires.
    /// OWNER: session-actor.
    cached_context_size: Option<u32>,
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
            profile: SessionProfile::default(),
            streaming_tool_call_indices: HashMap::new(),
            strategy_state: None,
            cwd: std::path::PathBuf::new(),
            token_ledger: Vec::new(),
            parent_session: None,
            cached_context_size: None,
        }
    }
}

/// UI state for a session — owned by IntentHandler (exempt from ownership restrictions).
///
/// These fields control visual presentation: scroll position, selection, input text.
#[derive(Debug)]
pub struct SessionUi {
    /// The user's in-progress message for this session.
    chat_input: ChatInputBoxState,
    /// Number of lines to skip from the top when rendering (ratatui scroll offset).
    ///
    /// `None` means "show the bottom of the conversation" (auto-scroll).
    /// `Some(n)` means the user has manually scrolled to offset `n`.
    scroll_offset: Option<u16>,
    /// The index of the currently selected chat entry, if any.
    ///
    /// Used by j/k navigation in Normal mode for targeting entries
    /// for actions like pinning. `None` means no entry is selected.
    selected_entry_index: Option<usize>,
    /// The maximum scroll offset computed during the last render.
    ///
    /// Used by scroll handlers to resolve the "at bottom" sentinel into
    /// a concrete offset so `scroll_up` / `scroll_down` work correctly.
    /// Uses `AtomicU16` for interior mutability since the element receives `&self`.
    last_max_offset: AtomicU16,
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
    core: SessionCore,
    /// UI state managed by IntentHandler.
    ui: SessionUi,
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
    pub fn new_with_strategy(strategy_id: PromptStrategyId) -> Self {
        Self {
            core: SessionCore {
                profile: SessionProfile::from_config(
                    crate::feat::provider_infra::NO_PROVIDER_ID.to_owned(),
                    strategy_id,
                ),
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

    /// Begin a new streaming response.
    ///
    /// Creates an empty `Assistant` entry, marks the session as streaming,
    /// and returns the index of the new entry.
    ///
    /// # Panics
    ///
    /// Panics if the session is already streaming. This is a programming error —
    /// the caller must ensure the previous stream has finished or been cancelled
    /// before starting a new one.
    pub fn begin_streaming(&mut self) -> usize {
        assert!(
            !self.core.is_streaming,
            "begin_streaming called while already streaming"
        );
        let entry = ChatEntry::assistant("");
        let index = self.push_entry(entry);
        self.core.streaming_entry_index = Some(index);
        self.core.is_streaming = true;
        index
    }

    /// Append a token to the streaming assistant entry.
    ///
    /// # Panics
    ///
    /// Panics if the session is not streaming. This is a programming error.
    #[expect(
        clippy::indexing_slicing,
        reason = "index comes from push_entry which always returns a valid index"
    )]
    #[expect(
        clippy::expect_used,
        reason = "streaming_entry_index invariant guaranteed by begin_streaming"
    )]
    #[expect(
        clippy::panic,
        reason = "streaming invariant violated: entry must be Assistant during active stream"
    )]
    pub fn append_stream_token<S>(&mut self, token: S)
    where
        S: AsRef<str>,
    {
        assert!(
            self.core.is_streaming,
            "append_stream_token called while not streaming"
        );
        let index = self
            .core
            .streaming_entry_index
            .expect("streaming_entry_index must be set when is_streaming");
        if let ChatEntry {
            kind: ChatEntryKind::Assistant(ref mut text),
            ..
        } = self.core.history[index]
        {
            text.push_str(token.as_ref());
        } else {
            panic!("streaming entry is not an Assistant entry");
        }
    }

    /// Mark streaming as finished (normal completion).
    pub fn finish_streaming(&mut self) {
        self.core.is_streaming = false;
        self.core.is_sending = false; // defensive: clear both on finish
        self.core.streaming_entry_index = None;
        self.core.streaming_tool_call_indices.clear();
    }

    /// Cancel streaming but keep partial text in history.
    pub fn cancel_streaming(&mut self) {
        self.core.is_streaming = false;
        self.core.is_sending = false; // defensive: clear both on cancel
        self.core.streaming_entry_index = None;
        self.core.streaming_tool_call_indices.clear();
    }

    /// Cancel streaming and drain queued messages back to the input buffer.
    ///
    /// Used when the user interrupts or switches to Normal mode during an
    /// active stream. The drained queue text is joined with newlines and
    /// replaces whatever was in the input box.
    pub fn cancel_stream_and_drain(&mut self) {
        self.cancel_streaming();
        let drained: Vec<String> = self.drain_queue().into_iter().collect();
        let drained_text = drained.join("\n");
        if !drained_text.is_empty() {
            self.chat_input_mut().replace_all(drained_text);
        }
    }

    /// Whether an LLM stream is actively producing tokens.
    pub fn is_streaming(&self) -> bool {
        self.core.is_streaming
    }

    // --- Tool call streaming ---

    /// Create a placeholder `ToolCall` entry and record its history index.
    ///
    /// Called when `ToolUseStarted` arrives — the tool name is known but arguments
    /// are still streaming in.
    pub fn begin_tool_call(&mut self, index: usize, id: &str, name: &str) {
        let entry = ChatEntry::tool_call(id, name, "");
        let history_index = self.push_entry(entry);
        self.core
            .streaming_tool_call_indices
            .insert(index, history_index);
    }

    /// Append an incremental delta to a streaming tool call's arguments.
    ///
    /// `partial_json` is appended to the existing arguments string — it is *not*
    /// the accumulated total.
    ///
    /// # Panics
    ///
    /// Panics if no tool call entry is tracked for the given stream index.
    #[expect(
        clippy::indexing_slicing,
        reason = "index comes from push_entry which always returns a valid index"
    )]
    #[expect(
        clippy::expect_used,
        reason = "stream index is always tracked before delta arrives"
    )]
    pub fn append_tool_call_delta(&mut self, index: usize, partial_json: &str) {
        let history_index = self
            .core
            .streaming_tool_call_indices
            .get(&index)
            .copied()
            .expect("append_tool_call_delta: no entry tracked for this stream index");
        if let ChatEntryKind::ToolCall {
            ref mut arguments, ..
        } = self.core.history[history_index].kind
        {
            arguments.push_str(partial_json);
        }
    }

    /// Overwrite a tool call entry with the final complete arguments.
    ///
    /// Searches recent history for a `ToolCall` entry matching the given ID.
    /// If not found (shouldn't happen in normal flow), pushes a new entry.
    #[cfg(test)]
    pub(crate) fn finalize_tool_call(&mut self, id: &str, name: &str, arguments: &str) {
        for entry in self.core.history.iter_mut().rev() {
            if let ChatEntryKind::ToolCall {
                id: ref entry_id, ..
            } = entry.kind
                && entry_id == id
            {
                entry.kind = ChatEntryKind::ToolCall {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    arguments: arguments.to_owned(),
                };
                return;
            }
        }
        // If not found (shouldn't happen), push a new entry.
        self.push_entry(ChatEntry::tool_call(id, name, arguments));
    }

    // --- Queue ---

    /// Read-only access to the message queue.
    pub fn queue(&self) -> &VecDeque<String> {
        &self.core.message_queue
    }

    /// Number of messages waiting in the queue.
    pub fn queue_len(&self) -> usize {
        self.core.message_queue.len()
    }

    /// Push a message onto the back of the queue.
    pub fn enqueue_message(&mut self, text: String) {
        self.core.message_queue.push_back(text);
    }

    /// Pop the front message from the queue, if any.
    pub fn dequeue_message(&mut self) -> Option<String> {
        self.core.message_queue.pop_front()
    }

    /// Drain all queued messages, returning them in order.
    pub fn drain_queue(&mut self) -> VecDeque<String> {
        std::mem::take(&mut self.core.message_queue)
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
    pub fn switch_strategy(&mut self, strategy_id: PromptStrategyId) {
        self.core.profile.strategy = strategy_id;
    }

    /// The currently active prompt strategy.
    pub fn active_strategy(&self) -> &PromptStrategyId {
        &self.core.profile.strategy
    }

    /// Read-only access to the session profile.
    pub fn profile(&self) -> &SessionProfile {
        &self.core.profile
    }

    /// Mutable access to the session profile.
    pub fn profile_mut(&mut self) -> &mut SessionProfile {
        &mut self.core.profile
    }

    /// Set the model for this session.
    pub fn set_model(&mut self, model: String) {
        self.core.profile.model = model;
    }

    /// The model for this session.
    pub fn model(&self) -> &str {
        &self.core.profile.model
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

    /// The current scroll offset (lines to skip from top).
    ///
    /// Returns `None` when auto-scrolled to the bottom, or `Some(n)` when
    /// the user has manually scrolled to a specific offset.
    pub fn scroll_offset(&self) -> Option<u16> {
        self.ui.scroll_offset
    }

    /// Whether the conversation is scrolled to the bottom (auto-scroll position).
    pub fn is_at_bottom(&self) -> bool {
        self.ui.scroll_offset.is_none()
    }

    /// Scroll up (toward older messages) by the given number of lines.
    ///
    /// If currently at the bottom (auto-scroll), resolves to `last_max_offset` first
    /// so the scroll is relative to the actual bottom position.
    pub fn scroll_up(&mut self, amount: u16) {
        let current = self
            .ui
            .scroll_offset
            .unwrap_or(self.ui.last_max_offset.load(Ordering::Relaxed));
        self.ui.scroll_offset = Some(current.saturating_sub(amount));
    }

    /// Scroll down (toward newer messages) by the given number of lines.
    ///
    /// If the resulting offset reaches or exceeds `last_max_offset`, resets to
    /// auto-scroll (bottom).
    pub fn scroll_down(&mut self, amount: u16) {
        let current = self
            .ui
            .scroll_offset
            .unwrap_or(self.ui.last_max_offset.load(Ordering::Relaxed));
        let next = current.saturating_add(amount);
        if next >= self.ui.last_max_offset.load(Ordering::Relaxed) {
            self.ui.scroll_offset = None;
        } else {
            self.ui.scroll_offset = Some(next);
        }
    }

    /// Reset scroll to show the bottom of the conversation.
    pub fn reset_scroll(&mut self) {
        self.ui.scroll_offset = None;
    }

    /// Scroll to the very top of the conversation.
    pub fn scroll_to_top(&mut self) {
        self.ui.scroll_offset = Some(0);
    }

    /// Scroll to the very bottom of the conversation (auto-scroll).
    pub fn scroll_to_bottom(&mut self) {
        self.ui.scroll_offset = None;
    }

    /// Update the cached maximum scroll offset from the renderer.
    ///
    /// Called by the chat log element during each render so that
    /// scroll handlers can resolve the "at bottom" state into a concrete offset.
    pub fn set_last_max_offset(&self, max_offset: u16) {
        self.ui.last_max_offset.store(max_offset, Ordering::Relaxed);
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

    // --- Selection ---

    /// Select the next entry (moving toward newer messages).
    ///
    /// If nothing is selected, selects the first entry.
    /// Clamps to the last entry index.
    /// No-op if history is empty.
    pub fn select_next_entry(&mut self) {
        if self.core.history.is_empty() {
            return;
        }
        let max = self.core.history.len() - 1;
        self.ui.selected_entry_index = Some(
            self.ui
                .selected_entry_index
                .map_or(0, |i| i.saturating_add(1).min(max)),
        );
    }

    /// Select the previous entry (moving toward older messages).
    ///
    /// If nothing is selected, selects the last entry.
    /// Clamps to 0.
    /// No-op if history is empty.
    pub fn select_prev_entry(&mut self) {
        if self.core.history.is_empty() {
            return;
        }
        self.ui.selected_entry_index = Some(
            self.ui
                .selected_entry_index
                .map_or(self.core.history.len() - 1, |i| i.saturating_sub(1)),
        );
    }

    /// Clear the entry selection.
    pub fn clear_selection(&mut self) {
        self.ui.selected_entry_index = None;
    }

    /// The index of the currently selected entry, if any.
    pub fn selected_entry_index(&self) -> Option<usize> {
        self.ui.selected_entry_index
    }

    /// The currently selected entry, if any.
    pub fn selected_entry(&self) -> Option<&ChatEntry> {
        let i = self.ui.selected_entry_index?;
        self.core.history.get(i)
    }

    /// The ID of the currently selected entry, if any.
    pub fn selected_entry_id(&self) -> Option<&ChatEntryId> {
        self.selected_entry().map(|e| &e.id)
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

    /// Returns this session's working directory for tool execution.
    pub fn cwd(&self) -> &std::path::Path {
        &self.core.cwd
    }

    /// Sets this session's working directory.
    pub fn set_cwd(&mut self, cwd: std::path::PathBuf) {
        self.core.cwd = cwd;
    }

    // --- Token ledger ---

    /// Read-only access to the token ledger.
    pub fn token_ledger(&self) -> &[TokenRecord] {
        &self.core.token_ledger
    }

    /// Push a token record onto the ledger.
    ///
    /// Records are immutable once pushed — this is the only way to add them.
    pub fn push_token_record(&mut self, record: TokenRecord) {
        self.core.token_ledger.push(record);
    }

    /// Update the last token record's received count.
    ///
    /// Called when `StreamCompleted` arrives to finalize the pending record.
    ///
    /// # Panics
    ///
    /// Panics if the ledger is empty.
    pub fn finalize_last_token_record(&mut self, tokens_received: u32) {
        let last = self
            .core
            .token_ledger
            .last_mut()
            .expect("ledger must not be empty");
        last.tokens_received = tokens_received;
    }

    /// The parent session, if this session was forked from another.
    pub fn parent_session(&self) -> &Option<SessionId> {
        &self.core.parent_session
    }

    /// Set the parent session.
    pub fn set_parent_session(&mut self, parent: SessionId) {
        self.core.parent_session = Some(parent);
    }

    /// The cached context size in tokens, if a prompt has been assembled.
    pub fn context_size(&self) -> Option<u32> {
        self.core.cached_context_size
    }

    /// Update the cached context size.
    pub fn set_context_size(&mut self, size: u32) {
        self.core.cached_context_size = Some(size);
    }

    /// Restore the token ledger from persisted data.
    pub fn restore_token_ledger(&mut self, records: Vec<TokenRecord>) {
        self.core.token_ledger = records;
    }

    /// Restore the parent session from persisted data.
    pub fn restore_parent_session(&mut self, parent: Option<SessionId>) {
        self.core.parent_session = parent;
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
mod tests;
