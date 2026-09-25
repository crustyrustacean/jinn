//! Cohesive durable field groups composed by [`SessionCore`](crate::SessionCore).
//!
//! Every group is flattened into `SessionCore`, preserving the established flat
//! persisted schema while keeping the live values under one authoritative lock.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use jiff::Timestamp;
use jinn_core_types::{ChatHistory, SessionId};
use jinn_mcp_msg::McpConnectionStatus;
use jinn_session_lifecycle_msg::LifecycleScriptState;
use jinn_session_msg::SessionOrigin;
use jinn_session_store_msg::SessionState;
use jinn_token_count_msg::TokenRecord;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

pub use jinn_core_types::SessionProfile;

/// Serde default for sessions that opt into persistence.
#[must_use]
pub fn default_persist() -> bool {
    true
}

/// Serde default for a working directory.
#[must_use]
pub fn default_cwd() -> PathBuf {
    PathBuf::from(".")
}

/// Identity, tree metadata, and runtime interaction state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionIdentityMetadataFields {
    /// Unique session identifier.
    pub session_id: SessionId,
    /// Last durable update timestamp.
    pub updated_at: Timestamp,
    /// Most recent local history mutation, used by the stall watchdog.
    #[serde(skip)]
    pub last_history_activity_at: Timestamp,
    /// Most recent provider output, used by the stall watchdog.
    #[serde(skip)]
    pub last_provider_activity_at: Timestamp,
    /// Creation timestamp.
    pub created_at: Timestamp,
    /// User-visible session title.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Parent session for forks and subagent lineage.
    #[serde(default)]
    pub parent_session: Option<SessionId>,
    /// Highest inherited entry ordinal after a fork.
    #[serde(default)]
    pub fork_ordinal: Option<usize>,
    /// Creation path.
    #[serde(default)]
    pub origin: SessionOrigin,
    /// Project association stamped at creation.
    #[serde(default)]
    pub project: Option<PathBuf>,
    /// Runtime persistence eligibility.
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

/// Location, lifecycle name, arguments, and script progression.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionLifecycleLocationFields {
    /// Working directory used by tools and scans.
    #[serde(default = "default_cwd")]
    pub cwd: PathBuf,
    /// Runtime home directory for attachment resolution.
    #[serde(skip)]
    pub home: PathBuf,
    /// Lifecycle selected at creation.
    #[serde(default)]
    pub lifecycle_name: Option<String>,
    /// Arguments replayed during teardown.
    #[serde(default)]
    pub lifecycle_args: Vec<String>,
    /// One-way lifecycle script progression.
    #[serde(default)]
    pub lifecycle_script_state: LifecycleScriptState,
}

impl Default for SessionLifecycleLocationFields {
    fn default() -> Self {
        Self {
            cwd: default_cwd(),
            home: PathBuf::from("."),
            lifecycle_name: None,
            lifecycle_args: Vec::new(),
            lifecycle_script_state: LifecycleScriptState::NothingRan,
        }
    }
}

/// History, token accounting, and task planning for one session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionHistoryWorkFields {
    /// Conversation history coordinated with active turn state.
    pub history: ChatHistory,
    /// Request/response token ledger.
    #[serde(default)]
    pub token_ledger: Vec<TokenRecord>,
    /// Durable task-plan state.
    #[serde(default)]
    pub task_list: jinn_tools_msg::TaskList,
}

/// Provider policy, durable integration configuration, and runtime MCP state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionIntegrationFields {
    /// Per-session provider and context strategy.
    pub profile: SessionProfile,
    /// Legacy serialized extension values retained for schema compatibility.
    #[serde(default)]
    pub blobs: HashMap<String, JsonValue>,
    /// Persisted MCP server enablement.
    #[serde(default)]
    pub enabled_mcp_servers: BTreeSet<String>,
    /// Runtime MCP connection status.
    #[serde(skip)]
    pub mcp_server_status: BTreeMap<String, McpConnectionStatus>,
    /// Runtime MCP stderr tail.
    #[serde(skip)]
    pub mcp_server_stderr: BTreeMap<String, String>,
}

/// Loaded/archived state and persistence policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStorageFields {
    /// Current storage state.
    #[serde(default)]
    pub session_state: SessionState,
    /// Whether this session participates in durable persistence.
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
