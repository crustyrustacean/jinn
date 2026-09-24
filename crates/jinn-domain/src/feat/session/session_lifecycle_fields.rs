//! Broad field groups used to compose [`SessionCore`](super::SessionCore).
//!
//! These groups keep the session's persistent and runtime state in cohesive
//! review units while the live values remain atomically owned by `SessionCore`.
//! Flattening every group preserves the established flat session schema.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use super::chat_session::{
    LifecycleScriptState, SessionOrigin, SessionState, default_cwd, default_persist,
};
use super::profile::SessionProfile;
use super::token_stats::TokenRecord;
use crate::protocol::{ChatHistory, SessionId};

/// Identity, tree metadata, and interaction metadata for one session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionIdentityMetadataFields {
    /// Unique identifier for this session.
    /// Generated at construction. Matches the HashMap key in `SessionState.sessions`.
    pub session_id: SessionId,
    /// When this session was last updated. Set at construction, updated on save.
    pub updated_at: Timestamp,
    /// Wall-clock timestamp of the most recent chat-history mutation
    /// (entry pushed, stream token appended, thinking token appended).
    /// The stall watchdog compares `now - last_history_activity_at` against
    /// the stall timeout to detect hung sessions. Seeded on phase entry
    /// (`begin_sending`/`begin_streaming`) so the HTTP-handshake gap is covered.
    /// Runtime-only turn state — not persisted (a loaded session is `Idle`).
    /// OWNER: session-actor.
    #[serde(skip)]
    pub last_history_activity_at: Timestamp,
    /// Wall-clock timestamp of the most recent **provider output** (assistant
    /// text, thinking text, or tool-call deltas from the model) — the genuine
    /// "the provider is responsive" signal. Unlike `last_history_activity_at`,
    /// this is bumped only by provider-output methods, not by retry markers or
    /// phase transitions. The stall watchdog resets a session's retry budget
    /// when this advances between ticks, so a responsive provider that suffers
    /// intermittent contention stalls is not prematurely cancelled.
    /// Runtime-only turn state — not persisted.
    /// OWNER: session-actor.
    #[serde(skip)]
    pub last_provider_activity_at: Timestamp,
    /// When this session was created. Set once at construction, never mutated.
    pub created_at: Timestamp,
    /// Human-readable title. `None` until the first user message is sent.
    /// OWNER: session-actor (set on first user message, changeable by user).
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Parent session ID, if this session was forked from another.
    /// `None` means this is a root session.
    /// OWNER: session-actor (set at session creation).
    #[serde(default)]
    pub parent_session: Option<SessionId>,
    /// Highest entry ordinal inherited from the parent at fork time.
    /// `None` for root sessions (all entries are "own").
    /// `Some(n)` means entries at indices 0..=n were inherited;
    /// only entries after index n count as turns for this session.
    /// Set once at fork creation, never mutated.
    /// OWNER: session-actor (set during fork).
    #[serde(default)]
    pub fork_ordinal: Option<usize>,
    /// Identity of this session's creation path. Set at construction by the
    /// creating path (`new_child` → [`SessionOrigin::Subagent`], fork →
    /// [`SessionOrigin::Fork`]); never mutated afterwards.
    /// OWNER: session-actor (set at session creation).
    #[serde(default)]
    pub origin: SessionOrigin,
    /// Project directory this session is associated with. Stamped once at
    /// session creation from the projects UI; never follows later `cwd`
    /// changes. `None` means the session has no project association.
    /// OWNER: IntentHandler (set at session creation).
    #[serde(default)]
    pub project: Option<PathBuf>,
    /// Whether the user has meaningfully interacted with this session.
    /// Sessions with `has_interacted = false` are not persisted to disk.
    /// OWNER: session-actor (set via MarkSessionInteracted command).
    #[serde(default)]
    pub has_interacted: bool,
}

impl Default for SessionIdentityMetadataFields {
    fn default() -> Self {
        Self {
            session_id: SessionId::new(),
            updated_at: Timestamp::now(),
            last_history_activity_at: Timestamp::now(),
            last_provider_activity_at: Timestamp::now(),
            created_at: Timestamp::now(),
            title: None,
            parent_session: None,
            fork_ordinal: None,
            origin: SessionOrigin::User,
            project: None,
            has_interacted: false,
        }
    }
}

/// Location and lifecycle state used to set up, run, and tear down a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionLifecycleLocationFields {
    /// Working directory for tool execution in this session.
    /// OWNER: IntentHandler (set on session creation and cd commands).
    #[serde(default = "default_cwd")]
    pub cwd: PathBuf,
    /// User home directory for resolving `@~/path` references in this session.
    /// Runtime-only — not persisted (resolved fresh at session creation from
    /// `services.paths.home_dir()`).
    /// OWNER: IntentHandler / session creation.
    #[serde(skip)]
    pub home: PathBuf,
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
    /// Lifecycle script progression - one-way: NothingRan → SetupRan → TeardownRan.
    /// OWNER: session-actor (advances only after script success).
    #[serde(default)]
    pub lifecycle_script_state: LifecycleScriptState,
}

impl Default for SessionLifecycleLocationFields {
    fn default() -> Self {
        Self {
            cwd: PathBuf::from("."),
            home: PathBuf::from("."),
            lifecycle_name: None,
            lifecycle_args: Vec::new(),
            lifecycle_script_state: LifecycleScriptState::NothingRan,
        }
    }
}

/// Conversation history, token accounting, and planning work for one session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionHistoryWorkFields {
    /// All messages in this conversation.
    ///
    /// OWNER: history-editor (the sole write path; reads via
    /// [`ChatSessionState::history`]). Restoring from persistence is the one
    /// exception, performed by [`ChatSessionState::restore_history`] before
    /// the session becomes live.
    pub(in crate::feat::session) history: ChatHistory,
    /// Token usage ledger - one immutable record per request/response pair.
    /// OWNER: session-actor (records tokens on assembly and StreamCompleted).
    #[serde(default)]
    pub token_ledger: Vec<TokenRecord>,
    /// Phased task list for agent session planning.
    /// OWNER: tools-actor.
    #[serde(default)]
    pub task_list: jinn_tools_msg::TaskList,
}

/// Provider configuration and per-session integration state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionIntegrationFields {
    /// Per-session model and strategy selection.
    /// OWNER: provider-actor (model), context-actor (strategy via SwitchPromptStrategy command)
    pub profile: SessionProfile,
    /// Generic blob storage for future subsystems.
    #[serde(default)]
    pub blobs: HashMap<String, JsonValue>,
    /// Names of MCP servers (`jinn.toml` `[[mcp_server]].name`) enabled for
    /// this session. Off by default — enabling spawns a dedicated `McpActor`
    /// and its child-process connection; disabling kills both. Persisted with
    /// the session.
    /// OWNER: IntentHandler (toggled via the MCP picker); McpCoordinatorActor
    /// reacts to the resulting commands.
    #[serde(default)]
    pub enabled_mcp_servers: BTreeSet<String>,
    /// Live connection status of each enabled MCP server, keyed by server
    /// name. Runtime-only (derived from `McpServerStatus` actor events); not
    /// persisted.
    /// OWNER: McpCoordinatorActor (writes on `McpServerStatus` events).
    #[serde(skip)]
    pub mcp_server_status: BTreeMap<String, jinn_mcp_msg::McpConnectionStatus>,
    /// Per-session captured stderr tail for each MCP server, updated live
    /// by the stderr-debounce republish.
    ///
    /// OWNER: McpCoordinatorActor (writes on `McpServerLog` events).
    #[serde(skip)]
    pub mcp_server_stderr: BTreeMap<String, String>,
}

/// Loaded/archived state and persistence policy for one session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStorageFields {
    /// Whether this session is loaded in memory or archived in the database.
    /// OWNER: session-actor (transitions on close/archive/unarchive).
    #[serde(default)]
    pub session_state: SessionState,
    /// Whether this session should be persisted to disk. Default true; set
    /// false for transient automated sessions (e.g. one-shots).
    /// OWNER: session-actor (set on creation).
    #[serde(default = "default_persist")]
    pub persist: bool,
}

impl Default for SessionStorageFields {
    fn default() -> Self {
        Self {
            session_state: SessionState::Loaded,
            persist: true,
        }
    }
}
