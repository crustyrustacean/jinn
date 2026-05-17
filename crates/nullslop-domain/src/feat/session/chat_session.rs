//! Chat session protocol — state types for a single conversation.
//!
//! [`ChatSessionState`] owns the history and streaming state for one chat session.
//! Multiple sessions can exist concurrently in the application, each identified
//! by a [`SessionId`](crate::protocol::SessionId).
//!
//! Fields are grouped into [`SessionCore`] (session-actor / context-actor)
//! and [`SessionUi`] (IntentHandler) sub-structs to make cross-boundary
//! writes visually obvious during code review.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU16, Ordering};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::feat::chat_input::ChatInputBoxState;
use crate::feat::context::strategy::types::StrategyState;
use crate::feat::session::profile::SessionProfile;
use crate::feat::session::token_stats::TokenRecord;
use crate::protocol::{
    ChatEntry, ChatEntryId, ChatEntryKind, PinPosition, PromptStrategyId, SessionId,
};

/// Ephemeral session state — lost on application restart.
///
/// Groups runtime-only fields that are specific to the current running instance
/// and have no meaning across restarts (stream indices, queues, in-progress flags).
/// The entire struct is skipped during serialization so individual fields cannot
/// be accidentally excluded from persistence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionCoreEphemeral {
    /// Index into `history` for the entry currently receiving stream tokens.
    pub(crate) streaming_entry_index: Option<usize>,
    /// Whether an LLM stream is actively producing tokens.
    pub(crate) is_streaming: bool,
    /// Messages waiting to be sent to the LLM, one at a time.
    pub(crate) message_queue: VecDeque<crate::protocol::ChatEntry>,
    /// Whether a message has been dispatched to the LLM but no tokens have arrived yet.
    pub(crate) is_sending: bool,
    /// Whether a prompt assembly request is in progress.
    pub(crate) is_assembling: bool,
    /// Maps stream tool call index to history index for in-progress tool calls.
    pub(crate) streaming_tool_call_indices: HashMap<usize, usize>,
    /// Index into `history` for the entry currently receiving thinking tokens.
    pub(crate) streaming_thinking_entry_index: Option<usize>,
    /// Cached context size in tokens (assembled prompt size).
    /// Updated when PromptAssembled fires. Not persisted across restarts.
    /// OWNER: session-actor.
    pub(crate) cached_context_size: Option<u32>,
    /// Maps tool_call_id to history index for pending streaming ToolResult entries.
    /// OWNER: session-actor.
    pub(crate) streaming_tool_result_indices: HashMap<String, usize>,
}

// Core session state — owned by session-actor and context-actor.
//
// IntentHandler is exempt and may read/write any field.
// No other actor should mutate these fields.
//
// Fields without `#[serde(skip)]` are persisted across restarts.
// All ephemeral (non-persisted) state lives in [`SessionCoreEphemeral`].

/// Serde default for the `cwd` field — resolves to the current directory.
fn default_cwd() -> std::path::PathBuf {
    std::path::PathBuf::from(".")
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCore {
    /// Unique identifier for this session.
    /// Generated at construction. Matches the HashMap key in `SessionState.sessions`.
    pub(crate) session_id: SessionId,
    /// Human-readable title. `None` until the first user message is sent.
    /// OWNER: session-actor (set on first user message, changeable by user).
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    /// When this session was last updated. Set at construction, updated on save.
    pub(crate) updated_at: Timestamp,
    /// When this session was created. Set once at construction, never mutated.
    pub(crate) created_at: Timestamp,
    /// All messages in this conversation.
    /// OWNER: session-actor (creates/removes entries, restores history)
    pub(crate) history: Vec<ChatEntry>,
    /// Per-session model and strategy selection.
    /// OWNER: provider-actor (model), context-actor (strategy via SwitchPromptStrategy command)
    pub(crate) profile: SessionProfile,
    /// Working directory for tool execution in this session.
    /// OWNER: IntentHandler (set on session creation and cd commands)
    #[serde(default = "default_cwd")]
    pub(crate) cwd: std::path::PathBuf,
    /// Token usage ledger — one immutable record per request/response pair.
    /// OWNER: session-actor (records tokens on PromptAssembled and StreamCompleted).
    #[serde(default)]
    pub(crate) token_ledger: Vec<TokenRecord>,
    /// Parent session ID, if this session was forked from another.
    /// `None` means this is a root session.
    /// OWNER: session-actor (set at session creation).
    #[serde(default)]
    pub(crate) parent_session: Option<SessionId>,
    /// Per-strategy persistent state. Keyed by strategy ID so switching
    /// strategies preserves previous state for when the user switches back.
    /// OWNER: context-actor (reads/writes during RestoreStrategyState, SwitchPromptStrategy).
    #[serde(default)]
    pub(crate) strategy_state: HashMap<PromptStrategyId, StrategyState>,
    /// Generic blob storage for future subsystems.
    #[serde(default)]
    pub(crate) blobs: HashMap<String, JsonValue>,
    /// Name of the session lifecycle that created this session.
    /// `None` means the implicit "blank" lifecycle (no setup command).
    /// OWNER: IntentHandler (set on session creation).
    #[serde(default)]
    pub(crate) lifecycle_name: Option<String>,
    /// Arguments passed to the lifecycle setup command.
    /// Replayed during teardown so the same args are available.
    /// OWNER: IntentHandler (set on session creation).
    #[serde(default)]
    pub(crate) lifecycle_args: Vec<String>,
    /// Runtime-only state — not persisted across restarts.
    #[serde(skip)]
    pub(crate) ephemeral: SessionCoreEphemeral,
}

impl Default for SessionCore {
    fn default() -> Self {
        Self {
            session_id: SessionId::new(),
            title: None,
            updated_at: Timestamp::now(),
            created_at: Timestamp::now(),
            history: Vec::new(),
            profile: SessionProfile::default(),
            cwd: std::path::PathBuf::from("."),
            token_ledger: Vec::new(),
            parent_session: None,
            strategy_state: HashMap::new(),
            blobs: HashMap::new(),
            lifecycle_name: None,
            lifecycle_args: Vec::new(),
            ephemeral: SessionCoreEphemeral::default(),
        }
    }
}

/// UI state for a session — owned by IntentHandler (exempt from ownership restrictions).
///
/// These fields control visual presentation: scroll position, selection, input text.
#[derive(Debug)]
pub struct SessionUi {
    /// The user's in-progress message for this session.
    pub(crate) chat_input: ChatInputBoxState,
    /// Number of lines to skip from the top when rendering (ratatui scroll offset).
    ///
    /// `None` means "show the bottom of the conversation" (auto-scroll).
    /// `Some(n)` means the user has manually scrolled to offset `n`.
    pub(crate) scroll_offset: Option<u16>,
    /// The index of the currently selected chat entry, if any.
    ///
    /// Used by j/k navigation in Normal mode for targeting entries
    /// for actions like pinning. `None` means no entry is selected.
    pub(crate) selected_entry_index: Option<usize>,
    /// The maximum scroll offset computed during the last render.
    ///
    /// Used by scroll handlers to resolve the "at bottom" sentinel into
    /// a concrete offset so `scroll_up` / `scroll_down` work correctly.
    /// Uses `AtomicU16` for interior mutability since the element receives `&self`.
    pub(crate) last_max_offset: AtomicU16,
    /// Per-entry wrapped line ranges computed by the renderer each frame.
    ///
    /// `entry_line_ranges[i] = (start_wrapped_line, end_wrapped_line)` in wrapped
    /// coordinate space. Used by intent handlers to determine which entries are
    /// visible in the viewport.
    pub(crate) entry_line_ranges: RwLock<Vec<(u16, u16)>>,
    /// The viewport height (render area height) set by the renderer each frame.
    pub(crate) viewport_height: AtomicU16,
    /// Number of blank lines prepended by the renderer for bottom-alignment.
    pub(crate) blank_count: AtomicU16,
    /// The set of chat entry IDs whose tool result content is expanded.
    ///
    /// When a tool result entry is expanded, its full content is shown
    /// instead of being truncated. This is ephemeral UI state — not persisted.
    pub(crate) expanded_entries: HashSet<ChatEntryId>,
}

impl Clone for SessionUi {
    fn clone(&self) -> Self {
        Self {
            chat_input: self.chat_input.clone(),
            scroll_offset: self.scroll_offset,
            selected_entry_index: self.selected_entry_index,
            last_max_offset: AtomicU16::new(self.last_max_offset.load(Ordering::Relaxed)),
            entry_line_ranges: RwLock::new(
                self.entry_line_ranges
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            ),
            viewport_height: AtomicU16::new(self.viewport_height.load(Ordering::Relaxed)),
            blank_count: AtomicU16::new(self.blank_count.load(Ordering::Relaxed)),
            expanded_entries: self.expanded_entries.clone(),
        }
    }
}

impl Default for SessionUi {
    fn default() -> Self {
        Self {
            chat_input: ChatInputBoxState::new(),
            scroll_offset: None,
            selected_entry_index: None,
            last_max_offset: AtomicU16::new(0),
            entry_line_ranges: RwLock::new(Vec::new()),
            viewport_height: AtomicU16::new(0),
            blank_count: AtomicU16::new(0),
            expanded_entries: HashSet::new(),
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSessionState {
    /// Core domain state managed by session-actor and context-actor.
    #[serde(flatten)]
    pub(crate) core: SessionCore,
    /// UI state managed by IntentHandler.
    #[serde(skip)]
    pub(crate) ui: SessionUi,
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

    /// Create a new session with a specific profile (model + strategy).
    #[must_use]
    pub fn new_with_profile(profile: SessionProfile) -> Self {
        Self {
            core: SessionCore {
                profile,
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

    /// The session's persona name.
    pub fn persona_name(&self) -> &str {
        &self.core.profile.persona_name
    }

    /// Set the session's persona name.
    pub fn set_persona_name(&mut self, name: String) {
        self.core.profile.persona_name = name;
    }

    /// Read-only access to the conversation history.
    pub fn history(&self) -> &[ChatEntry] {
        &self.core.history
    }

    /// Whether this session has no history entries.
    ///
    /// A session is "empty" when it has never had any entries pushed —
    /// no user messages, no system messages, nothing.
    /// Not to be confused with [`Self::is_idle`] which checks
    /// streaming/sending/assembling state.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.core.history.is_empty()
    }

    /// Append an entry to the history and return its index.
    ///
    /// Implements smart auto-scroll: only resets scroll and advances cursor
    /// to the new entry if the cursor was on the previous last entry (or history
    /// was empty). Otherwise, appends silently — preserving the user's scroll
    /// position and selection.
    pub fn push_entry(&mut self, entry: ChatEntry) -> usize {
        let prev_last = self.core.history.len().saturating_sub(1);
        let was_at_last = self.ui.selected_entry_index.is_none_or(|i| i == prev_last);
        let index = self.core.history.len();
        self.core.history.push(entry);
        if was_at_last {
            self.reset_scroll();
            let new_last = self.core.history.len() - 1;
            self.ui.selected_entry_index = Some(new_last);
        }
        index
    }

    /// Lazily create the Assistant entry for the current stream.
    ///
    /// Called on first `append_stream_token`, `finish_streaming`,
    /// `begin_tool_call`, or `cancel_streaming`. No-op if the entry
    /// already exists or the session is not streaming.
    fn ensure_assistant_entry(&mut self) {
        if self.core.ephemeral.streaming_entry_index.is_some() || !self.core.ephemeral.is_streaming
        {
            return;
        }
        let entry = ChatEntry::assistant("");
        let index = self.push_entry(entry);
        self.core.ephemeral.streaming_entry_index = Some(index);
    }

    /// Begin a new streaming response.
    ///
    /// Sets the streaming flag but does NOT create an Assistant entry.
    /// The entry is created lazily on first `append_stream_token`,
    /// `begin_tool_call`, or `finish_streaming`. This ensures entries are
    /// always appended in the correct order (thinking before assistant)
    /// without any index-shifting insertions.
    ///
    /// # Panics
    ///
    /// Panics if the session is already streaming. This is a programming error —
    /// the caller must ensure the previous stream has finished or been cancelled
    /// before starting a new one.
    pub fn begin_streaming(&mut self) {
        assert!(
            !self.core.ephemeral.is_streaming,
            "begin_streaming called while already streaming"
        );
        self.core.ephemeral.is_streaming = true;
    }

    /// Append a token to the streaming assistant entry.
    ///
    /// Lazily creates the Assistant entry if this is the first token.
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
        reason = "streaming_entry_index guaranteed by ensure_assistant_entry"
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
            self.core.ephemeral.is_streaming,
            "append_stream_token called while not streaming"
        );
        self.ensure_assistant_entry();
        let index = self
            .core
            .ephemeral
            .streaming_entry_index
            .expect("streaming_entry_index must be set after ensure_assistant_entry");
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

    /// Begin accumulating thinking tokens.
    ///
    /// Appends an empty `Thinking` entry to the history. The Assistant entry
    /// is created lazily later (on first `append_stream_token` or `finish_streaming`),
    /// so entries naturally appear in order: thinking before assistant.
    ///
    /// # Panics
    ///
    /// Panics if the session is not streaming, or if thinking has already begun.
    pub fn begin_thinking(&mut self) {
        assert!(
            self.core.ephemeral.is_streaming,
            "begin_thinking called while not streaming"
        );
        assert!(
            self.core.ephemeral.streaming_thinking_entry_index.is_none(),
            "begin_thinking called while already thinking"
        );
        let entry = ChatEntry::thinking("");
        let index = self.push_entry(entry);
        self.core.ephemeral.streaming_thinking_entry_index = Some(index);
    }

    /// Append a thinking token to the streaming Thinking entry.
    ///
    /// # Panics
    ///
    /// Panics if `begin_thinking()` has not been called.
    #[expect(clippy::indexing_slicing, reason = "index set by begin_thinking")]
    #[expect(
        clippy::expect_used,
        reason = "streaming invariant guaranteed by begin_thinking"
    )]
    pub fn append_thinking_token<S>(&mut self, token: S)
    where
        S: AsRef<str>,
    {
        let index = self
            .core
            .ephemeral
            .streaming_thinking_entry_index
            .expect("streaming_thinking_entry_index must be set");
        if let ChatEntry {
            kind: ChatEntryKind::Thinking(ref mut text),
            ..
        } = self.core.history[index]
        {
            text.push_str(token.as_ref());
        }
    }

    /// The index of the streaming thinking entry, if thinking is being accumulated.
    pub fn streaming_thinking_entry_index(&self) -> Option<usize> {
        self.core.ephemeral.streaming_thinking_entry_index
    }

    /// Mark streaming as finished (normal completion).
    ///
    /// Creates an empty Assistant entry if no tokens were ever appended
    /// (e.g., a stream that ended immediately).
    pub fn finish_streaming(&mut self) {
        self.ensure_assistant_entry();
        self.core.ephemeral.is_streaming = false;
        self.core.ephemeral.is_sending = false; // defensive: clear both on finish
        self.core.ephemeral.streaming_entry_index = None;
        self.core.ephemeral.streaming_tool_call_indices.clear();
        self.core.ephemeral.streaming_thinking_entry_index = None;
    }

    /// Cancel streaming but keep partial text in history.
    ///
    /// If an Assistant entry was created, its partial text is preserved.
    /// If no entry was created (stream cancelled before any tokens), just clears flags.
    pub fn cancel_streaming(&mut self) {
        self.ensure_assistant_entry();
        self.core.ephemeral.is_streaming = false;
        self.core.ephemeral.is_sending = false; // defensive: clear both on cancel
        self.core.ephemeral.streaming_entry_index = None;
        self.core.ephemeral.streaming_tool_call_indices.clear();
        self.core.ephemeral.streaming_thinking_entry_index = None;
        self.core.ephemeral.streaming_tool_result_indices.clear();
    }

    /// Cancel streaming and drain queued messages back to the input buffer.
    ///
    /// Used when the user interrupts or switches to Normal mode during an
    /// active stream. The display text from drained entries is joined with
    /// newlines and replaces whatever was in the input box.
    pub fn cancel_stream_and_drain(&mut self) {
        self.cancel_streaming();
        let drained = self.drain_queue();
        let display_texts: Vec<&str> = drained
            .iter()
            .filter_map(|e| match &e.kind {
                ChatEntryKind::User { display, .. } => Some(display.as_str()),
                _ => None,
            })
            .collect();
        let drained_text = display_texts.join("\n");
        if !drained_text.is_empty() {
            self.chat_input_mut().replace_all(drained_text);
        }
    }

    /// Whether an LLM stream is actively producing tokens.
    pub fn is_streaming(&self) -> bool {
        self.core.ephemeral.is_streaming
    }

    // --- Tool call streaming ---

    /// Create a placeholder `ToolCall` entry and record its history index.
    ///
    /// Called when `ToolUseStarted` arrives — the tool name is known but arguments
    /// are still streaming in.
    pub fn begin_tool_call(&mut self, index: usize, id: &str, name: &str) {
        self.ensure_assistant_entry();
        let entry = ChatEntry::tool_call(id, name, "");
        let history_index = self.push_entry(entry);
        self.core
            .ephemeral
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
            .ephemeral
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

    // --- Streaming tool result ---

    /// Create a pending ToolResult entry when a streaming tool starts executing.
    ///
    /// Creates the entry with `ToolResultStatus::Pending` and empty content,
    /// then tracks its history index for later content appends.
    pub fn begin_tool_result(&mut self, tool_call_id: &str, name: &str) {
        let entry = ChatEntry::tool_result(
            tool_call_id,
            name,
            "",
            crate::feat::session::tool_result_status::ToolResultStatus::Pending,
        );
        let history_index = self.push_entry(entry);
        self.core
            .ephemeral
            .streaming_tool_result_indices
            .insert(tool_call_id.to_owned(), history_index);
    }

    /// Append incremental output to a pending ToolResult entry.
    ///
    /// # Panics
    ///
    /// Panics if no pending entry exists for the given `tool_call_id`.
    #[expect(
        clippy::indexing_slicing,
        reason = "index comes from begin_tool_result which always returns a valid index"
    )]
    pub fn append_tool_result_output(&mut self, tool_call_id: &str, output: &str) {
        let Some(&history_index) = self
            .core
            .ephemeral
            .streaming_tool_result_indices
            .get(tool_call_id)
        else {
            return;
        };
        if let ChatEntryKind::ToolResult {
            ref mut content,
            ..
        } = self.core.history[history_index].kind
        {
            content.push_str(output);
        }
    }

    /// Finalize a pending ToolResult entry with the final content and status.
    ///
    /// If a pending entry exists, updates it with the final content and
    /// success/failure status. If no pending entry exists (non-streaming tool),
    /// pushes a new completed entry.
    #[expect(
        clippy::indexing_slicing,
        reason = "index comes from begin_tool_result which always returns a valid index"
    )]
    pub fn finalize_tool_result(
        &mut self,
        tool_call_id: &str,
        name: &str,
        content: &str,
        success: bool,
    ) {
        let status = if success {
            crate::feat::session::tool_result_status::ToolResultStatus::Success
        } else {
            crate::feat::session::tool_result_status::ToolResultStatus::Failure
        };

        if let Some(history_index) = self
            .core
            .ephemeral
            .streaming_tool_result_indices
            .remove(tool_call_id)
        {
            // Finalize existing pending entry.
            let entry = &mut self.core.history[history_index];
            match &mut entry.kind {
                ChatEntryKind::ToolResult {
                    content: entry_content,
                    status: entry_status,
                    ..
                } => {
                    *entry_content = content.to_owned();
                    *entry_status = status;
                }
                _ => {}
            }
        } else {
            // Non-streaming tool — push a new completed entry.
            self.push_entry(ChatEntry::tool_result(tool_call_id, name, content, status));
        }
    }

    // --- Queue ---

    /// Read-only access to the message queue.
    pub fn queue(&self) -> &VecDeque<crate::protocol::ChatEntry> {
        &self.core.ephemeral.message_queue
    }

    /// Number of messages waiting in the queue.
    pub fn queue_len(&self) -> usize {
        self.core.ephemeral.message_queue.len()
    }

    /// Push a message onto the back of the queue.
    pub fn enqueue_message(&mut self, entry: crate::protocol::ChatEntry) {
        self.core.ephemeral.message_queue.push_back(entry);
    }

    /// Pop the front message from the queue, if any.
    pub fn dequeue_message(&mut self) -> Option<crate::protocol::ChatEntry> {
        self.core.ephemeral.message_queue.pop_front()
    }

    /// Drain all queued messages, returning them in order.
    pub fn drain_queue(&mut self) -> std::collections::VecDeque<crate::protocol::ChatEntry> {
        std::mem::take(&mut self.core.ephemeral.message_queue)
    }

    // --- Assembling ---

    /// Mark the session as having a prompt assembly in progress.
    ///
    /// # Panics
    ///
    /// Panics if already sending, streaming, or assembling.
    pub fn begin_assembling(&mut self) {
        assert!(
            !self.core.ephemeral.is_sending
                && !self.core.ephemeral.is_streaming
                && !self.core.ephemeral.is_assembling,
            "begin_assembling called while already busy"
        );
        self.core.ephemeral.is_assembling = true;
    }

    /// Clear the assembling flag (called when prompt assembly completes).
    ///
    /// # Panics
    ///
    /// Panics if called while not in the assembling state.
    pub fn finish_assembling(&mut self) {
        assert!(
            self.core.ephemeral.is_assembling,
            "finish_assembling called while not assembling"
        );
        self.core.ephemeral.is_assembling = false;
    }

    /// Whether a prompt assembly is in progress.
    pub fn is_assembling(&self) -> bool {
        self.core.ephemeral.is_assembling
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
            !self.core.ephemeral.is_sending && !self.core.ephemeral.is_streaming,
            "begin_sending called while already sending or streaming"
        );
        self.core.ephemeral.is_sending = true;
    }

    /// Clear the sending flag (called when the first stream token arrives).
    ///
    /// # Panics
    ///
    /// Panics if not currently sending.
    pub fn finish_sending(&mut self) {
        assert!(
            self.core.ephemeral.is_sending,
            "finish_sending called while not sending"
        );
        self.core.ephemeral.is_sending = false;
    }

    /// Whether a message has been dispatched but no tokens have arrived yet.
    pub fn is_sending(&self) -> bool {
        self.core.ephemeral.is_sending
    }

    // --- Combined status ---

    /// Whether the session is completely idle (not sending, not streaming, not assembling).
    pub fn is_idle(&self) -> bool {
        !self.core.ephemeral.is_sending
            && !self.core.ephemeral.is_streaming
            && !self.core.ephemeral.is_assembling
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

    // --- Renderer viewport state ---

    /// Store per-entry wrapped line ranges computed by the renderer.
    ///
    /// `entry_line_ranges[i] = (start_wrapped_line, end_wrapped_line)` in the
    /// wrapped coordinate space. Called each frame by the chat log renderer.
    pub fn set_entry_line_ranges(&self, ranges: Vec<(u16, u16)>) {
        if let Ok(mut guard) = self.ui.entry_line_ranges.write() {
            *guard = ranges;
        }
    }

    /// Store the viewport height (render area height) from the renderer.
    pub fn set_viewport_height(&self, height: u16) {
        self.ui.viewport_height.store(height, Ordering::Relaxed);
    }

    /// Read the cached viewport height.
    pub fn viewport_height_value(&self) -> u16 {
        self.ui.viewport_height.load(Ordering::Relaxed)
    }

    /// Store the blank line count prepended for bottom-alignment.
    pub fn set_blank_count(&self, count: u16) {
        self.ui.blank_count.store(count, Ordering::Relaxed);
    }

    /// Returns the range of entry indices visible in the current viewport.
    ///
    /// Uses `entry_line_ranges`, `blank_count`, `scroll_offset`, and
    /// `viewport_height` to determine which entries have at least one line
    /// visible. Returns an empty range if no entries are visible or viewport
    /// data is unavailable.
    pub fn visible_entry_range(&self) -> Range<usize> {
        let ranges = match self.ui.entry_line_ranges.read() {
            Ok(guard) => guard.clone(),
            Err(_) => return 0..0,
        };
        if ranges.is_empty() {
            return 0..0;
        }

        let viewport_height = self.ui.viewport_height.load(Ordering::Relaxed);
        let blank_count = self.ui.blank_count.load(Ordering::Relaxed);
        let scroll_offset = self
            .ui
            .scroll_offset
            .unwrap_or(self.ui.last_max_offset.load(Ordering::Relaxed));

        let viewport_top = scroll_offset;
        let viewport_bottom = scroll_offset.saturating_add(viewport_height);

        let mut first_visible = None;
        let mut last_visible = None;

        for (i, &(start, end)) in ranges.iter().enumerate() {
            let abs_start = start.saturating_add(blank_count);
            let abs_end = end.saturating_add(blank_count);
            if abs_end > viewport_top && abs_start < viewport_bottom {
                if first_visible.is_none() {
                    first_visible = Some(i);
                }
                last_visible = Some(i);
            }
        }

        match (first_visible, last_visible) {
            (Some(first), Some(last)) => first..last + 1,
            _ => 0..0,
        }
    }

    /// Move the cursor to the first entry visible in the viewport.
    ///
    /// No-op if no entries are visible.
    pub fn move_cursor_to_first_visible(&mut self) {
        let range = self.visible_entry_range();
        if !range.is_empty() {
            self.ui.selected_entry_index = Some(range.start);
        }
    }

    /// Move the cursor to the last entry visible in the viewport.
    ///
    /// No-op if no entries are visible.
    pub fn move_cursor_to_last_visible(&mut self) {
        let range = self.visible_entry_range();
        if !range.is_empty() {
            self.ui.selected_entry_index = Some(range.end - 1);
        }
    }

    // --- History restoration ---

    /// Restore conversation history from a persisted snapshot.
    ///
    /// Replaces the current history with the given entries. Used by session
    /// persistence to rehydrate a session from disk.
    pub fn restore_history(&mut self, entries: Vec<ChatEntry>) {
        self.core.history = entries;
        if self.core.history.is_empty() {
            self.ui.selected_entry_index = None;
        } else {
            self.ui.selected_entry_index = Some(self.core.history.len() - 1);
        }
        self.reset_scroll();
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

    /// Set the selected entry index directly.
    ///
    /// Use for programmatic selection (e.g., sidebar pin sync).
    /// Does not validate bounds — caller must ensure index is valid.
    pub fn set_selected_entry_index(&mut self, index: usize) {
        self.ui.selected_entry_index = Some(index);
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

    /// Toggles the expanded state of a tool result entry.
    ///
    /// If the entry is currently expanded, it collapses. Otherwise, it expands.
    pub fn toggle_expand_entry(&mut self, id: ChatEntryId) {
        if self.ui.expanded_entries.contains(&id) {
            self.ui.expanded_entries.remove(&id);
        } else {
            self.ui.expanded_entries.insert(id);
        }
    }

    /// Whether a tool result entry is currently expanded to show full content.
    pub fn is_entry_expanded(&self, id: &ChatEntryId) -> bool {
        self.ui.expanded_entries.contains(id)
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
        self.core.ephemeral.cached_context_size
    }

    /// Update the cached context size.
    pub fn set_context_size(&mut self, size: u32) {
        self.core.ephemeral.cached_context_size = Some(size);
    }

    /// Restore the token ledger from persisted data.
    pub fn restore_token_ledger(&mut self, records: Vec<TokenRecord>) {
        self.core.token_ledger = records;
    }

    /// Restore the parent session from persisted data.
    pub fn restore_parent_session(&mut self, parent: Option<SessionId>) {
        self.core.parent_session = parent;
    }

    /// Restore the updated_at timestamp from persisted data.
    pub fn restore_updated_at(&mut self, ts: jiff::Timestamp) {
        self.core.updated_at = ts;
    }

    /// Restore the creation timestamp from persisted data.
    pub fn restore_created_at(&mut self, ts: jiff::Timestamp) {
        self.core.created_at = ts;
    }

    // --- New durable field accessors ---

    /// This session's unique identifier.
    pub fn session_id(&self) -> &SessionId {
        &self.core.session_id
    }

    /// Set the session ID (used when inserting into a HashMap with an external key).
    pub(crate) fn set_session_id(&mut self, id: SessionId) {
        self.core.session_id = id;
    }

    /// The session title. `None` until the first user message.
    pub fn title(&self) -> Option<&str> {
        self.core.title.as_deref()
    }

    /// Set the session title.
    pub fn set_title(&mut self, title: String) {
        self.core.title = Some(title);
    }

    /// When this session was last updated.
    pub fn updated_at(&self) -> &Timestamp {
        &self.core.updated_at
    }

    /// When this session was created. Never changes after construction.
    pub fn created_at(&self) -> &Timestamp {
        &self.core.created_at
    }

    /// Update the timestamp to now.
    pub fn touch(&mut self) {
        self.core.updated_at = Timestamp::now();
    }

    /// Per-strategy state for this session.
    pub fn strategy_state(&self) -> &HashMap<PromptStrategyId, StrategyState> {
        &self.core.strategy_state
    }

    /// Mutable access to per-strategy state.
    pub fn strategy_state_mut(&mut self) -> &mut HashMap<PromptStrategyId, StrategyState> {
        &mut self.core.strategy_state
    }

    /// Generic blob storage for future subsystems.
    pub fn blobs(&self) -> &HashMap<String, JsonValue> {
        &self.core.blobs
    }

    /// Mutable access to generic blob storage.
    pub fn blobs_mut(&mut self) -> &mut HashMap<String, JsonValue> {
        &mut self.core.blobs
    }

    // --- Lifecycle fields ---

    /// The name of the lifecycle that created this session, if any.
    pub fn lifecycle_name(&self) -> Option<&str> {
        self.core.lifecycle_name.as_deref()
    }

    /// Set the lifecycle name.
    pub fn set_lifecycle_name(&mut self, name: Option<String>) {
        self.core.lifecycle_name = name;
    }

    /// The args used during setup (replayed for teardown).
    pub fn lifecycle_args(&self) -> &[String] {
        &self.core.lifecycle_args
    }

    /// Set the lifecycle args.
    pub fn set_lifecycle_args(&mut self, args: Vec<String>) {
        self.core.lifecycle_args = args;
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
mod chat_session_tests;
