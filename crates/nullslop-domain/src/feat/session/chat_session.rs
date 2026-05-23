//! Chat session protocol — state types for a single conversation.
//!
//! [`ChatSessionState`] owns the history and streaming state for one chat session.
//! Multiple sessions can exist concurrently in the application, each identified
//! by a [`SessionId`](crate::protocol::SessionId).
//!
//! Fields are grouped into [`SessionCore`] (session-actor / context-actor)
//! and [`SessionUi`] (IntentHandler) sub-structs to make cross-boundary
//! writes visually obvious during code review.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU16, Ordering};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::feat::chat_input::ChatInputBoxState;
use crate::feat::ui::chat_log::visual_item::VisualItem;
use crate::feat::context::strategy::types::StrategyState;
use crate::feat::session::chat_history::ChatHistory;
use crate::feat::session::profile::SessionProfile;
use crate::feat::session::token_stats::TokenRecord;
use crate::protocol::{
    ChatEntry, ChatEntryId, ChatEntryKind, PinPosition, PromptStrategyId, SessionId,
};

/// Ephemeral session state — lost on application restart.
///
/// The current phase of a chat session's lifecycle.
///
/// Phases are mutually exclusive — a session is in exactly one phase at a time.
/// Transitions are enforced by the `begin_*`/`finish_*` methods with assertions
/// that document the valid state machine edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SessionPhase {
    /// Session is completely idle — not sending, streaming, assembling, or compacting.
    #[default]
    Idle,
    /// A prompt assembly request is in progress.
    Assembling,
    /// A message has been dispatched to the LLM but no tokens have arrived yet.
    Sending,
    /// LLM tokens are actively streaming into the session.
    Streaming,
    /// Context compaction is in progress.
    Compacting,
    /// A lifecycle teardown script is running.
    TearingDown,
}

impl std::str::FromStr for SessionPhase {
    type Err = SessionPhaseParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "idle" => Ok(Self::Idle),
            "assembling" => Ok(Self::Assembling),
            "sending" => Ok(Self::Sending),
            "streaming" => Ok(Self::Streaming),
            "compacting" => Ok(Self::Compacting),
            "tearing_down" => Ok(Self::TearingDown),
            _ => Err(SessionPhaseParseError(s.to_owned())),
        }
    }
}

/// Error returned when a string does not match any [`SessionPhase`] variant.
#[derive(Debug, wherror::Error)]
#[error("unknown session phase: {0}")]
pub struct SessionPhaseParseError(String);

/// Error returned when a streaming operation fails.
#[derive(Debug, wherror::Error)]
pub enum StreamingError {
    /// No streaming entry index is set.
    #[error("no streaming entry index")]
    NoStreamingEntry,
    /// The streaming entry has an unexpected kind.
    #[error("streaming entry is not an Assistant entry")]
    NotAssistantEntry,
    /// No thinking entry index is set.
    #[error("no thinking entry index")]
    NoThinkingEntry,
    /// No tool call tracked for the given stream index.
    #[error("no entry tracked for tool call stream index {index}")]
    NoToolCallIndex { index: usize },
    /// The token ledger is empty.
    #[error("token ledger is empty")]
    EmptyLedger,
}

/// Whether a session is in memory or at rest in the database.
///
/// `Loaded` sessions appear in the sidebar and are available for interaction.
/// `Archived` sessions exist only in the database and are hidden from the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    #[default]
    Loaded,
    Archived,
}

/// The lifecycle script progression for a session.
///
/// One-way transitions enforced by [`advance_after_setup`](Self::advance_after_setup)
/// and [`advance_after_teardown`](Self::advance_after_teardown).
/// These methods are only called after the corresponding script succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleScriptState {
    #[default]
    NothingRan,
    SetupRan,
    TeardownRan,
}

impl LifecycleScriptState {
    /// Transition `NothingRan → SetupRan`.
    ///
    /// Soft guard: if current state is not `NothingRan`, logs a warning and returns.
    pub fn advance_after_setup(&mut self) {
        if !matches!(self, Self::NothingRan) {
            tracing::warn!(current = ?self, "advance_after_setup: expected NothingRan, ignoring");
            return;
        }
        *self = Self::SetupRan;
    }

    /// Transition `SetupRan → TeardownRan`.
    ///
    /// Soft guard: if current state is not `SetupRan`, logs a warning and returns.
    pub fn advance_after_teardown(&mut self) {
        if !matches!(self, Self::SetupRan) {
            tracing::warn!(current = ?self, "advance_after_teardown: expected SetupRan, ignoring");
            return;
        }
        *self = Self::TeardownRan;
    }
}

/// Reference-counted busy indicator for tracking concurrent async operations.
///
/// Callers increment with [`Self::set_busy`] before starting an operation
/// and decrement with [`Self::busy_complete`] when it finishes.
/// [`Self::is_busy`] returns true while any operation is in flight.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BusyCounter {
    count: u32,
}

impl BusyCounter {
    /// Increment the busy counter. Represents one more in-flight operation.
    pub fn set_busy(&mut self) {
        self.count = self.count.saturating_add(1);
    }

    /// Decrement the busy counter. Floors at zero with a warning log on underflow.
    pub fn busy_complete(&mut self) {
        if self.count == 0 {
            tracing::warn!("busy_complete called with no outstanding busy tokens");
            return;
        }
        self.count -= 1;
    }

    /// Returns `true` while any operation is in flight (counter > 0).
    pub fn is_busy(&self) -> bool {
        self.count > 0
    }
}

/// Groups runtime-only fields that are specific to the current running instance
/// and have no meaning across restarts (stream indices, queues, in-progress flags).
/// The entire struct is skipped during serialization so individual fields cannot
/// be accidentally excluded from persistence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionCoreEphemeral {
    /// The current session phase (idle, sending, streaming, assembling, compacting).
    pub(crate) phase: SessionPhase,
    /// Index into `history` for the entry currently receiving stream tokens.
    pub(crate) streaming_entry_index: Option<usize>,
    /// Turn dispatch queue — drives all turn transitions through a single processor.
    pub(crate) message_queue: crate::feat::session::turn_queue::TurnQueue,
    /// Maps stream tool call index to history index for in-progress tool calls.
    pub(crate) streaming_tool_call_indices: HashMap<usize, usize>,
    /// Index into `history` for the entry currently receiving thinking tokens.
    pub(crate) streaming_thinking_entry_index: Option<usize>,
    /// Cached context size in tokens (assembled prompt size).
    /// Updated when context is assembled. Not persisted across restarts.
    /// OWNER: session-actor.
    pub(crate) cached_context_size: Option<u32>,
    /// Maps tool_call_id to history index for pending streaming ToolResult entries.
    /// OWNER: session-actor.
    pub(crate) streaming_tool_result_indices: HashMap<String, usize>,
    /// Set to true to request graceful turn termination at the next pause point.
    /// Checked at `on_tool_batch_completed` and `on_stream_completed`.
    /// OWNER: session-actor.
    pub(crate) soft_cancel_requested: bool,
    /// Indices of entries marked as ignored during compaction.
    /// Used to un-ignore on cancel. Empty when not compacting.
    /// Not persisted — compaction is ephemeral.
    #[serde(skip)]
    pub(crate) compaction_gathered_indices: Vec<usize>,
    /// Reference-counted busy counter for lifecycle scripts and other async operations.
    /// Rendered as the animated "Working..." spinner when non-zero.
    #[serde(default)]
    pub(crate) busy_counter: BusyCounter,
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
    pub(crate) history: ChatHistory,
    /// Per-session model and strategy selection.
    /// OWNER: provider-actor (model), context-actor (strategy via SwitchPromptStrategy command)
    pub(crate) profile: SessionProfile,
    /// Working directory for tool execution in this session.
    /// OWNER: IntentHandler (set on session creation and cd commands)
    #[serde(default = "default_cwd")]
    pub(crate) cwd: std::path::PathBuf,
    /// Token usage ledger — one immutable record per request/response pair.
    /// OWNER: session-actor (records tokens on assembly and StreamCompleted).
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
    /// Whether this session is loaded in memory or archived in the database.
    /// OWNER: session-actor (transitions on close/archive/unarchive).
    #[serde(default)]
    pub(crate) session_state: SessionState,
    /// Lifecycle script progression — one-way: NothingRan → SetupRan → TeardownRan.
    /// OWNER: session-actor (advances only after script success).
    #[serde(default)]
    pub(crate) lifecycle_script_state: LifecycleScriptState,
    /// Whether this session was created by a workflow node.
    /// OWNER: workflow-actor (set on creation).
    #[serde(default)]
    pub(crate) is_workflow: bool,
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
            history: ChatHistory::new(),
            profile: SessionProfile::default(),
            cwd: std::path::PathBuf::from("."),
            token_ledger: Vec::new(),
            parent_session: None,
            strategy_state: HashMap::new(),
            blobs: HashMap::new(),
            lifecycle_name: None,
            lifecycle_args: Vec::new(),
            session_state: SessionState::Loaded,
            lifecycle_script_state: LifecycleScriptState::NothingRan,
            is_workflow: false,
            ephemeral: SessionCoreEphemeral::default(),
        }
    }
}

/// Snapshot of chat log scroll position captured before entering the Pins section.
///
/// Used to restore the history viewport when the user navigates away from
/// Pins to another sidebar section (Persona/Sessions). Discarded without
/// restoring when the user leaves the sidebar entirely to Normal scope,
/// indicating they wanted to view the pinned entry in the history.
#[derive(Debug, Clone, Default)]
pub(crate) struct SavedHistoryPosition {
    /// The scroll offset at the time of capture.
    pub(crate) scroll_offset: Option<u16>,
    /// The selected entry index at the time of capture.
    pub(crate) selected_entry_index: Option<usize>,
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
    /// Snapshot of chat log position before entering Pins sidebar section.
    ///
    /// `None` when not in a Pins browsing session. Set when the cursor enters
    /// Pins, restored when the cursor leaves to another section, discarded
    /// when leaving the sidebar to Normal.
    pub(crate) saved_history_position: Option<SavedHistoryPosition>,
    /// Entry IDs whose ignored blocks are currently *shown* (expanded).
    ///
    /// Default: empty (all ignored blocks are collapsed).
    /// Key: the ID of the first entry in the contiguous ignored block.
    /// Ephemeral — not persisted across restarts.
    pub(crate) shown_ignored_blocks: HashSet<ChatEntryId>,
    /// The visual items list computed from flat history during render.
    ///
    /// Maps visual-item positions to either real entries or collapsed
    /// ignored blocks. Set by the renderer each frame, read by intent
    /// handlers for navigation and toggle.
    pub(crate) visual_items: RwLock<Vec<crate::feat::ui::chat_log::visual_item::VisualItem>>,
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
            saved_history_position: self.saved_history_position.clone(),
            shown_ignored_blocks: self.shown_ignored_blocks.clone(),
            visual_items: RwLock::new(
                self.visual_items
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            ),
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
            saved_history_position: None,
            shown_ignored_blocks: HashSet::new(),
            visual_items: RwLock::new(Vec::new()),
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
                    crate::feat::session::profile::DEFAULT_TOKEN_BUDGET,
                    crate::feat::session::profile::DEFAULT_SLIDING_WINDOW_SIZE,
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

    /// Mark entries at the given indices as ignored.
    ///
    /// Used by the compaction actor to mark entries that have been summarized.
    /// Ignores any indices that are out of bounds.
    pub fn mark_entries_ignored(&mut self, indices: &[usize]) {
        for &i in indices {
            if i < self.core.history.len() {
                self.core.history[i].ignored = true;
            }
        }
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
    /// Push a chat entry onto the history.
    ///
    /// Future work: restrict to the session feature module and require external
    /// code to use the `PushChatEntry` command (which also triggers persistence).
    #[allow(clippy::missing_panics_doc)]
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

    /// Insert an entry at a specific position in the history.
    ///
    /// Used by compaction to place the `Compaction` entry at the boundary
    /// between compacted and non-compacted entries, maintaining logical
    /// vec order.
    ///
    /// Adjusts ephemeral tracking indices (streaming, tool results) that
    /// reference positions >= the insertion point.
    ///
    /// Returns the index where the entry was inserted.
    pub fn insert_entry_at(&mut self, index: usize, entry: ChatEntry) -> usize {
        let clamped = index.min(self.core.history.len());
        self.core.history.insert(clamped, entry);
        // Shift tracking indices that point to entries at or after the insertion point.
        if let Some(ref mut i) = self.core.ephemeral.streaming_entry_index
            && *i >= clamped
        {
            *i += 1;
        }
        if let Some(ref mut i) = self.core.ephemeral.streaming_thinking_entry_index
            && *i >= clamped
        {
            *i += 1;
        }
        for i in self
            .core
            .ephemeral
            .streaming_tool_result_indices
            .values_mut()
        {
            if *i >= clamped {
                *i += 1;
            }
        }
        for key in self
            .core
            .ephemeral
            .streaming_tool_call_indices
            .keys()
            .copied()
            .collect::<Vec<_>>()
        {
            if let Some(v) = self
                .core
                .ephemeral
                .streaming_tool_call_indices
                .get_mut(&key)
                && *v >= clamped
            {
                *v += 1;
            }
        }
        clamped
    }

    /// Lazily create the Assistant entry for the current stream.
    ///
    /// Called on first `append_stream_token`, `finish_streaming`,
    /// `begin_tool_call`, or `cancel_streaming`. No-op if the entry
    /// already exists or the session is not streaming.
    fn ensure_assistant_entry(&mut self) {
        if self.core.ephemeral.streaming_entry_index.is_some()
            || !matches!(self.core.ephemeral.phase, SessionPhase::Streaming)
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
    /// Soft guard: if the session is not in Sending or Idle phase, logs a warning
    /// and returns without changing state.
    pub fn begin_streaming(&mut self) {
        if !matches!(
            self.core.ephemeral.phase,
            SessionPhase::Sending | SessionPhase::Idle
        ) {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.phase,
                "begin_streaming called while not in Sending or Idle phase — ignoring"
            );
            return;
        }
        self.core.ephemeral.phase = SessionPhase::Streaming;
    }

    /// Append a token to the streaming assistant entry.
    ///
    /// Lazily creates the Assistant entry if this is the first token.
    ///
    /// Soft guard: if the session is not streaming, logs a warning and returns an error.
    ///
    /// # Errors
    ///
    /// Returns `Err(StreamingError::NoStreamingEntry)` if the session is not in Streaming phase.
    #[expect(
        clippy::indexing_slicing,
        reason = "index comes from push_entry which always returns a valid index"
    )]
    pub fn append_stream_token<S>(&mut self, token: S) -> Result<(), StreamingError>
    where
        S: AsRef<str>,
    {
        if !matches!(self.core.ephemeral.phase, SessionPhase::Streaming) {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.phase,
                "append_stream_token called while not streaming — ignoring"
            );
            return Err(StreamingError::NoStreamingEntry);
        }
        self.ensure_assistant_entry();
        let index = self
            .core
            .ephemeral
            .streaming_entry_index
            .ok_or(StreamingError::NoStreamingEntry)?;
        if let ChatEntry {
            kind: ChatEntryKind::Assistant(ref mut text),
            ..
        } = self.core.history[index]
        {
            text.push_str(token.as_ref());
            Ok(())
        } else {
            Err(StreamingError::NotAssistantEntry)
        }
    }

    /// Begin accumulating thinking tokens.
    ///
    /// Appends an empty `Thinking` entry to the history. The Assistant entry
    /// is created lazily later (on first `append_stream_token` or `finish_streaming`),
    /// so entries naturally appear in order: thinking before assistant.
    ///
    /// Soft guard: if the session is not streaming or thinking has already begun,
    /// logs a warning and returns without changing state.
    pub fn begin_thinking(&mut self) {
        if !matches!(self.core.ephemeral.phase, SessionPhase::Streaming) {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.phase,
                "begin_thinking called while not streaming — ignoring"
            );
            return;
        }
        if self.core.ephemeral.streaming_thinking_entry_index.is_some() {
            tracing::warn!("begin_thinking called while already thinking — ignoring");
            return;
        }
        let entry = ChatEntry::thinking("");
        let index = self.push_entry(entry);
        self.core.ephemeral.streaming_thinking_entry_index = Some(index);
    }

    /// Append a thinking token to the streaming Thinking entry.
    ///
    /// # Panics
    ///
    /// Panics if `begin_thinking()` has not been called.
    ///
    /// # Errors
    ///
    /// Returns a [`StreamingError`] if the session is not in a valid streaming state.
    #[expect(clippy::indexing_slicing, reason = "index set by begin_thinking")]
    pub fn append_thinking_token<S>(&mut self, token: S) -> Result<(), StreamingError>
    where
        S: AsRef<str>,
    {
        let index = self
            .core
            .ephemeral
            .streaming_thinking_entry_index
            .ok_or(StreamingError::NoThinkingEntry)?;
        if let ChatEntry {
            kind: ChatEntryKind::Thinking(ref mut text),
            ..
        } = self.core.history[index]
        {
            text.push_str(token.as_ref());
        }
        Ok(())
    }

    /// The index of the streaming thinking entry, if thinking is being accumulated.
    pub fn streaming_thinking_entry_index(&self) -> Option<usize> {
        self.core.ephemeral.streaming_thinking_entry_index
    }

    /// Mark streaming as finished (normal completion).
    ///
    /// Creates an empty Assistant entry if no tokens were ever appended
    /// (e.g., a stream that ended immediately), unless `preserve_assistant`
    /// is `false`. When `false`, the method skips `ensure_assistant_entry()`
    /// so that error/cancel entries remain the last entry in history.
    pub fn finish_streaming(&mut self, preserve_assistant: bool) {
        if preserve_assistant {
            self.ensure_assistant_entry();
        }
        self.core.ephemeral.phase = SessionPhase::Idle;
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
        self.core.ephemeral.phase = SessionPhase::Idle;
        self.core.ephemeral.streaming_entry_index = None;
        self.core.ephemeral.streaming_tool_call_indices.clear();
        self.core.ephemeral.streaming_thinking_entry_index = None;
        self.core.ephemeral.streaming_tool_result_indices.clear();
    }

    /// Cancel streaming and drain queued messages back to the input buffer.
    ///
    /// Used when the user interrupts or switches to Normal mode during an
    /// active stream. The display text from drained `UserMessage` entries is
    /// joined with newlines and replaces whatever was in the input box.
    /// `ToolContinuation` and `CompactionNeeded` items are silently discarded.
    pub fn cancel_stream_and_drain(&mut self) {
        self.cancel_streaming();
        let drained = self.drain_queue();
        let display_texts: Vec<&str> = drained
            .iter()
            .filter_map(|item| match item {
                crate::feat::session::queue_item::QueueItem::UserMessage(entry) => {
                    match &entry.kind {
                        ChatEntryKind::User { display, .. } => Some(display.as_str()),
                        _ => None,
                    }
                }
                _ => None,
            })
            .collect();
        let drained_text = display_texts.join("\n");
        if !drained_text.is_empty() {
            self.chat_input_mut().replace_all(drained_text);
        }
    }

    /// Returns the current session lifecycle phase.
    pub fn phase(&self) -> SessionPhase {
        self.core.ephemeral.phase
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
    /// Returns an error if no tool call entry is tracked for the given stream index.
    ///
    /// # Errors
    ///
    /// Returns a [`StreamingError`] if the streaming state is invalid or the index is out of bounds.
    #[expect(
        clippy::indexing_slicing,
        reason = "index comes from push_entry which always returns a valid index"
    )]
    pub fn append_tool_call_delta(
        &mut self,
        index: usize,
        partial_json: &str,
    ) -> Result<(), StreamingError> {
        let history_index = self
            .core
            .ephemeral
            .streaming_tool_call_indices
            .get(&index)
            .copied()
            .ok_or(StreamingError::NoToolCallIndex { index })?;
        if let ChatEntryKind::ToolCall {
            ref mut arguments, ..
        } = self.core.history[history_index].kind
        {
            arguments.push_str(partial_json);
        }
        Ok(())
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
            ref mut content, ..
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
    ///
    /// Accepts optional truncation metadata and full content from the tool
    /// execution result. When truncation is present, stores both the truncated
    /// content and the original untruncated output.
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
        full_content: Option<String>,
        truncation: Option<nullslop_provider::tool_types::TruncationMeta>,
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
                    full_content: entry_full_content,
                    truncation: entry_truncation,
                    ..
                } => {
                    content.clone_into(entry_content);
                    *entry_status = status;
                    *entry_full_content = full_content;
                    *entry_truncation = truncation;
                }
                _ => {}
            }
        } else if let Some(meta) = truncation {
            // Non-streaming tool with truncation — push a truncated entry.
            let full = full_content.unwrap_or_default();
            self.push_entry(ChatEntry::tool_result_truncated(
                tool_call_id,
                name,
                content.to_owned(),
                full,
                status,
                meta,
            ));
        } else {
            // Non-streaming tool — push a new completed entry.
            self.push_entry(ChatEntry::tool_result(tool_call_id, name, content, status));
        }
    }

    // --- Queue ---

    /// Read-only access to the turn dispatch queue items.
    pub fn queue(
        &self,
    ) -> &std::collections::VecDeque<crate::feat::session::queue_item::QueueItem> {
        self.core.ephemeral.message_queue.items()
    }

    /// Number of items waiting in the queue.
    pub fn queue_len(&self) -> usize {
        self.core.ephemeral.message_queue.len()
    }

    /// Push an item onto the back of the queue.
    pub fn enqueue(&mut self, item: crate::feat::session::queue_item::QueueItem) {
        self.core.ephemeral.message_queue.enqueue(item);
    }

    /// Push an item onto the front of the queue (for priority items like `CompactionNeeded`).
    pub fn enqueue_front(&mut self, item: crate::feat::session::queue_item::QueueItem) {
        self.core.ephemeral.message_queue.enqueue_front(item);
    }

    /// Pop the front item from the queue, if any.
    pub(in crate::feat) fn dequeue(
        &mut self,
    ) -> Option<crate::feat::session::queue_item::QueueItem> {
        self.core.ephemeral.message_queue.pop()
    }

    /// Drain all queued items, returning them in order.
    pub(in crate::feat) fn drain_queue(
        &mut self,
    ) -> std::collections::VecDeque<crate::feat::session::queue_item::QueueItem> {
        self.core.ephemeral.message_queue.drain()
    }

    // --- Assembling ---

    /// Mark the session as having a prompt assembly in progress.
    ///
    /// Soft guard: if not idle, logs a warning and returns without changing phase.
    pub fn begin_assembling(&mut self) {
        if !matches!(self.core.ephemeral.phase, SessionPhase::Idle) {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.phase,
                "begin_assembling called while not idle — ignoring"
            );
            return;
        }
        self.core.ephemeral.phase = SessionPhase::Assembling;
    }

    /// Clear the assembling flag (called when prompt assembly completes).
    ///
    /// Soft guard: if not assembling, logs a warning and returns without changing phase.
    pub fn finish_assembling(&mut self) {
        if !matches!(self.core.ephemeral.phase, SessionPhase::Assembling) {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.phase,
                "finish_assembling called while not assembling — ignoring"
            );
            return;
        }
        self.core.ephemeral.phase = SessionPhase::Idle;
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
    /// Soft guard: if not idle, logs a warning and returns without changing phase.
    pub fn begin_sending(&mut self) {
        if !matches!(self.core.ephemeral.phase, SessionPhase::Idle) {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.phase,
                "begin_sending called while not idle — ignoring"
            );
            return;
        }
        self.core.ephemeral.phase = SessionPhase::Sending;
    }

    /// Clear the sending flag (called when the first stream token arrives).
    ///
    /// Soft guard: if not sending, logs a warning and returns without changing phase.
    pub fn finish_sending(&mut self) {
        if !matches!(self.core.ephemeral.phase, SessionPhase::Sending) {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.phase,
                "finish_sending called while not sending — ignoring"
            );
            return;
        }
        self.core.ephemeral.phase = SessionPhase::Idle;
    }

    /// Mark the session as compacting.
    ///
    /// Soft guard: if not idle, logs a warning and returns without changing phase.
    pub fn begin_compacting(&mut self, gathered_indices: Vec<usize>) {
        if !matches!(self.core.ephemeral.phase, SessionPhase::Idle) {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.phase,
                "begin_compacting called while not idle — ignoring"
            );
            return;
        }
        self.core.ephemeral.phase = SessionPhase::Compacting;
        self.core.ephemeral.compaction_gathered_indices = gathered_indices;
    }

    /// Mark compaction as finished.
    ///
    /// Soft guard: if the session is not currently compacting (e.g. compaction was
    /// cancelled by the user while the LLM call was in flight), logs a warning and
    /// returns early instead of panicking.
    pub fn finish_compacting(&mut self) {
        if !matches!(self.core.ephemeral.phase, SessionPhase::Compacting) {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.phase,
                "finish_compacting called while not compacting — ignoring"
            );
            return;
        }
        self.core.ephemeral.phase = SessionPhase::Idle;
    }

    /// Mark the session as running a lifecycle teardown script.
    ///
    /// Soft guard: if not idle, logs a warning and returns without changing phase.
    pub fn begin_tearing_down(&mut self) {
        if !matches!(self.core.ephemeral.phase, SessionPhase::Idle) {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.phase,
                "begin_tearing_down called while not idle — ignoring"
            );
            return;
        }
        self.core.ephemeral.phase = SessionPhase::TearingDown;
    }

    /// Mark teardown as finished, returning to `Idle`.
    ///
    /// Soft guard: if the session is not currently tearing down, logs a warning
    /// and returns without changing phase.
    pub fn finish_tearing_down(&mut self) {
        if !matches!(self.core.ephemeral.phase, SessionPhase::TearingDown) {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.phase,
                "finish_tearing_down called while not tearing down — ignoring"
            );
            return;
        }
        self.core.ephemeral.phase = SessionPhase::Idle;
    }

    /// Cancel an in-progress compaction.
    ///
    /// Sets phase to Idle, un-ignores entries that were marked during
    /// `begin_compacting`, and returns drained queued messages so the
    /// caller can start a new turn if needed.
    ///
    /// No-op if not currently compacting.
    pub fn cancel_compacting(
        &mut self,
    ) -> std::collections::VecDeque<crate::feat::session::queue_item::QueueItem> {
        if !matches!(self.core.ephemeral.phase, SessionPhase::Compacting) {
            return std::collections::VecDeque::new();
        }
        let indices = std::mem::take(&mut self.core.ephemeral.compaction_gathered_indices);
        for i in indices {
            if i < self.core.history.len() {
                self.core.history[i].ignored = false;
            }
        }
        self.core.ephemeral.phase = SessionPhase::Idle;
        self.drain_queue()
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

    /// Scroll the chat log so that the currently selected entry is visible.
    ///
    /// Uses `entry_line_ranges` and `viewport_height` (set by the renderer
    /// each frame) to compute the scroll offset that brings the selected
    /// entry into view. This is essentially the same logic as the renderer's
    /// scroll-to-selected adjustment, but applied as a state mutation for
    /// intent handlers.
    ///
    /// No-op if no entry is selected or if line range data is unavailable.
    pub fn scroll_to_selected(&mut self) {
        let Some(selected_idx) = self.ui.selected_entry_index else {
            return;
        };

        let ranges = match self.ui.entry_line_ranges.read() {
            Ok(guard) => guard.clone(),
            Err(_) => return,
        };

        let Some(&(start, end)) = ranges.get(selected_idx) else {
            return;
        };

        let viewport_height = self.ui.viewport_height.load(Ordering::Relaxed);
        let blank_count = self.ui.blank_count.load(Ordering::Relaxed);
        let max_offset = self.ui.last_max_offset.load(Ordering::Relaxed);

        if viewport_height == 0 {
            return;
        }

        let abs_start = start.saturating_add(blank_count);
        let abs_end = end.saturating_add(blank_count);
        let entry_height = abs_end.saturating_sub(abs_start);

        let current_offset = self.ui.scroll_offset.unwrap_or(max_offset);

        let new_offset = if entry_height <= viewport_height {
            // Entry fits in viewport — adjust only if it's outside.
            if abs_start < current_offset {
                abs_start
            } else if abs_end > current_offset.saturating_add(viewport_height) {
                abs_end.saturating_sub(viewport_height)
            } else {
                // Already visible — no change needed.
                return;
            }
        } else {
            // Entry is taller than viewport — align top.
            if abs_start >= current_offset.saturating_add(viewport_height) {
                abs_start
            } else if abs_end <= current_offset {
                abs_end.saturating_sub(viewport_height)
            } else {
                // Already overlapping — no change needed.
                return;
            }
        };

        let clamped = new_offset.min(max_offset);

        if clamped >= max_offset {
            self.ui.scroll_offset = None;
        } else {
            self.ui.scroll_offset = Some(clamped);
        }
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
    /// Resolves through visual items. Collapsed blocks are selectable.
    /// No-op if no entries are visible.
    pub fn move_cursor_to_first_visible(&mut self) {
        let range = self.visible_entry_range();
        if range.is_empty() {
            return;
        }
        let items = self.visual_items().clone();
        if items.is_empty() {
            // Fallback: use raw history index when visual items not yet computed.
            self.ui.selected_entry_index = Some(range.start);
        } else {
            let mut idx = range.start;
            while idx < range.end {
                let selectable = match items.get(idx) {
                    Some(VisualItem::CollapsedIgnoredBlock { .. }) => true,
                    Some(VisualItem::Entry(hist_idx)) => {
                        !self.core.history[*hist_idx].is_empty_assistant()
                    }
                    None => false,
                };
                if selectable {
                    break;
                }
                idx += 1;
            }
            if idx < items.len() {
                let selectable = match items.get(idx) {
                    Some(VisualItem::CollapsedIgnoredBlock { .. }) => true,
                    Some(VisualItem::Entry(hist_idx)) => {
                        !self.core.history[*hist_idx].is_empty_assistant()
                    }
                    None => false,
                };
                if selectable {
                    self.ui.selected_entry_index = Some(idx);
                }
            }
        }
    }

    /// Move the cursor to the last entry visible in the viewport.
    ///
    /// Resolves through visual items. Collapsed blocks are selectable.
    /// No-op if no entries are visible.
    pub fn move_cursor_to_last_visible(&mut self) {
        let range = self.visible_entry_range();
        if range.is_empty() {
            return;
        }
        let items = self.visual_items().clone();
        if items.is_empty() {
            // Fallback: use raw history index when visual items not yet computed.
            self.ui.selected_entry_index = Some(range.end.saturating_sub(1));
        } else {
            let mut idx = range.end.saturating_sub(1);
            while idx > range.start {
                let selectable = match items.get(idx) {
                    Some(VisualItem::CollapsedIgnoredBlock { .. }) => true,
                    Some(VisualItem::Entry(hist_idx)) => {
                        !self.core.history[*hist_idx].is_empty_assistant()
                    }
                    None => false,
                };
                if selectable {
                    break;
                }
                idx = idx.saturating_sub(1);
            }
            let selectable = match items.get(idx) {
                Some(VisualItem::CollapsedIgnoredBlock { .. }) => true,
                Some(VisualItem::Entry(hist_idx)) => {
                    !self.core.history[*hist_idx].is_empty_assistant()
                }
                None => false,
            };
            if selectable {
                self.ui.selected_entry_index = Some(idx);
            }
        }
    }

    // --- History restoration ---

    /// Restore conversation history from a persisted snapshot.
    ///
    /// Replaces the current history with the given entries. Used by session
    /// persistence to rehydrate a session from disk.
    pub fn restore_history(&mut self, entries: Vec<ChatEntry>) {
        self.core.history.replace_all(entries);
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
    /// If nothing is selected, selects the first visual item.
    /// Walks the visual items list, not the raw history.
    /// Collapsed blocks are selectable (so user can press `h` to expand).
    /// Skips empty assistant entries.
    /// Clamps to the last visual-item index.
    /// No-op if visual items list is empty.
    pub fn select_next_entry(&mut self) {
        let items = self.visual_items().clone();
        if items.is_empty() {
            // Before first render, fall back to direct history walking.
            self.select_next_entry_fallback();
            return;
        }

        let max = items.len() - 1;
        let start = self
            .ui
            .selected_entry_index
            .map_or(0, |i| i.saturating_add(1).min(max));
        let mut idx = start;
        while idx < max {
            let selectable = match items[idx] {
                VisualItem::CollapsedIgnoredBlock { .. } => true,
                VisualItem::Entry(hist_idx) => !self.core.history[hist_idx].is_empty_assistant(),
            };
            if selectable {
                break;
            }
            idx = idx.saturating_add(1);
        }
        let selectable = match items[idx] {
            VisualItem::CollapsedIgnoredBlock { .. } => true,
            VisualItem::Entry(hist_idx) => !self.core.history[hist_idx].is_empty_assistant(),
        };
        if selectable {
            self.ui.selected_entry_index = Some(idx);
        }
    }

    /// Select the previous entry (moving toward older messages).
    ///
    /// If nothing is selected, selects the last visual item.
    /// Walks the visual items list, not the raw history.
    /// Collapsed blocks are selectable (so user can press `h` to expand).
    /// Skips empty assistant entries.
    /// Clamps to 0.
    /// No-op if visual items list is empty.
    pub fn select_prev_entry(&mut self) {
        let items = self.visual_items().clone();
        if items.is_empty() {
            // Before first render, fall back to direct history walking.
            self.select_prev_entry_fallback();
            return;
        }

        let start = self
            .ui
            .selected_entry_index
            .map_or(items.len().saturating_sub(1), |i| i.saturating_sub(1));
        let mut idx = start;
        while idx > 0 {
            let selectable = match items[idx] {
                VisualItem::CollapsedIgnoredBlock { .. } => true,
                VisualItem::Entry(hist_idx) => !self.core.history[hist_idx].is_empty_assistant(),
            };
            if selectable {
                break;
            }
            idx = idx.saturating_sub(1);
        }
        let selectable = match items[idx] {
            VisualItem::CollapsedIgnoredBlock { .. } => true,
            VisualItem::Entry(hist_idx) => !self.core.history[hist_idx].is_empty_assistant(),
        };
        if selectable {
            self.ui.selected_entry_index = Some(idx);
        }
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

    /// Fallback: select next entry by walking raw history.
    /// Used when visual items haven't been computed yet (before first render).
    fn select_next_entry_fallback(&mut self) {
        if self.core.history.is_empty() {
            return;
        }
        let max = self.core.history.len() - 1;
        let start = self
            .ui
            .selected_entry_index
            .map_or(0, |i| i.saturating_add(1).min(max));
        let mut idx = start;
        while idx < max && self.core.history[idx].is_empty_assistant() {
            idx = idx.saturating_add(1);
        }
        if !self.core.history[idx].is_empty_assistant() {
            self.ui.selected_entry_index = Some(idx);
        }
    }

    /// Fallback: select prev entry by walking raw history.
    /// Used when visual items haven't been computed yet (before first render).
    fn select_prev_entry_fallback(&mut self) {
        if self.core.history.is_empty() {
            return;
        }
        let start = self
            .ui
            .selected_entry_index
            .map_or(self.core.history.len().saturating_sub(1), |i| {
                i.saturating_sub(1)
            });
        let mut idx = start;
        while idx > 0 && self.core.history[idx].is_empty_assistant() {
            idx = idx.saturating_sub(1);
        }
        if !self.core.history[idx].is_empty_assistant() {
            self.ui.selected_entry_index = Some(idx);
        }
    }

    // --- Saved history position ---

    /// Saves the current chat log scroll position as the "pre-pin" snapshot.
    ///
    /// Call this before `sync_chat_log_cursor` changes the viewport.
    /// No-op if a position is already saved (prevents overwriting during
    /// a single Pins visit).
    pub(crate) fn save_history_position(&mut self) {
        if self.ui.saved_history_position.is_some() {
            return;
        }
        self.ui.saved_history_position = Some(SavedHistoryPosition {
            scroll_offset: self.ui.scroll_offset,
            selected_entry_index: self.ui.selected_entry_index,
        });
    }

    /// Restores the chat log to the saved "pre-pin" position, if one exists.
    ///
    /// Consumes the saved position (take semantics).
    pub(crate) fn restore_history_position(&mut self) {
        if let Some(saved) = self.ui.saved_history_position.take() {
            self.ui.scroll_offset = saved.scroll_offset;
            self.ui.selected_entry_index = saved.selected_entry_index;
        }
    }

    /// Discards the saved position without restoring.
    ///
    /// Used when leaving the sidebar to Normal scope — the pin's position
    /// should persist in the chat log.
    pub(crate) fn discard_saved_history_position(&mut self) {
        self.ui.saved_history_position = None;
    }

    /// Returns whether there is a saved history position.
    pub(crate) fn has_saved_history_position(&self) -> bool {
        self.ui.saved_history_position.is_some()
    }

    /// The index of the currently selected entry, if any.
    pub fn selected_entry_index(&self) -> Option<usize> {
        self.ui.selected_entry_index
    }

    /// The currently selected entry, if any.
    ///
    /// Resolves through the visual items list: returns `None` if the
    /// selected item is a collapsed block rather than a real entry.
    /// Falls back to direct history indexing when visual items are empty
    /// (before the first render).
    pub fn selected_entry(&self) -> Option<&ChatEntry> {
        let vi_idx = self.ui.selected_entry_index?;
        let items = self.visual_items();
        if items.is_empty() {
            // Before first render, visual items haven't been computed yet.
            // Fall back to direct history indexing.
            return self.core.history.get(vi_idx);
        }
        match items.get(vi_idx)? {
            VisualItem::Entry(hist_idx) => self.core.history.get(*hist_idx),
            VisualItem::CollapsedIgnoredBlock { .. } => None,
        }
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

    // --- Ignored block visibility ---

    /// Toggle visibility of the ignored block containing the given entry.
    ///
    /// Finds the contiguous run of ignored entries containing the entry
    /// identified by `entry_id`, takes the first entry's ID as the block
    /// representative, and toggles it in `shown_ignored_blocks`.
    ///
    /// No-op if the entry is not found or is not ignored.
    pub fn toggle_ignored_block_visibility(&mut self, entry_id: &ChatEntryId) {
        let Some(idx) = self.core.history.iter().position(|e| e.id == *entry_id) else {
            return;
        };
        if !self.core.history[idx].ignored {
            return;
        }
        // Scan backward to find the start of the contiguous ignored block.
        let mut block_start = idx;
        while block_start > 0 && self.core.history[block_start - 1].ignored {
            block_start -= 1;
        }
        let block_representative = self.core.history[block_start].id.clone();
        if self.ui.shown_ignored_blocks.contains(&block_representative) {
            self.ui.shown_ignored_blocks.remove(&block_representative);
        } else {
            self.ui.shown_ignored_blocks.insert(block_representative);
        }
    }

    /// Store the visual items list computed during render.
    pub fn set_visual_items(
        &self,
        items: Vec<crate::feat::ui::chat_log::visual_item::VisualItem>,
    ) {
        if let Ok(mut guard) = self.ui.visual_items.write() {
            *guard = items;
        }
    }

    /// Read-only access to the visual items list.
    pub fn visual_items(
        &self,
    ) -> std::sync::RwLockReadGuard<'_, Vec<crate::feat::ui::chat_log::visual_item::VisualItem>> {
        self.ui.visual_items.read().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The visual item at the currently selected position, if any.
    pub fn selected_visual_item(
        &self,
    ) -> Option<crate::feat::ui::chat_log::visual_item::VisualItem> {
        let idx = self.ui.selected_entry_index?;
        self.ui.visual_items.read().unwrap_or_else(std::sync::PoisonError::into_inner).get(idx).cloned()
    }

    /// Resolve the selected visual-item index to a history index.
    ///
    /// Returns `None` if nothing is selected or the selected item is a
    /// collapsed block (not a real entry).
    pub fn selected_history_index(&self) -> Option<usize> {
        let vi_idx = self.ui.selected_entry_index?;
        let items = self.ui.visual_items.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        if items.is_empty() {
            // Before first render, visual items haven't been computed yet.
            // Fall back: selected_entry_index IS a history index in this case.
            return Some(vi_idx);
        }
        match items.get(vi_idx)? {
            crate::feat::ui::chat_log::visual_item::VisualItem::Entry(hist_idx) => Some(*hist_idx),
            crate::feat::ui::chat_log::visual_item::VisualItem::CollapsedIgnoredBlock { .. } => None,
        }
    }

    /// Returns `true` if the given entry is a `ToolCall` that is still
    /// actively streaming arguments from the LLM.
    pub fn is_tool_call_streaming(&self, entry_id: &ChatEntryId) -> bool {
        let Some(idx) = self.core.history.iter().position(|e| e.id == *entry_id) else {
            return false;
        };
        self.core
            .ephemeral
            .streaming_tool_call_indices
            .values()
            .any(|&v| v == idx)
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

    /// Update the last token record's received count and cost.
    ///
    /// Called when `StreamCompleted` arrives to finalize the pending record.
    ///
    /// # Errors
    ///
    /// Returns a [`StreamingError`] if the ledger is empty.
    pub fn finalize_last_token_record(
        &mut self,
        tokens_received: u32,
        cost: Option<f64>,
    ) -> Result<(), StreamingError> {
        let last = self
            .core
            .token_ledger
            .last_mut()
            .ok_or(StreamingError::EmptyLedger)?;
        last.tokens_received = tokens_received;
        last.cost = cost;
        Ok(())
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

    /// Returns the session's memory state.
    pub fn session_state(&self) -> SessionState {
        self.core.session_state
    }

    /// Sets the session's memory state.
    pub fn set_session_state(&mut self, state: SessionState) {
        self.core.session_state = state;
    }

    /// Returns the lifecycle script state.
    pub fn lifecycle_script_state(&self) -> LifecycleScriptState {
        self.core.lifecycle_script_state
    }

    /// Advances lifecycle state after successful setup: `NothingRan → SetupRan`.
    pub fn advance_lifecycle_after_setup(&mut self) {
        self.core.lifecycle_script_state.advance_after_setup();
    }

    /// Advances lifecycle state after successful teardown: `SetupRan → TeardownRan`.
    pub fn advance_lifecycle_after_teardown(&mut self) {
        self.core.lifecycle_script_state.advance_after_teardown();
    }

    /// Whether the session has any in-flight async operations that should
    /// show the "Working..." spinner.
    pub fn is_busy(&self) -> bool {
        self.core.ephemeral.busy_counter.is_busy()
    }

    /// Mark the session as busy (increment the counter).
    pub fn mark_busy(&mut self) {
        self.core.ephemeral.busy_counter.set_busy();
    }

    /// Mark one busy operation as complete (decrement the counter).
    pub fn mark_busy_complete(&mut self) {
        self.core.ephemeral.busy_counter.busy_complete();
    }

    /// Request graceful turn termination at the next pause point.
    ///
    /// The session actor checks this flag at `on_tool_batch_completed` and
    /// `on_stream_completed`. When set, the turn ends (\u2192 Idle) instead of
    /// continuing, allowing auto-compaction to trigger mid-turn.
    pub fn request_soft_cancel(&mut self) {
        self.core.ephemeral.soft_cancel_requested = true;
    }

    /// Take the soft cancel flag, clearing it.
    ///
    /// Returns `true` if a soft cancel was requested, and clears the flag.
    /// Returns `false` if no cancel was requested.
    pub fn take_soft_cancel(&mut self) -> bool {
        std::mem::take(&mut self.core.ephemeral.soft_cancel_requested)
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
