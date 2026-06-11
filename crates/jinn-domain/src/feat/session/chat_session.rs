//! Chat session protocol - state types for a single conversation.
//!
//! [`ChatSessionState`] owns the history and streaming state for one chat session.
//! Multiple sessions can exist concurrently in the application, each identified
//! by a [`SessionId`](crate::protocol::SessionId).
//!
//! Fields are grouped into [`SessionCore`] (session-actor / context-actor)
//! and [`SessionUi`] (IntentHandler) sub-structs to make cross-boundary
//! writes visually obvious during code review.

use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::atomic::{AtomicU16, Ordering};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::feat::chat_input::ChatInputBoxState;

use crate::feat::session::chat_history::ChatHistory;
use crate::feat::session::model_selection::ModelSelection;
use crate::feat::session::phase_machine::PhaseKind;
use crate::feat::session::profile::SessionProfile;
use crate::feat::session::steering_buffer::SteeringBuffer;
use crate::feat::session::token_stats::TokenRecord;
use crate::feat::ui::chat_log::visual_item::VisualItem;
use crate::protocol::{
    ChangeSource, ChatEntry, ChatEntryId, ChatEntryKind, ContextOverride, PinPosition, SessionId,
};

use crate::feat::session::entry_timing::EntryTiming;

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

/// Groups runtime-only fields that are specific to the current running instance
/// and have no meaning across restarts (stream indices, queues, in-progress flags).
/// The entire struct is skipped during serialization so individual fields cannot
/// be accidentally excluded from persistence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionCoreEphemeral {
    /// Validated phase transition machine.
    /// The single source of truth for phase state.
    pub machine: crate::feat::session::phase_machine::SessionPhaseMachine,
    /// Turn dispatch queue - drives all turn transitions through a single processor.
    pub message_queue: crate::feat::session::turn_queue::TurnQueue,
    /// Cached context size in tokens (assembled prompt size).
    /// Updated when context is assembled. Not persisted across restarts.
    /// OWNER: session-actor.
    pub cached_context_size: Option<u32>,

    /// Number of active background operations (e.g., lifecycle tasks).
    /// Ephemeral: not persisted, not serialized.
    /// OWNER: session-actor.
    #[serde(skip)]
    pub busy_count: usize,

    /// Pending history mutation batches from background workers.
    /// Drained and applied at safe application points (tool batch completion,
    /// stream completion). Not persisted across restarts.
    #[serde(skip)]
    pub pending_mutations: Vec<Vec<crate::feat::session::history_mutation::HistoryMutation>>,

    /// Discovered resources for THIS session, scoped to its cwd tree.
    /// Populated by the scan actors (skills / prompts / context-files).
    /// Ephemeral: not persisted, re-scanned from disk on session load.
    /// OWNER: scan actors (SkillsScanActor / PromptScanActor / context-files actor).
    /// See `.plans/project-locals/plan.md` decision D3 — per-session isolation.
    #[serde(skip)]
    pub discovered_skills: Vec<crate::feat::skills::Skill>,

    /// Discovered prompt templates for this session (merged global + project).
    /// OWNER: PromptScanActor.
    #[serde(skip)]
    pub discovered_prompt_templates: crate::feat::context::prompt_template::PromptTemplateStore,

    /// Discovered AGENTS.md/CLAUDE.md context files for this session, ordered
    /// root-first (root ancestor first, cwd last) for prompt assembly.
    /// OWNER: context-files scan actor.
    #[serde(skip)]
    pub discovered_context_files: Vec<crate::feat::context::env_context::ContextFile>,
}

// Core session state - owned by session-actor and context-actor.
//
// IntentHandler is exempt and may read/write any field.
// No other actor should mutate these fields.
//
// Fields without `#[serde(skip)]` are persisted across restarts.
// All ephemeral (non-persisted) state lives in [`SessionCoreEphemeral`].

/// Serde default for the `cwd` field - resolves to the current directory.
fn default_cwd() -> std::path::PathBuf {
    std::path::PathBuf::from(".")
}

/// Serde default for [`SessionCore::persist`] — sessions persist unless explicitly marked transient.
pub fn default_persist() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCore {
    /// Unique identifier for this session.
    /// Generated at construction. Matches the HashMap key in `SessionState.sessions`.
    pub session_id: SessionId,
    /// Human-readable title. `None` until the first user message is sent.
    /// OWNER: session-actor (set on first user message, changeable by user).
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// When this session was last updated. Set at construction, updated on save.
    pub updated_at: Timestamp,
    /// When this session was created. Set once at construction, never mutated.
    pub created_at: Timestamp,
    /// All messages in this conversation.
    /// OWNER: session-actor (creates/removes entries, restores history)
    pub history: ChatHistory,
    /// Per-session model and strategy selection.
    /// OWNER: provider-actor (model), context-actor (strategy via SwitchPromptStrategy command)
    pub profile: SessionProfile,
    /// Working directory for tool execution in this session.
    /// OWNER: IntentHandler (set on session creation and cd commands)
    #[serde(default = "default_cwd")]
    pub cwd: std::path::PathBuf,
    /// Token usage ledger - one immutable record per request/response pair.
    /// OWNER: session-actor (records tokens on assembly and StreamCompleted).
    #[serde(default)]
    pub token_ledger: Vec<TokenRecord>,
    /// Parent session ID, if this session was forked from another.
    /// `None` means this is a root session.
    /// OWNER: session-actor (set at session creation).
    #[serde(default)]
    pub parent_session: Option<SessionId>,

    /// Generic blob storage for future subsystems.
    #[serde(default)]
    pub blobs: HashMap<String, JsonValue>,
    /// Name of the session lifecycle that created this session.
    /// `None` means the implicit "blank" lifecycle (no setup command).
    /// OWNER: IntentHandler (set on session creation).
    #[serde(default)]
    pub lifecycle_name: Option<String>,
    /// Arguments passed to the lifecycle setup command.
    /// Replayed during teardown so the same args are available.
    /// OWNER: IntentHandler (set on session creation).
    #[serde(default)]
    pub lifecycle_args: Vec<String>,
    /// Whether this session is loaded in memory or archived in the database.
    /// OWNER: session-actor (transitions on close/archive/unarchive).
    #[serde(default)]
    pub session_state: SessionState,
    /// Lifecycle script progression - one-way: NothingRan → SetupRan → TeardownRan.
    /// OWNER: session-actor (advances only after script success).
    #[serde(default)]
    pub lifecycle_script_state: LifecycleScriptState,
    /// Whether this session is program-initiated (plugin, subagent, judge, etc.),
    /// not a normal user conversation. When true, the session uses its
    /// `assembly_overrides` instead of global defaults and is hidden from the sidebar.
    /// OWNER: session-actor / plugin-dispatch-actor (set on creation).
    #[serde(default)]
    pub is_automated: bool,

    /// Whether this session should be persisted to disk. Default true; set
    /// false for transient automated sessions (e.g. plugin enrichment one-shots).
    /// OWNER: plugin-dispatch-actor (set on creation).
    #[serde(default = "default_persist")]
    pub persist: bool,

    /// Whether the user has meaningfully interacted with this session.
    /// Sessions with `has_interacted = false` are not persisted to disk.
    /// OWNER: session-actor (set via MarkSessionInteracted command).
    #[serde(default)]
    pub has_interacted: bool,
    /// Assembly overrides for automated sessions. When set, these replace global
    /// defaults in `assemble_prompt`. Runtime-only - not persisted.
    /// OWNER: session-actor / plugin-dispatch-actor (set before first message).
    #[serde(skip)]
    pub assembly_overrides: Option<crate::feat::context::assemble::AssemblyOverrides>,
    /// Phased task list for agent session planning.
    /// OWNER: tools-actor (mutated by task list tools).
    #[serde(default)]
    pub task_list: crate::feat::todo_list::TaskList,
    /// Attached plugins - persistent per-session plugin attachments.
    /// OWNER: plugin-dispatch-actor (attach/detach/toggle).
    #[serde(default)]
    pub attached_plugins: Vec<crate::feat::attached_plugin::AttachedPlugin>,
    /// Runtime-only state - not persisted across restarts.
    #[serde(skip)]
    pub ephemeral: SessionCoreEphemeral,
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

            blobs: HashMap::new(),
            lifecycle_name: None,
            lifecycle_args: Vec::new(),
            session_state: SessionState::Loaded,
            lifecycle_script_state: LifecycleScriptState::NothingRan,
            is_automated: false,
            persist: true,

            assembly_overrides: None,
            task_list: crate::feat::todo_list::TaskList::default(),
            attached_plugins: Vec::new(),
            has_interacted: false,
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
pub struct SavedHistoryPosition {
    /// The scroll offset at the time of capture.
    pub scroll_offset: Option<u16>,
    /// The entry ID of the cursor at the time of capture.
    pub selected_cursor_id: Option<ChatEntryId>,
}

/// UI state for a session - owned by IntentHandler (exempt from ownership restrictions).
///
/// These fields control visual presentation: scroll position, selection, input text.
#[derive(Debug)]
pub struct SessionUi {
    /// The user's in-progress message for this session.
    pub chat_input: ChatInputBoxState,
    /// In-memory steering buffer for this session.
    ///
    /// Accumulates user-submitted text fragments that will be drained
    /// into a single `User` chat entry at the next prompt-assembly
    /// boundary. Not serialized - `SessionUi` itself is in-memory only,
    /// so this field is dropped on session close.
    pub steering_buffer: SteeringBuffer,
    /// Number of lines to skip from the top when rendering (ratatui scroll offset).
    ///
    /// `None` means "show the bottom of the conversation" (auto-scroll).
    /// `Some(n)` means the user has manually scrolled to offset `n`.
    pub scroll_offset: Option<u16>,
    /// The entry ID of the currently selected cursor position, if any.
    ///
    /// This is the source of truth for selection. The visual-item index
    /// is resolved on demand via `selected_entry_index()`.
    /// `None` means no entry is selected.
    pub selected_cursor_id: Option<ChatEntryId>,
    /// The maximum scroll offset computed during the last render.
    ///
    /// Used by scroll handlers to resolve the "at bottom" sentinel into
    /// a concrete offset so `scroll_up` / `scroll_down` work correctly.
    /// Uses `AtomicU16` for interior mutability since the element receives `&self`.
    pub last_max_offset: AtomicU16,
    /// The actual viewport scroll offset after clamping and scroll-to-selected
    /// adjustment, as computed by the render pipeline.
    ///
    /// Unlike `scroll_offset` (the user's intent), this reflects what's
    /// actually displayed. Written by the renderer each frame, read by
    /// intent handlers to determine visible entries.
    pub rendered_scroll_offset: AtomicU16,
    /// Per-entry wrapped line ranges computed by the renderer each frame.
    ///
    /// `entry_line_ranges[i] = (start_wrapped_line, end_wrapped_line)` in wrapped
    /// coordinate space. Used by intent handlers to determine which entries are
    /// visible in the viewport.
    pub entry_line_ranges: RwLock<Vec<(u16, u16)>>,
    /// The viewport height (render area height) set by the renderer each frame.
    pub viewport_height: AtomicU16,
    /// Number of blank lines prepended by the renderer for bottom-alignment.
    pub blank_count: AtomicU16,
    /// The set of chat entry IDs whose tool result content is expanded.
    ///
    /// When a tool result entry is expanded, its full content is shown
    /// instead of being truncated. This is ephemeral UI state - not persisted.
    pub expanded_entries: HashSet<ChatEntryId>,
    /// Snapshot of chat log position before entering Pins sidebar section.
    ///
    /// `None` when not in a Pins browsing session. Set when the cursor enters
    /// Pins, restored when the cursor leaves to another section, discarded
    /// when leaving the sidebar to Normal.
    pub saved_history_position: Option<SavedHistoryPosition>,
    /// Entry IDs whose ignored blocks are currently *shown* (expanded).
    ///
    /// Default: empty (all ignored blocks are collapsed).
    /// Key: the ID of the first entry in the contiguous ignored block.
    /// Ephemeral - not persisted across restarts.
    pub shown_ignored_blocks: HashSet<ChatEntryId>,
    /// The visual items list computed from flat history during render.
    ///
    /// Maps visual-item positions to either real entries or collapsed
    /// ignored blocks. Set by the renderer each frame, read by intent
    /// handlers for navigation and toggle.
    pub visual_items: RwLock<Vec<crate::feat::ui::chat_log::visual_item::VisualItem>>,
    /// Tracks an active "x-sweep": holding `x` to apply a fixed ignore state
    /// across consecutive entries.
    ///
    /// `Some((instant, override))` means a sweep is active:
    /// - `instant`: timestamp of the last `x` press in this sweep
    /// - `override`: the `ContextOverride` to apply to subsequent entries
    ///
    /// Cleared by: >100ms gap, or any non-`ChatEntryIgnoreSelected` intent.
    pub ignore_sweep: Option<(
        std::time::Instant,
        crate::feat::session::chat_entry::ContextOverride,
    )>,
}

impl Clone for SessionUi {
    fn clone(&self) -> Self {
        Self {
            chat_input: self.chat_input.clone(),
            steering_buffer: self.steering_buffer.clone(),
            scroll_offset: self.scroll_offset,
            selected_cursor_id: self.selected_cursor_id.clone(),
            last_max_offset: AtomicU16::new(self.last_max_offset.load(Ordering::Relaxed)),
            rendered_scroll_offset: AtomicU16::new(
                self.rendered_scroll_offset.load(Ordering::Relaxed),
            ),
            entry_line_ranges: RwLock::new(self.entry_line_ranges.read().clone()),
            viewport_height: AtomicU16::new(self.viewport_height.load(Ordering::Relaxed)),
            blank_count: AtomicU16::new(self.blank_count.load(Ordering::Relaxed)),
            expanded_entries: self.expanded_entries.clone(),
            saved_history_position: self.saved_history_position.clone(),
            shown_ignored_blocks: self.shown_ignored_blocks.clone(),
            visual_items: RwLock::new(self.visual_items.read().clone()),
            ignore_sweep: self.ignore_sweep,
        }
    }
}

impl Default for SessionUi {
    fn default() -> Self {
        Self {
            chat_input: ChatInputBoxState::new(),
            steering_buffer: SteeringBuffer::default(),
            scroll_offset: None,
            selected_cursor_id: None,
            last_max_offset: AtomicU16::new(0),
            rendered_scroll_offset: AtomicU16::new(0),
            entry_line_ranges: RwLock::new(Vec::new()),
            viewport_height: AtomicU16::new(0),
            blank_count: AtomicU16::new(0),
            expanded_entries: HashSet::new(),
            saved_history_position: None,
            shown_ignored_blocks: HashSet::new(),
            visual_items: RwLock::new(Vec::new()),
            ignore_sweep: None,
        }
    }
}

/// The state of a single chat session.
///
/// Owns the conversation history and tracks whether an LLM response is
/// currently streaming in. The streaming entry is an in-progress `Assistant`
/// entry at a known index - tokens are appended to it until the stream
/// completes or is cancelled.
///
/// Fields are grouped into [`SessionCore`] (session-actor / context-actor)
/// and [`SessionUi`] (IntentHandler) sub-structs to make cross-boundary
/// writes visually obvious during code review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSessionState {
    /// Core domain state managed by session-actor and context-actor.
    #[serde(flatten)]
    pub core: SessionCore,
    /// UI state managed by IntentHandler.
    #[serde(skip)]
    pub ui: SessionUi,
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
    /// Immutable access to this session's steering buffer.
    pub fn steering_buffer(&self) -> &SteeringBuffer {
        &self.ui.steering_buffer
    }
    /// Mutable access to this session's steering buffer.
    pub fn steering_buffer_mut(&mut self) -> &mut SteeringBuffer {
        &mut self.ui.steering_buffer
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
            if let Some(entry) = self.core.history.get_mut(i) {
                entry.apply_context_override(
                    ContextOverride::ForcedExclude,
                    ChangeSource::Internal {
                        label: "mark_entries_ignored".into(),
                    },
                );
            }
        }
    }

    /// Toggle the context override on the currently selected entry.
    ///
    /// If the entry uses the default, override to the opposite of the kind default.
    /// If the entry is already overridden, revert to the default.
    /// Returns `Some(entry_id)` if the override was changed, `None` if no-op
    /// (entry was already in the toggled state) or no entry is selected.
    pub fn toggle_entry_ignored(&mut self) -> Option<crate::protocol::ChatEntryId> {
        if let Some(idx) = self.selected_entry_index() {
            let items = self.visual_items().clone();
            let hist_idx = if items.is_empty() {
                idx
            } else {
                match items.get(idx) {
                    Some(VisualItem::Entry(h)) => *h,
                    _ => return None, // collapsed block or invalid
                }
            };
            if let Some(entry) = self.core.history.get_mut(hist_idx) {
                let new_value = match entry.context_override() {
                    ContextOverride::Default => {
                        if entry.kind.is_included_by_default() {
                            ContextOverride::ForcedExclude
                        } else {
                            ContextOverride::ForcedInclude
                        }
                    }
                    ContextOverride::ForcedExclude | ContextOverride::ForcedInclude => {
                        ContextOverride::Default
                    }
                };
                if entry.apply_context_override(new_value, ChangeSource::User) {
                    return Some(entry.id.clone());
                }
            }
        }
        None
    }

    /// Set the context override on the currently selected entry to a specific
    /// value (not a toggle). Used by the x-sweep to apply a captured state.
    ///
    ///
    /// Returns `Some(entry_id)` if the override was changed, `None` if no-op
    /// (entry was already at the target state) or no entry is selected.
    pub fn set_entry_context_override(
        &mut self,
        override_state: ContextOverride,
    ) -> Option<crate::protocol::ChatEntryId> {
        if let Some(idx) = self.selected_entry_index() {
            let items = self.visual_items().clone();
            let hist_idx = if items.is_empty() {
                idx
            } else {
                match items.get(idx) {
                    Some(VisualItem::Entry(h)) => *h,
                    _ => return None,
                }
            };
            if let Some(entry) = self.core.history.get_mut(hist_idx)
                && entry.apply_context_override(override_state, ChangeSource::User)
            {
                return Some(entry.id.clone());
            }
        }
        None
    }

    /// After a sweep changes an entry from excluded to in-context, propagate
    /// `shown_ignored_blocks` to any new sub-blocks created by the split.
    ///
    /// When an entry inside a shown (expanded) ignored block becomes in-context,
    /// it splits the block. If the original block was shown, the new forward
    /// sub-block should also be shown so entries remain visible.
    ///
    /// No-op if the entry was not inside a shown block.
    pub fn propagate_shown_on_unignore(&mut self, entry_id: &ChatEntryId) {
        let Some(idx) = self.core.history.iter().position(|e| e.id == *entry_id) else {
            return;
        };

        // Scan backward to find the containing block's start.
        let mut block_start = idx;
        while block_start > 0 {
            let Some(prev) = self.core.history.get(block_start - 1) else {
                break;
            };
            if prev.is_in_context() || prev.pin_position.is_some() {
                break;
            }
            block_start -= 1;
        }

        let Some(block_entry) = self.core.history.get(block_start) else {
            return;
        };
        let block_representative = block_entry.id.clone();
        if !self.ui.shown_ignored_blocks.contains(&block_representative) {
            return; // Block was not shown — nothing to propagate.
        }

        let forward_start = idx + 1;
        // Scan forward from the changed entry to find a new forward sub-block.
        let Some(forward_entry) = self.core.history.get(forward_start) else {
            return;
        };
        if forward_entry.is_in_context() || forward_entry.pin_position.is_some() {
            return; // No excluded entries after — no sub-block to create.
        }

        // The forward sub-block's representative is its first entry.
        self.ui
            .shown_ignored_blocks
            .insert(forward_entry.id.clone());
    }

    /// Rebuild the visual items list from the current history and
    /// `shown_ignored_blocks`. Needed during sweep operations to keep the
    /// visual items consistent with mutated entry state between render passes.
    pub fn rebuild_visual_items(&self) {
        use crate::feat::ui::chat_log::visual_item::{
            DEFAULT_MIN_COLLAPSE_COUNT, PROXIMITY_COUNT, build_visual_items,
        };
        let items = build_visual_items(
            &self.core.history,
            &self.ui.shown_ignored_blocks,
            PROXIMITY_COUNT,
            DEFAULT_MIN_COLLAPSE_COUNT,
        );
        self.set_visual_items(items);
    }

    /// Returns the sweep target state if an active sweep exists and has not
    /// expired (>100ms since last press). Consumes (clears) the sweep state
    /// regardless of expiry - the caller must re-store it if continuing.
    pub fn take_ignore_sweep(&mut self) -> Option<ContextOverride> {
        let (instant, override_state) = self.ui.ignore_sweep.take()?;
        (instant.elapsed() < std::time::Duration::from_millis(100)).then_some(override_state)
    }

    /// Starts or continues a sweep by storing the target state and current time.
    pub fn set_ignore_sweep(&mut self, target: ContextOverride) {
        self.ui.ignore_sweep = Some((std::time::Instant::now(), target));
    }

    /// Clears the sweep state, resetting to normal toggle behavior.
    pub fn clear_ignore_sweep(&mut self) {
        self.ui.ignore_sweep = None;
    }

    /// Whether this session has no history entries.
    ///
    /// A session is "empty" when it has never had any entries pushed -
    /// no user messages, no system messages, nothing.
    /// Not to be confused with [`Self::is_idle`] which checks
    /// streaming/sending/assembling state.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.core.history.is_empty()
    }

    /// Whether this session is program-initiated (automated), not a normal user conversation.
    #[must_use]
    pub fn is_automated(&self) -> bool {
        self.core.is_automated
    }

    /// Mark this session as having been meaningfully interacted with by the user.
    /// Once set, the session becomes eligible for persistence.
    pub fn mark_interacted(&mut self) {
        self.core.has_interacted = true;
    }

    /// Whether this session has been interacted with.
    #[must_use]
    pub fn has_interacted(&self) -> bool {
        self.core.has_interacted
    }

    /// Whether this session should be persisted to disk.
    ///
    /// Returns `false` immediately when `persist == false` (explicitly marked
    /// transient — e.g. a plugin enrichment one-shot). Otherwise returns `true`
    /// if any of:
    /// - The user has interacted with this session (`has_interacted`)
    /// - The session has a lifecycle (setup/teardown scripts)
    /// - The session was forked from another session
    #[must_use]
    pub fn is_persistable(&self) -> bool {
        if !self.core.persist {
            return false;
        }
        if self.core.lifecycle_name.is_some() {
            return true;
        }
        if self.core.parent_session.is_some() {
            return true;
        }
        if self.core.has_interacted {
            return true;
        }
        false
    }

    /// Append an entry to the history and return its index.
    ///
    /// Implements smart auto-scroll: only resets scroll and advances cursor
    /// to the new entry if the cursor was on the previous last entry (or history
    /// was empty). Otherwise, appends silently - preserving the user's scroll
    /// position and selection.
    /// Push a chat entry onto the history.
    ///
    /// Future work: restrict to the session feature module and require external
    /// code to use the `PushChatEntry` command (which also triggers persistence).
    pub fn push_entry(&mut self, entry: ChatEntry) -> usize {
        let was_at_last = self
            .ui
            .selected_cursor_id
            .as_ref()
            .is_none_or(|id| self.core.history.last().is_some_and(|e| &e.id == id));
        let index = self.core.history.len();
        self.core.history.push(entry);
        if was_at_last {
            self.reset_scroll();
            if let Some(entry) = self.core.history.last() {
                self.ui.selected_cursor_id = Some(entry.id.clone());
            }
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
        // Delegate index shifting to the machine (handles all 4 streaming fields).
        self.core
            .ephemeral
            .machine
            .shift_streaming_indices_for_insert_at(clamped);
        clamped
    }

    /// Lazily create the Assistant entry for the current stream.
    ///
    /// Called on first `append_stream_token`, `finish_streaming`,
    /// `begin_tool_call`, or `cancel_streaming`. No-op if the entry
    /// already exists or the session is not streaming.
    fn ensure_assistant_entry(&mut self, dispatched_at: jiff::Timestamp) {
        if self
            .core
            .ephemeral
            .machine
            .streaming_entry_index()
            .is_some()
            || !matches!(self.core.ephemeral.machine.kind(), PhaseKind::Streaming)
        {
            return;
        }
        let mut entry = ChatEntry::assistant("");
        entry.timing = EntryTiming::streamed(dispatched_at);
        entry.timing.set_first_token();
        let index = self.push_entry(entry);
        self.core.ephemeral.machine.set_streaming_entry_index(index);
    }

    #[expect(
        clippy::indexing_slicing,
        reason = "index comes from streaming_entry_index which is always valid"
    )]
    fn finish_streaming_entry(&mut self, idx: usize) {
        self.core.history[idx].timing.finish();
    }

    /// Begin a new streaming response.
    //
    // Phase 1 wiring: delegates to machine.on_first_token() and syncs the
    // legacy phase field. If the machine rejects the transition (e.g. not in
    // Sending), logs a warning and returns without changing state - matching
    // the old soft-guard behavior.
    //
    // Note: The old code accepted both `Sending` and `Idle` phases. The machine
    // only accepts `Sending → Streaming`. To maintain backward compat during the
    // migration, we also accept `Idle → Streaming` by first transitioning to
    // `Sending` then to `Streaming`.
    pub fn begin_streaming(&mut self) {
        use crate::feat::session::phase_machine::PhaseTransitions;
        // If Idle, first transition to Sending (some callers skip begin_sending()).
        if matches!(self.core.ephemeral.machine.kind(), PhaseKind::Idle)
            && let Err(e) = self.core.ephemeral.machine.on_dispatch_message()
        {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.machine.kind(),
                err = %e,
                "begin_streaming: on_dispatch_message rejected - ignoring"
            );
            return;
        }
        if let Err(e) = self.core.ephemeral.machine.on_first_token() {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.machine.kind(),
                err = %e,
                "begin_streaming: machine rejected transition - ignoring"
            );
        }
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
    pub fn append_stream_token<S>(
        &mut self,
        token: S,
        dispatched_at: jiff::Timestamp,
    ) -> Result<(), StreamingError>
    where
        S: AsRef<str>,
    {
        if !matches!(self.core.ephemeral.machine.kind(), PhaseKind::Streaming) {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.machine.kind(),
                "append_stream_token called while not streaming - ignoring"
            );
            return Err(StreamingError::NoStreamingEntry);
        }
        self.ensure_assistant_entry(dispatched_at);
        let index = self
            .core
            .ephemeral
            .machine
            .streaming_entry_index()
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
    pub fn begin_thinking(&mut self, dispatched_at: jiff::Timestamp) {
        if !matches!(self.core.ephemeral.machine.kind(), PhaseKind::Streaming) {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.machine.kind(),
                "begin_thinking called while not streaming - ignoring"
            );
            return;
        }
        if self
            .core
            .ephemeral
            .machine
            .streaming_thinking_entry_index()
            .is_some()
        {
            tracing::warn!("begin_thinking called while already thinking - ignoring");
            return;
        }
        let mut entry = ChatEntry::thinking("");
        entry.timing = EntryTiming::streamed(dispatched_at);
        entry.timing.set_first_token();
        // Insert thinking BEFORE the assistant entry when the assistant entry
        // already exists. Some providers (OpenRouter) send reasoning tokens
        // AFTER content tokens, so the assistant entry is already in history.
        // The thinking entry should appear before it in the chat log.
        let index = if let Some(assistant_idx) = self.core.ephemeral.machine.streaming_entry_index()
        {
            self.insert_entry_at(assistant_idx, entry)
        } else {
            self.push_entry(entry)
        };
        self.core
            .ephemeral
            .machine
            .set_streaming_thinking_entry_index(index);
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
            .machine
            .streaming_thinking_entry_index()
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
        self.core.ephemeral.machine.streaming_thinking_entry_index()
    }

    /// Mark streaming as finished (normal completion).
    //
    // Phase 1 wiring: delegates to machine.on_stream_completed_finished()
    // and syncs legacy phase field.
    pub fn finish_streaming(&mut self, preserve_assistant: bool, dispatched_at: jiff::Timestamp) {
        use crate::feat::session::phase_machine::PhaseTransitions;
        if preserve_assistant {
            self.ensure_assistant_entry(dispatched_at);
        }

        // Set finished_at on the assistant entry.
        if let Some(idx) = self.core.ephemeral.machine.streaming_entry_index() {
            self.finish_streaming_entry(idx);
        }

        if let Err(e) = self.core.ephemeral.machine.on_stream_completed_finished() {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.machine.kind(),
                err = %e,
                "finish_streaming: machine rejected transition"
            );
        }
        // Streaming indices cleared automatically by Phase::Streaming drop.
    }

    /// Cancel streaming but keep partial text in history.
    //
    // Phase 1 wiring: delegates to machine.cancel() and syncs legacy phase.
    pub fn cancel_streaming(&mut self, dispatched_at: jiff::Timestamp) {
        self.ensure_assistant_entry(dispatched_at);

        // Set finished_at on the assistant entry.
        if let Some(idx) = self.core.ephemeral.machine.streaming_entry_index() {
            self.finish_streaming_entry(idx);
        }
        if let Err(e) = self.core.ephemeral.machine.cancel() {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.machine.kind(),
                err = %e,
                "cancel_streaming: machine rejected cancel"
            );
        }
        // All streaming indices cleaned up by StreamingPhase drop on cancel()
    }

    /// Cancel streaming and drain steering fragments plus queued messages back
    /// into the input buffer.
    ///
    /// Used when the user interrupts via ESC-confirm during an active stream.
    /// Steering fragments (drained first) and the display text of each queued
    /// `UserMessage` are joined with `"\n\n---\n\n"` and replace whatever was
    /// in the input box. `ToolContinuation` items are silently discarded.
    /// If nothing was drained, the input box is left untouched.
    pub fn cancel_stream_and_drain(&mut self) {
        self.cancel_streaming(jiff::Timestamp::now());
        let drained_text = self.drain_cancel_chunks().join("\n\n---\n\n");
        if !drained_text.is_empty() {
            self.chat_input_mut().replace_all(drained_text);
        }
    }

    /// Collects the text chunks drained out of the steering buffer and turn
    /// queue, in cancel-recovery order.
    ///
    /// Steering fragments come first, followed by the display text of each
    /// queued `UserMessage`. `ToolContinuation` items are discarded. The
    /// caller applies whatever separator it wants.
    fn drain_cancel_chunks(&mut self) -> Vec<String> {
        let steering = self.steering_buffer_mut().drain_fragments();
        let queue = self.drain_queue();
        steering
            .into_iter()
            .chain(queue.into_iter().filter_map(|item| match item {
                crate::feat::session::queue_item::QueueItem::UserMessage(entry) => {
                    match &entry.kind {
                        ChatEntryKind::User { display, .. } => Some(display.clone()),
                        _ => None,
                    }
                }
                crate::feat::session::queue_item::QueueItem::ToolContinuation => None,
            }))
            .collect()
    }

    /// Returns the current session lifecycle phase.
    pub fn phase(&self) -> PhaseKind {
        self.core.ephemeral.machine.kind()
    }

    // --- Tool call streaming ---

    /// Create a placeholder `ToolCall` entry and record its history index.
    ///
    /// Called when `ToolUseStarted` arrives - the tool name is known but arguments
    /// are still streaming in.
    pub fn begin_tool_call(
        &mut self,
        index: usize,
        id: &str,
        name: &str,
        dispatched_at: jiff::Timestamp,
    ) {
        self.ensure_assistant_entry(dispatched_at);
        let mut entry = ChatEntry::tool_call(id, name, "");
        entry.timing = EntryTiming::streamed(dispatched_at);
        entry.timing.set_first_token();
        let history_index = self.push_entry(entry);
        let Some(indices) = self
            .core
            .ephemeral
            .machine
            .streaming_tool_call_indices_mut()
        else {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.machine.kind(),
                index,
                "begin_tool_call called while not streaming - ignoring"
            );
            return;
        };
        indices.insert(index, history_index);
    }

    /// Append an incremental delta to a streaming tool call's arguments.
    ///
    /// `partial_json` is appended to the existing arguments string - it is *not*
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
            .machine
            .streaming_tool_call_indices()
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
    pub fn finalize_tool_call(&mut self, id: &str, name: &str, arguments: &str) {
        for entry in self.core.history.iter_mut().rev() {
            if let ChatEntryKind::ToolCall {
                id: ref _entry_id, ..
            } = entry.kind
            {
                entry.kind = ChatEntryKind::ToolCall {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    arguments: arguments.to_owned(),
                };
                entry.timing.finish();
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
    pub fn begin_tool_result(
        &mut self,
        tool_call_id: &str,
        name: &str,
        dispatched_at: jiff::Timestamp,
    ) {
        // Early return if not in Streaming phase — don't push orphaned entries.
        if self
            .core
            .ephemeral
            .machine
            .streaming_tool_result_indices_mut()
            .is_none()
        {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.machine.kind(),
                tool_call_id,
                "begin_tool_result called while not streaming - ignoring"
            );
            return;
        }

        let mut entry = ChatEntry::tool_result(
            tool_call_id,
            name,
            "",
            crate::feat::session::tool_result_status::ToolResultStatus::Pending,
        );
        entry.timing = EntryTiming::streamed(dispatched_at);
        entry.timing.set_first_token();
        let history_index = self.push_entry(entry);

        // Re-acquire the streaming index map after push_entry releases &mut self.
        if let Some(indices) = self
            .core
            .ephemeral
            .machine
            .streaming_tool_result_indices_mut()
        {
            indices.insert(tool_call_id.to_owned(), history_index);
        }
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
            .machine
            .streaming_tool_result_indices()
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
        clippy::too_many_arguments,
        reason = "mirrors begin_tool_result + new pin_position; refactor would require struct-builder pattern"
    )]
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
        mut full_content: Option<String>,
        mut truncation: Option<jinn_provider::tool_types::TruncationMeta>,
        pin_position: Option<PinPosition>,
    ) {
        let status = if success {
            crate::feat::session::tool_result_status::ToolResultStatus::Success
        } else {
            crate::feat::session::tool_result_status::ToolResultStatus::Failure
        };

        if let Some(history_index) = self
            .core
            .ephemeral
            .machine
            .streaming_tool_result_indices_mut()
            .and_then(|map| map.remove(tool_call_id))
        {
            // Finalize existing pending entry.
            let entry = &mut self.core.history[history_index];
            match &mut entry.kind {
                ChatEntryKind::ToolResult {
                    content: entry_content,
                    status: entry_status,
                    full_content: entry_full_content,
                    truncation: entry_truncation,
                    pin_position: entry_kind_pin,
                    ..
                } => {
                    content.clone_into(entry_content);
                    *entry_status = status;
                    *entry_full_content = full_content;
                    *entry_truncation = truncation;
                    *entry_kind_pin = pin_position;
                    // Entry-level pin mirrors the kind-level pin so assembly,
                    // compaction, and UI consumers read a single field.
                    entry.pin_position = pin_position;
                    entry.timing.finish();
                }

                _ => {}
            }
        } else {
            // Streaming index not available. Search history for an existing
            // ToolResult with matching kind.id (e.g., a pending entry created
            // by begin_tool_result before the streaming phase was dropped).
            let mut existing_found = false;
            for entry in self.core.history.iter_mut().rev() {
                match &mut entry.kind {
                    ChatEntryKind::ToolResult {
                        id: entry_id,
                        content: entry_content,
                        status: entry_status,
                        full_content: entry_full_content,
                        truncation: entry_truncation,
                        pin_position: entry_kind_pin,
                        ..
                    } if entry_id == tool_call_id => {
                        content.clone_into(entry_content);
                        *entry_status = status;
                        *entry_full_content = full_content.take();
                        *entry_truncation = truncation.take();
                        *entry_kind_pin = pin_position;
                        entry.pin_position = pin_position;
                        entry.timing.finish();
                        existing_found = true;
                        break;
                    }

                    _ => {}
                }
            }

            if !existing_found {
                // No existing entry found - push a new one.
                let mut entry = if let Some(meta) = truncation {
                    let full = full_content.unwrap_or_default();
                    ChatEntry::tool_result_truncated(
                        tool_call_id,
                        name,
                        content.to_owned(),
                        full,
                        status,
                        meta,
                    )
                } else {
                    ChatEntry::tool_result(tool_call_id, name, content, status)
                };
                // Propagate tool-requested pin onto both the kind variant and
                // the entry-level field so assembly/compaction read a single source.
                if let ChatEntryKind::ToolResult {
                    pin_position: ref mut kp,
                    ..
                } = entry.kind
                {
                    *kp = pin_position;
                }
                entry.pin_position = pin_position;
                self.push_entry(entry);
            }
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

    /// Push an item onto the front of the queue (for priority items).
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

    /// Read-only access to the session profile.
    pub fn profile(&self) -> &SessionProfile {
        &self.core.profile
    }

    /// Mutable access to the session profile.
    pub fn profile_mut(&mut self) -> &mut SessionProfile {
        &mut self.core.profile
    }

    /// Set the model selection for this session.
    pub fn set_model(&mut self, model: ModelSelection) {
        self.core.profile.model = model;
    }

    /// Whether a tool is enabled for this session.
    ///
    /// Returns `true` if the tool name is not in the disabled set.
    /// An empty disabled set means all tools are enabled.
    #[must_use]
    pub fn is_tool_enabled(&self, tool_name: &str) -> bool {
        !self.core.profile.disabled_tools.contains(tool_name)
    }

    /// Read-only access to this session's disabled tool names.
    ///
    /// Opt-out model: tools not in this set are enabled.
    pub fn disabled_tools(&self) -> &HashSet<String> {
        &self.core.profile.disabled_tools
    }

    /// Replace the disabled tool set for this session.
    ///
    /// Used by the tool picker to commit toggle state.
    pub fn set_disabled_tools(&mut self, tools: HashSet<String>) {
        self.core.profile.disabled_tools = tools;
    }

    /// Returns `true` if the skill is enabled for this session.
    ///
    /// An empty disabled set means all skills are enabled.
    #[must_use]
    pub fn is_skill_enabled(&self, skill_name: &str) -> bool {
        !self.core.profile.disabled_skills.contains(skill_name)
    }

    /// Read-only access to this session's disabled skill names.
    ///
    /// Opt-out model: skills not in this set are enabled.
    pub fn disabled_skills(&self) -> &HashSet<String> {
        &self.core.profile.disabled_skills
    }

    /// Replace the disabled skill set for this session.
    ///
    /// Used by the skill picker to commit toggle state.
    pub fn set_disabled_skills(&mut self, skills: HashSet<String>) {
        self.core.profile.disabled_skills = skills;
    }
    /// Compute the set of skill names that are currently loaded in this session.
    ///
    /// A skill is considered loaded if its body is present in history as a pinned
    /// ToolResult from the `skill` tool whose content begins with `<skill name="X"`.
    pub fn loaded_skills(&self) -> HashSet<String> {
        use crate::feat::session::chat_entry::ChatEntryKind;
        use crate::feat::skills::parse_loaded_skill_name;

        let mut out = HashSet::new();
        for entry in self.history() {
            if !entry.is_pinned() {
                continue;
            }
            let ChatEntryKind::ToolResult {
                name: tool_name,
                content,
                ..
            } = &entry.kind
            else {
                continue;
            };
            if tool_name != "skill" {
                continue;
            }
            // Skill bodies are pinned as `<skill name="X" ...>` — extract X.
            let Some(skill_name) = parse_loaded_skill_name(content) else {
                continue;
            };
            out.insert(skill_name.to_owned());
        }
        out
    }

    /// The model selection for this session.
    pub fn model_selection(&self) -> &ModelSelection {
        &self.core.profile.model
    }
    pub fn model(&self) -> &ModelSelection {
        &self.core.profile.model
    }

    // --- Sending ---

    /// Mark the session as having dispatched a message to the LLM.
    //
    // Phase 1 wiring: delegates to machine.on_dispatch_message() and syncs
    // the legacy phase field.
    pub fn begin_sending(&mut self) {
        use crate::feat::session::phase_machine::PhaseTransitions;
        if let Err(e) = self.core.ephemeral.machine.on_dispatch_message() {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.machine.kind(),
                err = %e,
                "begin_sending: machine rejected transition - ignoring"
            );
        }
    }

    /// Clear the sending flag (called when the first stream token arrives).
    //
    /// Complete the sending phase via the machine's validated transition.
    ///
    /// This should be called when a tool batch completes and the tool loop
    /// is disabled. The machine reads the `tool_loop_disabled` flag and
    /// transitions `Sending → Idle` (if set) or `Sending → Streaming` (if not).
    ///
    /// The caller must ensure `set_tool_loop_disabled(true)` has been called
    /// before this method if the tool loop should be terminated.
    pub fn finish_sending_via_machine(&mut self) {
        use crate::feat::session::phase_machine::PhaseTransitions;
        if let Err(e) = self.core.ephemeral.machine.on_tool_batch_completed() {
            tracing::warn!(
                current_phase = ?self.core.ephemeral.machine.kind(),
                err = %e,
                "finish_sending_via_machine: machine rejected transition - ignoring"
            );
        }
    }

    /// Transition to Working phase (a background operation started).
    ///
    /// Increment the busy counter. Called when a background operation starts.
    /// The count is ephemeral (not persisted).
    pub fn begin_busy(&mut self) {
        self.core.ephemeral.busy_count += 1;
    }

    /// Decrement the busy counter (floor at 0). Called when one background
    /// operation completes. Returns the new count.
    pub fn complete_busy(&mut self) -> usize {
        self.core.ephemeral.busy_count = self.core.ephemeral.busy_count.saturating_sub(1);
        self.core.ephemeral.busy_count
    }

    /// Hard-reset the busy counter to zero. Cancels all tracked operations.
    pub fn cancel_busy(&mut self) {
        self.core.ephemeral.busy_count = 0;
    }

    /// Returns the current number of active background operations.
    pub fn busy_count(&self) -> usize {
        self.core.ephemeral.busy_count
    }

    /// Returns `true` when any background operation is in progress.
    pub fn is_busy(&self) -> bool {
        self.core.ephemeral.busy_count > 0
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
        let Some(selected_idx) = self.selected_entry_index() else {
            return;
        };

        let ranges = self.ui.entry_line_ranges.read().clone();

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

        let current_offset = self.ui.rendered_scroll_offset.load(Ordering::Relaxed);

        let new_offset = if entry_height <= viewport_height {
            // Entry fits in viewport - adjust only if it's outside.
            if abs_start < current_offset {
                abs_start
            } else if abs_end > current_offset.saturating_add(viewport_height) {
                abs_end.saturating_sub(viewport_height)
            } else {
                // Already visible - no change needed.
                return;
            }
        } else {
            // Entry is taller than viewport - align top.
            if abs_start >= current_offset.saturating_add(viewport_height) {
                abs_start
            } else if abs_end <= current_offset {
                abs_end.saturating_sub(viewport_height)
            } else {
                // Already overlapping - no change needed.
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

    /// Returns the screen-space Y coordinate of the top of the currently-selected
    /// chat entry within the chat-log area, or `None` if no entry is selected or
    /// the render-pipeline cache is empty.
    ///
    /// The returned Y is in terminal (absolute) coordinates: it already incorporates
    /// `chat_log_area_y` and `blank_count`. Callers can pass it directly as the
    /// `entry_top_y` argument to
    /// [`audit_popup_rect`](crate::feat::ui::chat_log::audit_popup::audit_popup_rect).
    ///
    /// If the selected entry's top is scrolled above the viewport, returns
    /// `chat_log_area_y` (clamped to the top of the chat-log area).
    ///
    /// Returns a meaningful value only after the chat-log render pipeline has
    /// populated the cached fields for the current frame.
    pub fn selected_entry_screen_y(&self, chat_log_area_y: u16) -> Option<u16> {
        let vi_idx = self.selected_entry_index()?;
        let ranges = self.ui.entry_line_ranges.read();
        let &(start, _end) = ranges.get(vi_idx)?;
        drop(ranges);

        let blank_count = self.ui.blank_count.load(Ordering::Relaxed);
        let scroll_offset = self.ui.rendered_scroll_offset.load(Ordering::Relaxed);

        // wrapped-line coord of entry top, with bottom-alignment blank padding
        let abs_start = start.saturating_add(blank_count);

        // viewport top in the same coord space
        let viewport_top = scroll_offset;

        // visible-Y offset within viewport (0 = top of chat-log area)
        let viewport_offset = abs_start.saturating_sub(viewport_top);

        // absolute screen Y; clamped to chat-log area top
        let screen_y = chat_log_area_y.saturating_add(viewport_offset);
        Some(screen_y)
    }

    /// Store the rendered scroll offset (actual viewport position after clamping
    /// and scroll-to-selected adjustment). Called by the render pipeline each frame.
    pub fn set_rendered_scroll_offset(&self, offset: u16) {
        self.ui
            .rendered_scroll_offset
            .store(offset, Ordering::Relaxed);
    }

    // --- Renderer viewport state ---

    /// Store per-entry wrapped line ranges computed by the renderer.
    ///
    /// `entry_line_ranges[i] = (start_wrapped_line, end_wrapped_line)` in the
    /// wrapped coordinate space. Called each frame by the chat log renderer.
    pub fn set_entry_line_ranges(&self, ranges: Vec<(u16, u16)>) {
        {
            let mut guard = self.ui.entry_line_ranges.write();
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
        let ranges = self.ui.entry_line_ranges.read().clone();
        if ranges.is_empty() {
            return 0..0;
        }

        let viewport_height = self.ui.viewport_height.load(Ordering::Relaxed);
        let blank_count = self.ui.blank_count.load(Ordering::Relaxed);
        let scroll_offset = self.ui.rendered_scroll_offset.load(Ordering::Relaxed);

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
            self.set_selected_entry_index(range.start);
        } else {
            let mut idx = range.start;
            while idx < range.end {
                let selectable = match items.get(idx) {
                    Some(VisualItem::CollapsedIgnoredBlock { .. }) => true,
                    Some(VisualItem::Entry(hist_idx)) => self
                        .core
                        .history
                        .get(*hist_idx)
                        .is_some_and(|e| !e.is_empty_assistant()),
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
                    Some(VisualItem::Entry(hist_idx)) => self
                        .core
                        .history
                        .get(*hist_idx)
                        .is_some_and(|e| !e.is_empty_assistant()),
                    None => false,
                };
                if selectable {
                    self.set_selected_entry_index(idx);
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
            self.set_selected_entry_index(range.end.saturating_sub(1));
        } else {
            let mut idx = range.end.saturating_sub(1);
            while idx > range.start {
                let selectable = match items.get(idx) {
                    Some(VisualItem::CollapsedIgnoredBlock { .. }) => true,
                    Some(VisualItem::Entry(hist_idx)) => self
                        .core
                        .history
                        .get(*hist_idx)
                        .is_some_and(|e| !e.is_empty_assistant()),
                    None => false,
                };
                if selectable {
                    break;
                }
                idx = idx.saturating_sub(1);
            }
            let selectable = match items.get(idx) {
                Some(VisualItem::CollapsedIgnoredBlock { .. }) => true,
                Some(VisualItem::Entry(hist_idx)) => self
                    .core
                    .history
                    .get(*hist_idx)
                    .is_some_and(|e| !e.is_empty_assistant()),
                None => false,
            };
            if selectable {
                self.set_selected_entry_index(idx);
            }
        }
    }

    // --- History restoration ---

    /// Restore conversation history from a persisted snapshot.
    ///
    /// Replaces the current history with the given entries. Used by session
    /// persistence to rehydrate a session from disk.
    #[expect(
        clippy::else_if_without_else,
        reason = "no-op on fallthrough is intentional"
    )]
    pub fn restore_history(&mut self, entries: Vec<ChatEntry>) {
        self.core.history.replace_all(entries);
        if self.core.history.is_empty() {
            self.ui.selected_cursor_id = None;
        } else if let Some(entry) = self.core.history.last() {
            self.ui.selected_cursor_id = Some(entry.id.clone());
        }
        self.reset_scroll();
    }

    // --- Pinning ---

    /// Pin an entry by ID, setting its pin position.
    ///
    /// If no entry with the given ID exists, this is a no-op.
    /// Pin an entry by ID, setting its pin position.
    ///
    /// If no entry with the given ID exists, this is a no-op.
    ///
    /// When pinning an ignored entry inside a shown (expanded) ignored block,
    /// the pinned entry becomes a block splitter in `build_visual_items`.
    /// This propagates `shown_ignored_blocks` to any new forward sub-block
    /// created by the split, keeping all entries visible.
    pub fn pin_entry(&mut self, id: &ChatEntryId, position: PinPosition) {
        let Some(entry) = self.core.history.iter_mut().find(|e| e.id == *id) else {
            return;
        };
        let is_ignored = !entry.is_in_context();
        entry.pin_position = Some(position);

        // Propagation: only for non-context entries inside shown blocks.
        if !is_ignored {
            return;
        }

        let Some(idx) = self.core.history.iter().position(|e| e.id == *id) else {
            return;
        };

        // Scan backward to find the containing block's start.
        // Same boundary rules as `build_visual_items` and `toggle_ignored_block_visibility`.
        let mut block_start = idx;
        while block_start > 0
            && self
                .core
                .history
                .get(block_start - 1)
                .is_some_and(|e| !e.is_in_context())
            && self
                .core
                .history
                .get(block_start - 1)
                .is_some_and(|e| e.pin_position.is_none())
        {
            block_start -= 1;
        }

        let Some(block_entry) = self.core.history.get(block_start) else {
            return;
        };
        let block_representative = block_entry.id.clone();
        if !self.ui.shown_ignored_blocks.contains(&block_representative) {
            return; // Block was collapsed - nothing to propagate.
        }

        // Scan forward from the pinned entry to find the new forward sub-block.
        let forward_start = idx + 1;
        if forward_start >= self.core.history.len() {
            return; // No entries after the pin.
        }

        let Some(forward_entry) = self.core.history.get(forward_start) else {
            return;
        };
        if forward_entry.is_in_context() || forward_entry.pin_position.is_some() {
            return; // Forward entry is not part of an ignored block.
        }

        // The forward sub-block's representative is its first entry.
        self.ui
            .shown_ignored_blocks
            .insert(forward_entry.id.clone());
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
            .selected_entry_index()
            .map_or(0, |i| i.saturating_add(1).min(max));
        let mut idx = start;
        while idx < max {
            let selectable = match items.get(idx) {
                Some(VisualItem::CollapsedIgnoredBlock { .. }) => true,
                Some(VisualItem::Entry(hist_idx)) => self
                    .core
                    .history
                    .get(*hist_idx)
                    .is_some_and(|e: &ChatEntry| !e.is_empty_assistant()),
                None => false,
            };
            if selectable {
                break;
            }
            idx = idx.saturating_add(1);
        }
        let selectable = match items.get(idx) {
            Some(VisualItem::CollapsedIgnoredBlock { .. }) => true,
            Some(VisualItem::Entry(hist_idx)) => self
                .core
                .history
                .get(*hist_idx)
                .is_some_and(|e: &ChatEntry| !e.is_empty_assistant()),
            None => false,
        };
        if selectable {
            self.set_selected_entry_index(idx);
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
            .selected_entry_index()
            .map_or(items.len().saturating_sub(1), |i| i.saturating_sub(1));
        let mut idx = start;
        while idx > 0 {
            let selectable = match items.get(idx) {
                Some(VisualItem::CollapsedIgnoredBlock { .. }) => true,
                Some(VisualItem::Entry(hist_idx)) => self
                    .core
                    .history
                    .get(*hist_idx)
                    .is_some_and(|e: &ChatEntry| !e.is_empty_assistant()),
                None => false,
            };
            if selectable {
                break;
            }
            idx = idx.saturating_sub(1);
        }
        let selectable = match items.get(idx) {
            Some(VisualItem::CollapsedIgnoredBlock { .. }) => true,
            Some(VisualItem::Entry(hist_idx)) => self
                .core
                .history
                .get(*hist_idx)
                .is_some_and(|e: &ChatEntry| !e.is_empty_assistant()),
            None => false,
        };
        if selectable {
            self.set_selected_entry_index(idx);
        }
    }

    /// Clear the entry selection.
    pub fn clear_selection(&mut self) {
        self.ui.selected_cursor_id = None;
    }

    /// Set the selected entry index directly.
    ///
    /// Use for programmatic selection (e.g., sidebar pin sync).
    /// Does not validate bounds - caller must ensure index is valid.
    pub fn set_selected_entry_index(&mut self, index: usize) {
        let id = {
            let items = self.visual_items();
            if items.is_empty() {
                self.core.history.get(index).map(|e| e.id.clone())
            } else {
                items.get(index).and_then(|item| {
                    crate::feat::ui::chat_log::visual_item::entry_id_from_visual_item(
                        item,
                        &self.core.history,
                    )
                })
            }
        };
        if let Some(id) = id {
            self.ui.selected_cursor_id = Some(id);
        }
    }

    /// Fallback: select next entry by walking raw history.
    /// Used when visual items haven't been computed yet (before first render).
    fn select_next_entry_fallback(&mut self) {
        if self.core.history.is_empty() {
            return;
        }
        let max = self.core.history.len() - 1;
        let start = self
            .selected_entry_index()
            .map_or(0, |i| i.saturating_add(1).min(max));
        let mut idx = start;
        while idx < max
            && self
                .core
                .history
                .get(idx)
                .is_none_or(super::chat_entry::ChatEntry::is_empty_assistant)
        {
            idx = idx.saturating_add(1);
        }
        if self
            .core
            .history
            .get(idx)
            .is_some_and(|e| !e.is_empty_assistant())
        {
            self.set_selected_entry_index(idx);
        }
    }

    /// Fallback: select prev entry by walking raw history.
    /// Used when visual items haven't been computed yet (before first render).
    fn select_prev_entry_fallback(&mut self) {
        if self.core.history.is_empty() {
            return;
        }
        let start = self
            .selected_entry_index()
            .map_or(self.core.history.len().saturating_sub(1), |i| {
                i.saturating_sub(1)
            });
        let mut idx = start;
        while idx > 0
            && self
                .core
                .history
                .get(idx)
                .is_none_or(super::chat_entry::ChatEntry::is_empty_assistant)
        {
            idx = idx.saturating_sub(1);
        }
        if self
            .core
            .history
            .get(idx)
            .is_some_and(|e| !e.is_empty_assistant())
        {
            self.set_selected_entry_index(idx);
        }
    }

    // --- Saved history position ---

    /// Saves the current chat log scroll position as the "pre-pin" snapshot.
    ///
    /// Call this before `sync_chat_log_cursor` changes the viewport.
    /// No-op if a position is already saved (prevents overwriting during
    /// a single Pins visit).
    pub fn save_history_position(&mut self) {
        if self.ui.saved_history_position.is_some() {
            return;
        }
        self.ui.saved_history_position = Some(SavedHistoryPosition {
            scroll_offset: self.ui.scroll_offset,
            selected_cursor_id: self.ui.selected_cursor_id.clone(),
        });
    }

    /// Restores the chat log to the saved "pre-pin" position, if one exists.
    ///
    /// Consumes the saved position (take semantics).
    pub fn restore_history_position(&mut self) {
        if let Some(saved) = self.ui.saved_history_position.take() {
            self.ui.scroll_offset = saved.scroll_offset;
            self.ui.selected_cursor_id = saved.selected_cursor_id;
        }
    }

    /// Discards the saved position without restoring.
    ///
    /// Used when leaving the sidebar to Normal scope - the pin's position
    /// should persist in the chat log.
    pub fn discard_saved_history_position(&mut self) {
        self.ui.saved_history_position = None;
    }

    /// Returns whether there is a saved history position.
    pub fn has_saved_history_position(&self) -> bool {
        self.ui.saved_history_position.is_some()
    }

    /// The index of the currently selected entry, if any.
    pub fn selected_entry_index(&self) -> Option<usize> {
        let cursor_id = self.ui.selected_cursor_id.as_ref()?;
        let items = self.visual_items();
        if items.is_empty() {
            return self.core.history.iter().position(|e| &e.id == cursor_id);
        }
        crate::feat::ui::chat_log::visual_item::resolve_entry_id_to_vi_index(
            cursor_id,
            &items,
            &self.core.history,
        )
    }

    /// The stored cursor ID (source of truth for selection).
    ///
    /// Unlike `selected_entry_id()` which returns `None` for collapsed blocks,
    /// this always returns the stored ID even when a collapsed block is selected.
    pub fn selected_cursor_id(&self) -> Option<&ChatEntryId> {
        self.ui.selected_cursor_id.as_ref()
    }

    /// Set the selected cursor to a specific entry by ID.
    ///
    /// Sets [`SessionUi::selected_cursor_id`] directly, bypassing
    /// visual-item index resolution. Use when the entry ID is already
    /// known (e.g., sidebar pin sync).
    pub fn set_selected_cursor_id(&mut self, id: ChatEntryId) {
        self.ui.selected_cursor_id = Some(id);
    }

    /// The currently selected entry, if any.
    ///
    /// Resolves through the visual items list: returns `None` if the
    /// selected item is a collapsed block rather than a real entry.
    /// Falls back to direct history indexing when visual items are empty
    /// (before the first render).
    pub fn selected_entry(&self) -> Option<&ChatEntry> {
        let vi_idx = self.selected_entry_index()?;
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
        let Some(entry) = self.core.history.get(idx) else {
            return;
        };
        if entry.is_in_context() {
            return;
        }
        // Scan backward to find the start of the contiguous ignored block.
        // Must match `build_visual_items` block definition: pinned entries
        // act as block splitters even when ignored.
        let mut block_start = idx;
        while block_start > 0
            && self
                .core
                .history
                .get(block_start - 1)
                .is_some_and(|e| !e.is_in_context())
            && self
                .core
                .history
                .get(block_start - 1)
                .is_some_and(|e| e.pin_position.is_none())
        {
            block_start -= 1;
        }
        let Some(block_rep) = self.core.history.get(block_start) else {
            return;
        };
        let block_representative = block_rep.id.clone();
        if self.ui.shown_ignored_blocks.contains(&block_representative) {
            self.ui.shown_ignored_blocks.remove(&block_representative);
        } else {
            self.ui.shown_ignored_blocks.insert(block_representative);
        }
    }

    /// Store the visual items list computed during render.
    pub fn set_visual_items(&self, items: Vec<crate::feat::ui::chat_log::visual_item::VisualItem>) {
        {
            let mut guard = self.ui.visual_items.write();
            *guard = items;
        }
    }

    /// Read-only access to the visual items list.
    pub fn visual_items(
        &self,
    ) -> parking_lot::RwLockReadGuard<'_, Vec<crate::feat::ui::chat_log::visual_item::VisualItem>>
    {
        self.ui.visual_items.read()
    }

    /// The visual item at the currently selected position, if any.
    pub fn selected_visual_item(
        &self,
    ) -> Option<crate::feat::ui::chat_log::visual_item::VisualItem> {
        let idx = self.selected_entry_index()?;
        self.ui.visual_items.read().get(idx).cloned()
    }

    /// Whether the cursor is currently on a collapsed ignored block.
    pub fn is_selected_collapsed_block(&self) -> bool {
        self.selected_visual_item()
            .is_some_and(|item| matches!(item, VisualItem::CollapsedIgnoredBlock { .. }))
    }

    /// Resolve the selected visual-item index to a history index.
    ///
    /// Returns `None` if nothing is selected or the selected item is a
    /// collapsed block (not a real entry).
    pub fn selected_history_index(&self) -> Option<usize> {
        let vi_idx = self.selected_entry_index()?;
        let items = self.ui.visual_items.read();

        if items.is_empty() {
            // Before first render, visual items haven't been computed yet.
            // Fall back: selected_entry_index IS a history index in this case.
            return Some(vi_idx);
        }
        match items.get(vi_idx)? {
            crate::feat::ui::chat_log::visual_item::VisualItem::Entry(hist_idx) => Some(*hist_idx),
            crate::feat::ui::chat_log::visual_item::VisualItem::CollapsedIgnoredBlock {
                ..
            } => None,
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
            .machine
            .is_tool_call_at_history_index(idx)
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
    /// Records are immutable once pushed - this is the only way to add them.
    pub fn push_token_record(&mut self, record: TokenRecord) {
        self.core.token_ledger.push(record);
    }

    /// Read-only access to this session's task list.
    pub fn task_list(&self) -> &crate::feat::todo_list::TaskList {
        &self.core.task_list
    }

    /// Mutable access to this session's task list.
    pub fn task_list_mut(&mut self) -> &mut crate::feat::todo_list::TaskList {
        &mut self.core.task_list
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
        model_used: Option<String>,
    ) -> Result<(), StreamingError> {
        let last = self
            .core
            .token_ledger
            .last_mut()
            .ok_or(StreamingError::EmptyLedger)?;
        last.tokens_received = tokens_received;
        last.cost = cost;
        last.model_used = model_used;
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

    // ----- Discovered resources (per-session, cwd-scoped) -----
    //
    // These are populated by the scan actors and read by prompt assembly and
    // the skill tool. They are NOT persisted — see `.plans/project-locals/plan.md`
    // decision D3 for the per-session isolation rationale.

    /// Returns the skills discovered for this session's cwd tree.
    pub fn discovered_skills(&self) -> &[crate::feat::skills::Skill] {
        &self.core.ephemeral.discovered_skills
    }

    /// Returns the prompt templates discovered for this session's cwd tree.
    pub fn discovered_prompt_templates(
        &self,
    ) -> &crate::feat::context::prompt_template::PromptTemplateStore {
        &self.core.ephemeral.discovered_prompt_templates
    }

    /// Returns the context files discovered for this session's cwd tree.
    pub fn discovered_context_files(&self) -> &[crate::feat::context::env_context::ContextFile] {
        &self.core.ephemeral.discovered_context_files
    }

    /// Replaces the discovered skills set for this session (scan-actor write path).
    pub fn set_discovered_skills(&mut self, skills: Vec<crate::feat::skills::Skill>) {
        self.core.ephemeral.discovered_skills = skills;
    }

    /// Replaces the discovered prompt-template store for this session.
    pub fn set_discovered_prompt_templates(
        &mut self,
        store: crate::feat::context::prompt_template::PromptTemplateStore,
    ) {
        self.core.ephemeral.discovered_prompt_templates = store;
    }

    /// Replaces the discovered context files for this session.
    pub fn set_discovered_context_files(
        &mut self,
        files: Vec<crate::feat::context::env_context::ContextFile>,
    ) {
        self.core.ephemeral.discovered_context_files = files;
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
    pub fn set_session_id(&mut self, id: SessionId) {
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

    /// Force-exclude any `ToolCall` entries that lack matching `ToolResult` entries,
    /// and their empty parent `Assistant` entry.
    ///
    /// Called after hard cancel (ESC) to ensure the assembled prompt doesn't
    /// contain dangling `tool_calls` without corresponding `tool` results,
    /// which causes LLM providers to reject the request (e.g., ZAI error 1214).
    ///
    /// Uses `ContextOverride::ForcedExclude` rather than removing entries,
    /// preserving them for display in the UI.
    pub fn force_exclude_dangling_tool_calls(&mut self) -> Vec<ChatEntryId> {
        // Collect tool_call_ids that have matching ToolResult entries.
        let result_ids: Vec<String> = self
            .core
            .history
            .iter()
            .filter_map(|entry| match &entry.kind {
                ChatEntryKind::ToolResult { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();

        // Find indices of dangling ToolCalls and their empty parent Assistants.
        let mut indices_to_exclude: Vec<usize> = Vec::new();
        for (i, entry) in self.core.history.iter().enumerate() {
            if let ChatEntryKind::ToolCall { id, .. } = &entry.kind
                && !result_ids.iter().any(|rid| rid == id)
            {
                indices_to_exclude.push(i);
                // Check if preceding entry is an empty Assistant.
                if i > 0
                    && let Some(prev) = self.core.history.get(i - 1)
                    && let ChatEntryKind::Assistant(text) = &prev.kind
                    && text.is_empty()
                {
                    indices_to_exclude.push(i - 1);
                }
            }
        }

        // Mark entries as ForcedExclude and collect the ids of those that actually changed.
        let mut changed = Vec::new();
        for idx in indices_to_exclude {
            let Some(entry) = self.core.history.get_mut(idx) else {
                continue;
            };
            let prev = entry.context_override();
            if prev != ContextOverride::ForcedExclude {
                entry.apply_context_override(
                    ContextOverride::ForcedExclude,
                    ChangeSource::Internal {
                        label: "dangling_tool_call_sweep".into(),
                    },
                );
                changed.push(entry.id.clone());
            }
        }
        changed
    }

    /// Disable the tool loop for this session's current turn.
    ///
    /// Delegates to [`SessionPhaseMachine::set_tool_loop_disabled`].
    pub fn set_tool_loop_disabled(&mut self) {
        self.core.ephemeral.machine.set_tool_loop_disabled();
    }

    /// Take the tool-loop-disabled flag, clearing it.
    ///
    /// Delegates to [`SessionPhaseMachine::take_tool_loop_disabled`].
    pub fn take_tool_loop_disabled(&mut self) -> bool {
        self.core.ephemeral.machine.take_tool_loop_disabled()
    }

    /// Check whether the tool loop is disabled, without clearing.
    ///
    /// Delegates to [`SessionPhaseMachine::is_tool_loop_disabled`].
    pub fn is_tool_loop_disabled(&self) -> bool {
        self.core.ephemeral.machine.is_tool_loop_disabled()
    }

    // --- History mutations (background workers) ---

    /// Resolve a [`ChatEntryId`] to its current index in history.
    ///
    /// Returns `None` if the entry no longer exists.
    pub fn find_entry_index_by_id(&self, id: &ChatEntryId) -> Option<usize> {
        self.core.history.iter().position(|e| e.id == *id)
    }

    /// Queue a batch of mutations for deferred application.
    ///
    /// Empty batches are silently ignored.
    pub fn queue_mutations(
        &mut self,
        batch: Vec<crate::feat::session::history_mutation::HistoryMutation>,
    ) {
        if !batch.is_empty() {
            self.core.ephemeral.pending_mutations.push(batch);
        }
    }

    /// Drain all pending mutation batches.
    pub fn drain_pending_mutations(
        &mut self,
    ) -> Vec<Vec<crate::feat::session::history_mutation::HistoryMutation>> {
        std::mem::take(&mut self.core.ephemeral.pending_mutations)
    }

    /// Apply a batch of mutations. Resolves IDs to current positions.
    ///
    /// Silently skips mutations targeting nonexistent entries.
    /// Processing order within a batch is preserved - earlier mutations
    /// are visible to later ones in the same batch.
    pub fn apply_mutations(
        &mut self,
        batch: Vec<crate::feat::session::history_mutation::HistoryMutation>,
    ) -> Vec<ChatEntryId> {
        use crate::feat::session::history_mutation::HistoryMutation;
        let mut changed = Vec::new();

        for mutation in batch {
            match mutation {
                HistoryMutation::SetContextOverride {
                    entry_id,
                    value,
                    source,
                } => {
                    if let Some(entry) = self.core.history.iter_mut().find(|e| e.id == entry_id) {
                        if entry.context_override() == ContextOverride::ForcedInclude
                            && value == ContextOverride::ForcedExclude
                        {
                            continue;
                        }
                        // Don't allow workers to re-include entries the user
                        // explicitly excluded.
                        if value == ContextOverride::ForcedInclude
                            && matches!(source, ChangeSource::Worker { .. })
                            && entry.is_user_force_excluded()
                        {
                            continue;
                        }
                        let was_changed = entry.apply_context_override(value, source);
                        if was_changed {
                            changed.push(entry_id);
                        }
                    }
                }
                HistoryMutation::InsertEntry {
                    after_entry_id,
                    entry,
                } => {
                    let insert_at = match after_entry_id {
                        Some(id) => match self.find_entry_index_by_id(&id) {
                            Some(idx) => idx + 1,
                            None => continue,
                        },
                        None => 0,
                    };
                    self.insert_entry_at(insert_at, entry);
                }
                HistoryMutation::PinEntry { entry_id, position } => {
                    self.pin_entry(&entry_id, position);
                }
                HistoryMutation::UnpinEntry { entry_id } => {
                    self.unpin_entry(&entry_id);
                }
            }
        }
        changed
    }

    /// Drain all pending mutation batches and apply them.
    ///
    /// Returns the number of batches applied and the entry IDs whose
    /// `context_override` actually changed value during this drain.
    pub fn drain_and_apply_pending_mutations(&mut self) -> (usize, Vec<ChatEntryId>) {
        let batches = self.drain_pending_mutations();
        let count = batches.len();
        let mut changed = Vec::new();
        for batch in batches {
            let mut batch_changed = self.apply_mutations(batch);
            changed.append(&mut batch_changed);
        }
        (count, changed)
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
#[must_use]
#[derive(Debug, Default)]
pub struct ChatSessionStateBuilder {
    ops: Vec<BuilderOp>,
}

#[cfg(test)]
#[derive(Debug)]
enum BuilderOp {
    PushEntry(Box<ChatEntry>),
    BeginStreaming,
    BeginSending,
    PinLast(PinPosition),
}

#[cfg(test)]
impl ChatSessionStateBuilder {
    /// Push a user entry onto the history.
    pub fn with_user_entry(mut self, text: &str) -> Self {
        self.ops
            .push(BuilderOp::PushEntry(Box::new(ChatEntry::user(text))));
        self
    }

    /// Push any entry onto the history.
    pub fn with_entry(mut self, entry: ChatEntry) -> Self {
        self.ops.push(BuilderOp::PushEntry(Box::new(entry)));
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
                    let entry = *entry;
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
