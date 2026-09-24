//! Owned field groups for composing [`SessionCore`](super::SessionCore).
//!
//! Each group keeps one session facet's fields together while the fields remain
//! stored on `SessionCore`. Flattening a group preserves the flat serialized
//! schema, so composing state does not change existing session snapshots.

use serde::{Deserialize, Serialize};

use super::chat_session::{LifecycleScriptState, SessionState};
use super::chat_session::{default_cwd, default_persist};

/// Session lifecycle state owned by the session-lifecycle facet.
///
/// The serialized representation remains flat when this type is flattened into
/// [`SessionCore`](super::SessionCore).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionLifecycleFields {
    /// Human-readable title. `None` until the first user message is sent.
    /// OWNER: session-actor (set on first user message, changeable by user).
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Working directory for tool execution in this session.
    /// OWNER: IntentHandler (set on session creation and cd commands).
    #[serde(default = "default_cwd")]
    pub cwd: std::path::PathBuf,
    /// User home directory for resolving `@~/path` references in this session.
    /// Runtime-only — not persisted (resolved fresh at session creation from
    /// `services.paths.home_dir()`).
    /// OWNER: IntentHandler / session creation.
    #[serde(skip)]
    pub home: std::path::PathBuf,
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

    /// Whether this session should be persisted to disk. Default true; set
    /// false for transient automated sessions (e.g. one-shots).
    /// OWNER: session-actor (set on creation).
    #[serde(default = "default_persist")]
    pub persist: bool,
}

impl Default for SessionLifecycleFields {
    fn default() -> Self {
        Self {
            title: None,
            cwd: std::path::PathBuf::from("."),
            home: std::path::PathBuf::from("."),
            lifecycle_name: None,
            lifecycle_args: Vec::new(),
            session_state: SessionState::Loaded,
            lifecycle_script_state: LifecycleScriptState::NothingRan,
            persist: true,
        }
    }
}

/// Conversation history and pin state owned by the session-history facet.
///
/// This scaffold is intentionally empty. Its future fields are `history`, pin
/// state, and per-entry override state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionHistoryFields;

/// MCP server selection and runtime connection state owned by the MCP facet.
///
/// This scaffold is intentionally empty. Its future fields are
/// `enabled_mcp_servers`, `mcp_server_status`, and `mcp_server_stderr`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionMcpFields;

/// Per-session model and strategy selection owned by the provider-selection facet.
///
/// This scaffold is intentionally empty. Its future field is `profile`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionProfileFields;
