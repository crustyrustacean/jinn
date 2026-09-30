//! Coherent durable session snapshot exchanged between state and storage.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use jinn_attendant_msg::{
    AttendantBehavior, AttendantReport, AttendantTrigger, default_seed_template,
};
use jinn_core_types::{ChatEntry, ChatEntryKind, SessionId, SessionProfile};
use jinn_session_lifecycle_msg::LifecycleScriptState;
use jinn_session_msg::SessionOrigin;
use jinn_session_store_msg::SessionState;
use jinn_token_count_msg::TokenRecord;
use jinn_tools_msg::TaskList;

use crate::core::SessionCore;
use crate::fields::default_prep_mode;

/// Monotonic in-process revision for one authoritative session capture.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct SessionRevision(u64);

impl SessionRevision {
    /// Creates a revision from a monotonic counter value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the underlying counter value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl From<u64> for SessionRevision {
    fn from(value: u64) -> Self {
        Self::new(value)
    }
}

impl From<SessionRevision> for u64 {
    fn from(value: SessionRevision) -> Self {
        value.get()
    }
}

/// Flat durable metadata for one session revision.
///
/// Runtime-only clocks, phase state, input/UI state, and MCP connection state
/// are intentionally absent. `session_state` is derived from the authoritative
/// `sessions.archived` column when metadata is written, preserving the legacy
/// flat metadata shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshotMetadata {
    /// Unique identifier for this session.
    pub session_id: SessionId,
    /// Human-readable title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Last durable update timestamp.
    pub updated_at: Timestamp,
    /// Session creation timestamp.
    pub created_at: Timestamp,
    /// Provider and prompt policy for this session.
    pub profile: SessionProfile,
    /// Working directory captured at this revision.
    pub cwd: PathBuf,
    /// Parent session for forks and subagents.
    #[serde(default)]
    pub parent_session: Option<SessionId>,
    /// Highest inherited entry ordinal for a fork.
    #[serde(default)]
    pub fork_ordinal: Option<usize>,
    /// Creation path for this session.
    #[serde(default)]
    pub origin: SessionOrigin,
    /// Project association stamped at session creation.
    #[serde(default)]
    pub project: Option<PathBuf>,
    /// Compatibility storage for existing serialized extensions.
    #[serde(default)]
    pub blobs: HashMap<String, JsonValue>,
    /// Lifecycle selector captured at creation.
    #[serde(default)]
    pub lifecycle_name: Option<String>,
    /// Replayed lifecycle arguments.
    #[serde(default)]
    pub lifecycle_args: Vec<String>,
    /// One-way lifecycle script progress.
    #[serde(default)]
    pub lifecycle_script_state: LifecycleScriptState,
    /// Durable task list owned by tools at runtime.
    #[serde(default)]
    pub task_list: TaskList,
    /// Durable MCP enablement set.
    #[serde(default)]
    pub enabled_mcp_servers: BTreeSet<String>,
    /// Whether this session may be persisted.
    #[serde(default = "crate::fields::default_persist")]
    pub persist: bool,
    /// Loaded/archived state, reconstructed from the sessions table.
    #[serde(skip, default)]
    pub session_state: SessionState,
    /// What a run in an attendant sees of the conversation.
    #[serde(default)]
    pub behavior: AttendantBehavior,
    /// Whether the attendant is still being composed.
    ///
    /// Absent means composing, the state `N` creates an attendant in: a
    /// bool has no third value, so the default is the answer rather than a
    /// sentinel standing in for one.
    #[serde(default = "default_prep_mode")]
    pub prep_mode: bool,
    /// The condition that causes an automatic attendant re-run.
    #[serde(default)]
    pub trigger: AttendantTrigger,
    /// User-editable seed text used to inject the prior report.
    #[serde(default = "default_seed_template")]
    pub seed_template: String,
    /// The attendant's append-only report log.
    #[serde(default)]
    pub reports: Vec<AttendantReport>,
}

impl From<&SessionCore> for SessionSnapshotMetadata {
    fn from(core: &SessionCore) -> Self {
        Self {
            session_id: core.identity.session_id.clone(),
            title: core.identity.title.clone(),
            updated_at: core.identity.updated_at,
            created_at: core.identity.created_at,
            profile: core.integrations.profile.clone(),
            cwd: core.lifecycle.cwd.clone(),
            parent_session: core.identity.parent_session.clone(),
            fork_ordinal: core.identity.fork_ordinal,
            origin: core.identity.origin,
            project: core.identity.project.clone(),
            blobs: core.integrations.blobs.clone(),
            lifecycle_name: core.lifecycle.lifecycle_name.clone(),
            lifecycle_args: core.lifecycle.lifecycle_args.clone(),
            lifecycle_script_state: core.lifecycle.lifecycle_script_state,
            task_list: core.history_work.task_list.clone(),
            enabled_mcp_servers: core.integrations.enabled_mcp_servers.clone(),
            persist: core.storage.persist,
            session_state: core.storage.session_state,
            behavior: core.attendant.behavior,
            prep_mode: core.attendant.prep_mode,
            trigger: core.attendant.trigger,
            seed_template: core.attendant.seed_template.clone(),
            reports: core.attendant.reports.clone(),
        }
    }
}

impl From<SessionSnapshotMetadata> for SessionCore {
    fn from(metadata: SessionSnapshotMetadata) -> Self {
        let mut core = Self::default();
        core.identity.session_id = metadata.session_id;
        core.identity.title = metadata.title;
        core.identity.updated_at = metadata.updated_at;
        core.identity.created_at = metadata.created_at;
        core.integrations.profile = metadata.profile;
        core.lifecycle.cwd = metadata.cwd;
        core.identity.parent_session = metadata.parent_session;
        core.identity.fork_ordinal = metadata.fork_ordinal;
        core.identity.origin = metadata.origin;
        core.identity.project = metadata.project;
        core.integrations.blobs = metadata.blobs;
        core.lifecycle.lifecycle_name = metadata.lifecycle_name;
        core.lifecycle.lifecycle_args = metadata.lifecycle_args;
        core.lifecycle.lifecycle_script_state = metadata.lifecycle_script_state;
        core.history_work.task_list = metadata.task_list;
        core.integrations.enabled_mcp_servers = metadata.enabled_mcp_servers;
        core.storage.persist = metadata.persist;
        core.storage.session_state = metadata.session_state;
        core.attendant.behavior = metadata.behavior;
        core.attendant.prep_mode = metadata.prep_mode;
        core.attendant.trigger = metadata.trigger;
        core.attendant.seed_template = metadata.seed_template;
        core.attendant.reports = metadata.reports;
        core
    }
}

/// Complete durable state captured from one authoritative session revision.
///
/// History excludes transient UI hints because the store never persists them.
/// Attachments remain embedded in their `ChatEntry` values until the SQLite
/// adapter normalizes them into `entry_blobs` within the same transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    /// In-process monotonic capture revision.
    pub revision: SessionRevision,
    /// Flat durable session metadata.
    pub metadata: SessionSnapshotMetadata,
    /// Ordered persisted conversation entries, including attachments.
    pub entries: Vec<ChatEntry>,
    /// Ordered token-accounting ledger.
    pub token_ledger: Vec<TokenRecord>,
}

impl SessionSnapshot {
    /// Derives a durable child snapshot from this coherent source snapshot.
    ///
    /// The child inherits durable metadata and entries through the inclusive
    /// `at_ordinal` boundary. It receives fresh identity timestamps, fork
    /// identity, and the source id as its parent. The subagent task-tool
    /// suppression is removed while every other profile policy is preserved.
    #[must_use]
    pub fn forked_from(&self, new_session_id: SessionId, at_ordinal: usize) -> Self {
        let mut metadata = self.metadata.clone();
        metadata.session_id = new_session_id;
        metadata.updated_at = Timestamp::now();
        metadata.created_at = metadata.updated_at;
        metadata.parent_session = Some(self.metadata.session_id.clone());
        metadata.fork_ordinal = Some(at_ordinal);
        metadata.origin = SessionOrigin::Fork;
        metadata.session_state = SessionState::Loaded;
        metadata
            .profile
            .disabled_tools
            .remove(jinn_tools_msg::TASK_TOOL_NAME);

        Self {
            revision: SessionRevision::new(1),
            metadata,
            entries: self.entries.iter().take(at_ordinal + 1).cloned().collect(),
            // Token records are turn-level accounting and were not inherited by
            // historical forks. Keep that behavior while making the child a
            // complete snapshot with an explicit empty ledger.
            token_ledger: Vec::new(),
        }
    }

    /// Returns this snapshot's session identifier.
    #[must_use]
    pub fn session_id(&self) -> &SessionId {
        &self.metadata.session_id
    }

    /// Returns the snapshot title.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.metadata.title.as_deref()
    }

    /// Returns the snapshot working directory.
    #[must_use]
    pub fn cwd(&self) -> &std::path::Path {
        &self.metadata.cwd
    }

    /// Returns the snapshot parent session.
    #[must_use]
    pub fn parent_session(&self) -> &Option<SessionId> {
        &self.metadata.parent_session
    }

    /// Returns the highest inherited entry ordinal for a fork.
    #[must_use]
    pub const fn fork_ordinal(&self) -> Option<usize> {
        self.metadata.fork_ordinal
    }

    /// Returns the creation origin.
    #[must_use]
    pub const fn origin(&self) -> SessionOrigin {
        self.metadata.origin
    }

    /// Returns the selected model.
    #[must_use]
    pub const fn model_selection(&self) -> &jinn_core_types::ModelSelection {
        &self.metadata.profile.model
    }

    /// Returns the complete provider and prompt policy.
    #[must_use]
    pub const fn profile(&self) -> &SessionProfile {
        &self.metadata.profile
    }

    /// Returns the project association.
    #[must_use]
    pub fn project(&self) -> Option<&std::path::Path> {
        self.metadata.project.as_deref()
    }

    /// Returns the selected persona.
    #[must_use]
    pub fn persona_name(&self) -> &str {
        &self.metadata.profile.persona_name
    }

    /// Returns the lifecycle selector.
    #[must_use]
    pub fn lifecycle_name(&self) -> Option<&str> {
        self.metadata.lifecycle_name.as_deref()
    }

    /// Returns the lifecycle arguments.
    #[must_use]
    pub fn lifecycle_args(&self) -> &[String] {
        &self.metadata.lifecycle_args
    }

    /// Returns the lifecycle script progression.
    #[must_use]
    pub const fn lifecycle_script_state(&self) -> LifecycleScriptState {
        self.metadata.lifecycle_script_state
    }

    /// Returns whether this session is eligible for persistence.
    #[must_use]
    pub const fn persist(&self) -> bool {
        self.metadata.persist
    }

    /// Returns the loaded/archived state.
    #[must_use]
    pub const fn session_state(&self) -> SessionState {
        self.metadata.session_state
    }

    /// Returns the ordered persisted conversation entries.
    #[must_use]
    pub fn history(&self) -> &[ChatEntry] {
        &self.entries
    }

    /// Returns the ordered token-accounting ledger.
    #[must_use]
    pub fn token_ledger(&self) -> &[TokenRecord] {
        &self.token_ledger
    }

    /// Reconstructs a complete live session from this coherent snapshot.
    ///
    /// The returned session has fresh runtime-only state and is not visible to
    /// callers until their state container publishes it. Its capture counter
    /// starts fresh, so a caller that re-saves a restored session must have
    /// told storage what floor to resume above — see
    /// [`Self::restore_live_above`].
    #[must_use]
    pub fn restore_live(self) -> crate::chat_session::ChatSessionState {
        self.restore_live_above(SessionRevision::new(0))
    }

    /// Reconstructs a live session whose captures resume above `floor`.
    ///
    /// Storage refuses a write whose revision it has already accepted, and it
    /// keeps that record for the whole process run. A session rebuilt from
    /// storage therefore has to start its capture numbering above the last
    /// revision written for it, or its next save is refused as stale and its
    /// next archive fails outright.
    #[must_use]
    pub fn restore_live_above(
        self,
        floor: SessionRevision,
    ) -> crate::chat_session::ChatSessionState {
        let mut session = crate::chat_session::ChatSessionState::default();
        session.set_core(self.into_core_resuming_after(floor));
        session
    }

    /// Reconstructs a live session core from this coherent snapshot.
    #[must_use]
    pub fn into_core(self) -> SessionCore {
        self.into_core_resuming_after(SessionRevision::new(0))
    }

    /// Reconstructs a live session core whose captures resume above `floor`.
    #[must_use]
    pub fn into_core_resuming_after(self, floor: SessionRevision) -> SessionCore {
        let mut core = SessionCore::from(self.metadata);
        core.resume_captures_after(floor);
        core.restore_history(self.entries);
        core.restore_token_ledger(self.token_ledger);
        core
    }
}

impl From<(SessionRevision, &SessionCore)> for SessionSnapshot {
    fn from((revision, core): (SessionRevision, &SessionCore)) -> Self {
        let entries = core
            .history_work
            .history
            .iter()
            .filter(|entry| !matches!(entry.kind, ChatEntryKind::Transient(_)))
            .cloned()
            .collect();
        Self {
            revision,
            metadata: SessionSnapshotMetadata::from(core),
            entries,
            token_ledger: core.history_work.token_ledger.clone(),
        }
    }
}
