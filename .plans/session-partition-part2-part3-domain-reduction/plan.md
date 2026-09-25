# Context-Rich Specification: Complete Session Partition and Domain-Crate Reduction

## Problem

`jinn-domain` is approximately 77k lines and still contains the complete live session domain implementation:

- `ChatSessionState` and `SessionCore`;
- the five broad flattened field groups;
- `SessionCoreEphemeral`;
- `SessionPersistenceActor` and its turn handlers;
- session creation, store/lifecycle coordination helpers;
- large frontend and session projection code.

The five broad groups implemented by `.plans/session-core-broad-groups-migration-refresh/plan.md` are correctly a mechanical extraction scaffold. They preserve the flat persistence schema and the `ChatSessionState` API, but they do not reduce domain ownership or move the session implementation into a feature owner. The old Part 2 handoff in `.plans/session-partition/handoff-to-part2.md` describes the intended next direction, but its field-home table and handler inventory are stale after the completed actor partition and broad-group work.

The migration must now complete both session partition parts as one large program:

1. **Part 2:** move the live session state and portable vocabulary behind a neutral state boundary, make storage snapshot-based, and migrate consumers.
2. **Part 3:** move the coordinated turn reducer out of the kernel as one cohesive session-turn owner rather than distributing individual handlers that must mutate the same turn state atomically.

The migration must preserve the current guarantees:

- one authoritative live session aggregate for atomic turn state;
- one coherent snapshot per session revision;
- metadata, history, attachments, and token-ledger writes in one SQLite transaction;
- fully initialized sessions before load completion is published;
- one-owner semantics for asynchronous session operations;
- one `trouper::ActorSystem` and the existing `BusService` fabric;
- legacy flat metadata compatibility.

## Solution

Create a neutral non-slice state crate and a cohesive session-turn slice:

```text
jinn-domain
  shared State/AppState/capabilities/services
  IntentHandler and cross-slice kernel seams
  frontend-only shared projections

jinn-session-state
  authoritative live session aggregate
  SessionCore, broad groups, SessionCoreEphemeral
  SessionSnapshot and reconstruction contracts
  revision/capture boundary

jinn-session-turn
  SessionPersistenceActor
  enqueue/streaming/tool/retry/context folds
  coordinated turn persistence

jinn-session-store
  SessionStoreActor
  snapshot-based SQLite persistence
  load/fork/archive/search

jinn-session-lifecycle
  SessionLifecycleActor
  setup/teardown/close/cwd operations
```

The live session state is not scattered across independent typed cells. A single authoritative aggregate remains the source of truth for fields that participate in one turn fold. The state layer provides a coherent capture/reconstruction boundary. Non-atomic runtime projections, including MCP status/stderr, may use existing typed cells when they are excluded from the session snapshot and persisted transaction.

The final `jinn-domain` retains shared application state, capabilities, service seams, synchronous IntentHandler orchestration, and other genuinely cross-slice behavior. It no longer owns the complete session implementation or the turn actor handlers.

The existing SQLite metadata JSON remains flat and backward-compatible. The persistence API changes from accepting a live `ChatSessionState` to accepting a complete `SessionSnapshot` containing durable metadata, history, token ledger, task data, and a revision. SQLite continues to write all durable session data in one transaction.

The typed-cell read-only/write-capable API is a separate later improvement. This program does not split session atomic state into independent cells and does not implement `CellReader`/`CellWriter`.

---

# Dialectical Outcomes (Why)

- **The five broad groups were a preparation stage, not the final destination.** The completed broad-groups plan explicitly kept all live values in `SessionCore`, preserving atomic capture and avoiding duplicate state. This plan consumes that scaffold by moving the complete session implementation to a neutral state layer, not by deleting the grouping structure without a replacement.

- **The historical Part 2 handoff is retained as design intent but re-derived against the current tree.** It named lifecycle, history, store, and vocabulary destinations, but predates the five broad groups. Current groups are mixed: lifecycle contains `cwd`/`home`; history-work contains history, token accounting, and task state; integrations contains profile policy, MCP configuration, MCP runtime state, and `blobs`. The implementation follows current writer/reader evidence rather than copying the old table.

- **A neutral state crate is required.** Moving `ChatSessionState` or `SessionCore` directly into an actor-bearing slice would invert dependency direction and create cycles. `jinn-session-store` already depends on `jinn-domain`; `jinn-domain` cannot depend back on the store. A neutral `jinn-session-state` crate can own live state and snapshots while implementation slices depend on it.

- **A cohesive session-turn slice is required for Part 3.** The old Part 3 inventory described streaming, tool calls, enqueue, and retry as independently movable. Current code shows that they coordinate history, phase, generation guards, pending mutations, queues, input, model resolution, and persistence. Moving them separately would split one lock/version boundary. They move together as one turn reducer.

- **Atomic state is not the same as cross-cell atomicity.** A future read-only/write-capable typed-cell API would improve authority and prevent readers from calling `update`, but it would not make separate cells transactionally consistent. Session state therefore remains behind one authoritative aggregate/coordinator. Typed-cell authority hardening is explicitly deferred.

- **A complete `SessionSnapshot` is mandatory once live state leaves the persistence aggregate.** An identity/timestamp-only `SessionCore` cannot represent profile, lifecycle, history, task, MCP configuration, or token data. The snapshot is the portable durable reconstruction payload; the live aggregate is the runtime source of truth.

- **The store service remains a shared DI seam.** `SessionStore` and `SessionStoreService` remain in the shared kernel/services layer so both the turn path and store actor can use the same abstraction. The SQLite implementation and `SessionStoreActor` remain in `jinn-session-store`; the trait no longer depends on `ChatSessionState`.

- **SQLite transaction atomicity must be paired with coherent live snapshot capture.** A database transaction cannot repair a torn in-memory snapshot. The state layer captures one revision before persistence; the store writes that complete snapshot in one transaction.

- **The existing Record is intentionally amended at the end.** The current entry stating that `SessionCore` remains the kernel-owned atomic persistence unit is true today but false for the target. The implementation writes replacement facts only after verifying the final code.

- **The migration is a large, measurable program, not a symbolic file move.** Completion requires removing the session aggregate and turn implementation from `jinn-domain`, eliminating direct live session imports from production slices, and publishing before/after production LOC evidence.

---

# Relevant Files (Where)

## Existing files to read and modify

### Kernel session state and shared state

- `crates/jinn-domain/src/feat/session/chat_session.rs`
  - Current `ChatSessionState`, `SessionCore`, constructors, accessors, restore methods, and compatibility exports.
  - Must be dismantled or reduced to compatibility forwarding after the state crate owns the live types.
- `crates/jinn-domain/src/feat/session/session_lifecycle_fields.rs`
  - Current five broad groups: `SessionIdentityMetadataFields`, `SessionLifecycleLocationFields`, `SessionHistoryWorkFields`, `SessionIntegrationFields`, `SessionStorageFields`.
  - Move the aggregate definitions to `jinn-session-state` or replace them with forwarding compatibility exports.
- `crates/jinn-domain/src/feat/session/session_actor.rs`
  - Current `SessionPersistenceActor`, actor path, subscription chain, dependency struct, and spawn function.
  - Move the production implementation to `jinn-session-turn`.
- `crates/jinn-domain/src/feat/session/session_actor/handlers/enqueue.rs`
  - Coordinated user/turn intake fold; moves to `jinn-session-turn`.
- `crates/jinn-domain/src/feat/session/session_actor/handlers/streaming.rs`
  - Stream token/completion fold with history, phase, guards, pending batches, and persistence; moves as part of the turn reducer.
- `crates/jinn-domain/src/feat/session/session_actor/handlers/tool_calls.rs`
  - Tool-call and continuation fold; moves as part of the turn reducer.
- `crates/jinn-domain/src/feat/session/session_actor/handlers/stall_retry.rs`
  - Retry fold coordinated with stream guards, history, and redispatch; moves as part of the turn reducer.
- `crates/jinn-domain/src/feat/session/session_actor/handlers/context.rs`
  - Mixed context/history/frontend handler; split only where clean, with atomic portions remaining in the turn reducer.
- `crates/jinn-domain/src/feat/session/session_actor/handlers/misc.rs`
  - Model refresh, skills refresh, history mutation application, and pending-mutation coordination; split only at clean boundaries.
- `crates/jinn-domain/src/feat/session/session_actor/handlers/persistence.rs`
  - Direct turn-path snapshot save and interaction marking; moves to the turn slice or becomes a state service call.
- `crates/jinn-domain/src/feat/session/session_actor/handlers.rs`
  - Handler module declarations; update after moves.
- `crates/jinn-domain/src/feat/session/session_actor/helpers.rs`
  - Actor test and construction helpers; update imports and any kernel-only fixtures.
- `crates/jinn-domain/src/feat/session/session_store.rs`
  - Current `SessionStore` trait; replace `ChatSessionState` signatures with `SessionSnapshot`.
- `crates/jinn-domain/src/feat/session/session_store/service.rs`
  - Current service wrapper; update to snapshot-based methods and shared state types.
- `crates/jinn-domain/src/common/session_map.rs`
  - Current `SessionMap` stores `HashMap<SessionId, ChatSessionState>` and owns active-session invariants; move or wrap the session map in the state crate while preserving the active-session API.
- `crates/jinn-domain/src/common/state.rs`
  - Current global `State` lock; preserve as the shared application spine or introduce a state-layer session registry behind it.
- `crates/jinn-domain/src/common/tcaps/session.rs`
  - Current broad `SessionCap` projection; replace direct broad mutable reach-throughs with state-layer operations as consumers migrate.
- `crates/jinn-domain/src/common/app_state.rs`
  - Current `AppState` contains `SessionState` (`SessionMap`) and frontend state; adjust the session field’s type/import without changing frontend invariants.
- `crates/jinn-domain/src/feat/intent/handler.rs`
  - Synchronous frontend orchestration; update session state imports and call sites, but preserve synchronous behavior where the TUI requires it.
- `crates/jinn-domain/src/feat/session/sessions_list.rs`
  - Remove after moving the sessions-list library and sidebar adapter.
- `crates/jinn-domain/src/feat/session/sessions_list/*.rs`
  - Move pure projection/tree logic to `jinn-session-list`; move `AppState`/frontend/sidebar-cell reconciliation to `jinn-sidebar`; IntentHandler retains only routing.
- `crates/jinn-domain/src/feat/session/session_summary.rs`
  - Move `SessionSummary` to `jinn-session-store-msg`.
- `crates/jinn-domain/src/feat/session/tree_aggregate.rs`
  - `FrozenTreeNode` and aggregate value types; move pure projection values to `jinn-session-store-msg` or an appropriate lower-level state contract.
- `crates/jinn-domain/src/feat/session/token_stats.rs`
  - `TokenRecord` and `TokenStats`; move to `jinn-token-count-msg`.
- `crates/jinn-domain/src/feat/session/phase_machine.rs` and `phase_machine/`
  - Pure phase value/state vocabulary; move to `jinn-session-msg` where dependency-safe, retaining kernel compatibility exports during migration.

### Existing feature/session support

- `crates/jinn-domain/src/feat/session_lifecycle/`
  - Intent-facing lifecycle code remains kernel during Part 2 unless it can consume the new state layer without dependency inversion.
  - Contract re-exports move to `jinn-session-lifecycle-msg`.
- `crates/jinn-domain/src/feat/context/snapshot.rs`
  - Converts live session state into `AssemblyInputs`; update to consume the state-layer read facade/snapshot.
- `crates/jinn-domain/src/feat/context/`
  - `PromptTemplateStore`, `PathResolveContext`, `PendingPath`, `ContextFile`, expansion/loading helpers, and their tests move to the required neutral `jinn-context` crate before `ChatSessionState` moves.
  - Assembly-only helpers such as tool-prompt formatting remain with `jinn-context-assembly`; they are not required by the live aggregate unless compiler tracing shows a direct state dependency.
- `crates/jinn-domain/src/feat/ui/frontend_state.rs`
  - Current slice-cell readers and frontend session integration; migrate session-specific references without moving unrelated frontend code.
- `crates/jinn-domain/src/feat/ui/picker_states.rs`
  - Session picker entry types and frontend state; import session/store values from `jinn-session-msg` and `jinn-session-store-msg` after Phase 3.
- `crates/jinn-domain/src/feat/chat_input/`
  - Chat input state is already in `jinn-chat-input-msg`; flip the remaining shim imports as a cleanup, but do not move the entire chat-input feature in this session program.
- `crates/jinn-domain/src/feat/skills/`
  - UI-bound skill picker surface remains kernel for now; portable skill vocabulary is already in `jinn-skills`/`jinn-skills-msg`.

### Existing slices and contracts

- `crates/slices/jinn-session-store/src/sqlite.rs`
  - `SqliteSessionStore`, `PersistableCore`, `From<&SessionCore>`, reconstruction, and transaction code; convert to `SessionSnapshot` and coherent read snapshots.
- `crates/slices/jinn-session-store/src/sqlite_tests.rs`
  - Existing metadata, legacy, fork, archive, and search regression tests; add snapshot/transaction/read-consistency tests.
- `crates/slices/jinn-session-store/src/session_store_actor.rs`
  - `SessionStoreActor`; update to snapshot reconstruction and explicit archive/fork flows.
- `crates/slices/jinn-session-store/src/session_store_actor/handlers/{load,archive,persistence,picker,startup}.rs`
  - Migrate live session reconstruction and archive/persist paths.
- `crates/slices/jinn-session-lifecycle/src/session_lifecycle_actor.rs`
  - `SessionLifecycleActor`; update state access to the new state-layer facade.
- `crates/slices/jinn-session-lifecycle/src/session_lifecycle_actor/handlers/{setup,teardown,close,cwd}.rs`
  - Update lifecycle reads/writes and snapshot publication.
- `crates/slices/jinn-session-lifecycle-msg/src/`
  - Add/move `CloseSession`, `TeardownSessionTree`, and any remaining lifecycle commands/events.
- `crates/slices/jinn-session-store-msg/src/`
  - Add `SessionSummary`, `FrozenTreeNode`, store commands, and ID-only `SessionLoadCompleted`; migrate all subscribers in the same phase.
- `crates/jinn-session-msg/src/`
  - Add phase-machine values and shared `SessionClosed`, `MarkSessionInteracted`, `UserInteracted`, and other session events that have no existing owner.
- `crates/slices/jinn-session-history-msg/src/`
  - Existing history message family; preserve as the single declaration home.
- `crates/slices/jinn-token-count-msg/src/`
  - Add `TokenRecord`, `TokenStats`, and `AggregatedTokenStats` plus pure ledger aggregation.
- `crates/slices/jinn-mcp-slice/src/coordinator.rs`
  - Move runtime MCP status/stderr from `SessionCore` to an MCP-owned live cell; retain durable enabled-server configuration in the session snapshot.
- `crates/slices/jinn-sidebar/src/sections/sessions*`
  - Migrate sessions-list/state access and canonical reconciliation.
- `crates/slices/jinn-tools/src/{task.rs,orchestrator.rs}`
  - Replace direct `ChatSessionState` construction/reads with child-session contracts, read projections, and MCP status access.
- `crates/slices/jinn-discord/src/backend/gateway.rs`
  - Replace direct live-state reads with the session read facade/snapshot projection.
- `crates/slices/jinn-token-count/src/count_actor.rs`
  - Read token/history projections instead of the live concrete aggregate.
- `crates/slices/jinn-context-assembly/src/{assemble.rs,size_actor.rs}`
  - Consume the state-layer assembly input builder; do not read the old live kernel type.
- `crates/jinn-picker-specs/src/`
  - Flip session store/lifecycle/phase/profile imports to lower-level contract crates where available.
- `src/actor_wiring.rs`
  - Replace the kernel `SessionPersistenceActor::spawn` call with `jinn_session_turn::activate(...)`; retain the existing store/lifecycle activation order and readiness guarantees.

## New files to create

- `crates/jinn-context/Cargo.toml`
- `crates/jinn-context/src/lib.rs`
- `crates/jinn-context/src/prompt_template.rs` — canonical `PromptTemplate` noun plus store/lookup API.
- `crates/jinn-context/src/prompt_template/{attachment_path,expand,loader,store}.rs`
- `crates/jinn-context/src/env_context.rs`
- `crates/jinn-context/src/frontmatter.rs` — move the existing `+++` TOML frontmatter parser from `jinn-domain`; the separate YAML skill parser remains in `jinn-skills`.
- `crates/jinn-session-state/Cargo.toml`
- `crates/jinn-session-state/src/lib.rs`
- `crates/jinn-session-state/src/chat_session.rs`
- `crates/jinn-session-state/src/core.rs`
- `crates/jinn-session-state/src/fields.rs`
- `crates/jinn-session-state/src/snapshot.rs`
- `crates/jinn-session-state/src/session_map.rs`
- `crates/jinn-session-list/Cargo.toml`
- `crates/jinn-session-list/src/lib.rs`
- `crates/jinn-session-list/src/{entry,tree,visual_parent}.rs` — owner-neutral loaded-session projection and visible-tree algorithms; it defines no cell because `SessionsSectionState` already lives in `jinn-sidebar-msg`.
- `crates/slices/jinn-mcp-msg/src/runtime_state.rs` — `McpRuntimeState { status_by_server, stderr_by_server }` plus the shared slot key; the implementation crate and all readers use this type.
- `crates/slices/jinn-session-turn/Cargo.toml`
- `crates/slices/jinn-session-turn/src/lib.rs`
- `crates/slices/jinn-session-turn/src/session_actor.rs`
- `crates/slices/jinn-session-turn/src/session_actor/handlers/enqueue.rs`
- `crates/slices/jinn-session-turn/src/session_actor/handlers/streaming.rs`
- `crates/slices/jinn-session-turn/src/session_actor/handlers/tool_calls.rs`
- `crates/slices/jinn-session-turn/src/session_actor/handlers/stall_retry.rs`
- `crates/slices/jinn-session-turn/src/session_actor/handlers/context.rs`
- `crates/slices/jinn-session-turn/src/session_actor/handlers/misc.rs`
- `crates/slices/jinn-session-turn/src/session_actor/handlers/persistence.rs`
- `crates/slices/jinn-session-turn/src/session_actor/helpers.rs`
- `crates/slices/jinn-session-turn/src/session_actor/handlers.rs`
- `crates/slices/jinn-session-turn/src/session_actor_tests.rs` — focused turn-reducer tests migrated from the kernel actor test modules.
- `crates/slices/jinn-session-store-msg/src/session_snapshot.rs` is not created; `SessionSnapshot` belongs to `jinn-session-state` and is not serialized as a nested message.
- `tests/slices/session_partition.rs` — cross-crate composition, load visibility, fork/archive, and turn behavior.

## Documentation to update at the end

- `/mnt/zed/repos/jinn/actor-migration/migration.md`
- `/mnt/zed/repos/jinn/actor-migration/slices.md`
- `/mnt/zed/repos/jinn/actor-migration/catalog.md`
- `/mnt/zed/repos/jinn/actor-migration/cleanup.md`
- `.agents/RECORD.md`
- `.plans/session-partition/handoff-to-part2.md` remains historical and is not modified.
- `.plans/session-core-broad-groups-migration-refresh/plan.md` remains historical; do not rewrite it.
- `/mnt/zed/repos/jinn/actor-migration/post-cleanup.md` is not modified.
- `/mnt/zed/repos/jinn/actor-migration/raw/**` is not modified.

---

# Key Code Context (What)

## Current `SessionCore`

`crates/jinn-domain/src/feat/session/chat_session.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCore {
    #[serde(flatten)]
    pub identity: SessionIdentityMetadataFields,
    #[serde(flatten)]
    pub lifecycle: SessionLifecycleLocationFields,
    #[serde(flatten)]
    pub history_work: SessionHistoryWorkFields,
    #[serde(flatten)]
    pub integrations: SessionIntegrationFields,
    #[serde(flatten)]
    pub storage: SessionStorageFields,
    #[serde(skip)]
    pub ephemeral: SessionCoreEphemeral,
}
```

Every group is flattened, so the persisted metadata JSON remains flat. The broad groups are not independent live state owners today.

## Current `SessionMap` and `State`

`crates/jinn-domain/src/common/session_map.rs`:

```rust
pub struct SessionMap {
    sessions: HashMap<SessionId, ChatSessionState>,
    frozen_nodes: HashMap<SessionId, FrozenTreeNode>,
    active_session: SessionId,
    session_load_guard: Option<SessionLoadGuard>,
    default_cwd: PathBuf,
    slices: std::sync::OnceLock<jinn_slices::Slices>,
}
```

`crates/jinn-domain/src/common/state.rs`:

```rust
pub struct State {
    inner: Arc<RwLock<AppState>>,
}
```

The current global state lock is the reason the live `ChatSessionState` can be cloned as one coherent snapshot. The new state layer must preserve an equivalent single capture boundary even if the global `State` remains in `jinn-domain`.

## Current store seam

`crates/jinn-domain/src/feat/session/session_store.rs`:

```rust
#[async_trait]
pub trait SessionStore: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    async fn save(&self, session: &ChatSessionState)
        -> Result<(), Report<SessionStoreError>>;

    async fn load_summaries(&self)
        -> Result<Vec<SessionSummary>, Report<SessionStoreError>>;

    async fn load_session(&self, session_id: &SessionId)
        -> Result<Option<ChatSessionState>, Report<SessionStoreError>>;

    async fn fork(&self, source_session_id: &SessionId, at_ordinal: usize)
        -> Result<SessionId, Report<SessionStoreError>>;
}
```

The final trait must accept and return `SessionSnapshot`/`SessionSummary` lower-level values, not `ChatSessionState`. The service wrapper remains in the shared DI layer and is updated in parallel with the trait.

## Current actor topology

`src/actor_wiring.rs` currently starts:

```rust
jinn_session_store::activate(&services, state.clone());
jinn_session_lifecycle::activate(
    &services,
    state.clone(),
    jinn_session_lifecycle_msg::BuiltinRegistry::new(),
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned()),
);
let _session = jinn_domain::feat::session::session_actor::SessionPersistenceActor::spawn(
    &services.trouper_system,
    jinn_domain::feat::session::session_actor::SessionPersistenceActorDeps {
        deps: actor_deps.clone(),
        state: state.clone(),
        cap: jinn_domain::common::tcaps::mint::mint_session_cap(),
        frontend_cap: jinn_domain::common::tcaps::mint::mint_frontend_cap(),
        counter: token_counter,
        token_cache: entry_token_cache.clone(),
        image_converter: jinn_domain::feat::image_convert::ImageConverterService::system(),
    },
);
```

The final wiring starts the session-turn slice activation at the same point where the kernel actor starts, preserving subscription order and startup readiness.

## Current broad group definitions

`crates/jinn-domain/src/feat/session/session_lifecycle_fields.rs`:

```rust
pub struct SessionIdentityMetadataFields {
    pub session_id: SessionId,
    pub updated_at: Timestamp,
    #[serde(skip)]
    pub last_history_activity_at: Timestamp,
    #[serde(skip)]
    pub last_provider_activity_at: Timestamp,
    pub created_at: Timestamp,
    pub title: Option<String>,
    pub parent_session: Option<SessionId>,
    pub fork_ordinal: Option<usize>,
    pub origin: SessionOrigin,
    pub project: Option<PathBuf>,
    pub has_interacted: bool,
}

pub struct SessionLifecycleLocationFields {
    #[serde(default = "default_cwd")]
    pub cwd: PathBuf,
    #[serde(skip)]
    pub home: PathBuf,
    pub lifecycle_name: Option<String>,
    pub lifecycle_args: Vec<String>,
    pub lifecycle_script_state: LifecycleScriptState,
}

pub struct SessionHistoryWorkFields {
    pub history: ChatHistory,
    pub token_ledger: Vec<TokenRecord>,
    pub task_list: jinn_tools_msg::TaskList,
}

pub struct SessionIntegrationFields {
    pub profile: SessionProfile,
    pub blobs: HashMap<String, JsonValue>,
    pub enabled_mcp_servers: BTreeSet<String>,
    #[serde(skip)]
    pub mcp_server_status: BTreeMap<String, McpConnectionStatus>,
    #[serde(skip)]
    pub mcp_server_stderr: BTreeMap<String, String>,
}

pub struct SessionStorageFields {
    pub session_state: SessionState,
    #[serde(default = "default_persist")]
    pub persist: bool,
}
```

The exact field attributes must be carried into the state crate or re-expressed in the snapshot DTO with equivalent serde behavior. The implementation must not silently nest the five groups.

## Current persistence shape

`crates/slices/jinn-session-store/src/sqlite.rs` currently has a flat `PersistableCore` conversion. The new `SessionSnapshot` must preserve the same serialized field names and defaults:

```rust
#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub revision: SessionRevision,
    pub metadata: SessionSnapshotMetadata,
    pub entries: Vec<ChatEntry>,
    pub token_ledger: Vec<TokenRecord>,
    pub session_state: SessionState,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionRevision(u64);
```

`SessionSnapshot` is an in-process capture payload and is not itself serialized as a nested blob. `SessionSnapshotMetadata` preserves the existing flat JSON stored in `sessions.metadata`; `entries` and `token_ledger` map to the existing normalized tables; `session_state` maps to `sessions.archived`. Runtime-only phase, streaming, pending mutation, queue, busy, and MCP runtime status/stderr data must not enter the snapshot.

## Current typed-cell limitation

`crates/jinn-slices/src/cell.rs`:

```rust
pub struct TypedCell<T> {
    inner: Arc<RwLock<T>>,
}

impl<T> TypedCell<T> {
    pub fn update<F>(&self, f: F)
    where
        F: FnOnce(&mut T);

    pub fn read(&self) -> RwLockReadGuard<'_, T>;
}
```

`crates/jinn-slices/src/slices.rs`:

```rust
pub fn register<T>(&self, key: SlotKey, initial: T)
    -> Result<TypedCell<T>, SlotTaken>;

pub fn reader<T>(&self, key: &SlotKey)
    -> Option<TypedCell<T>>;
```

Both handles expose `update`. This plan does not change that API. MCP runtime state may use a cell because it is independently live and excluded from the session snapshot. The session aggregate itself must not be reconstructed from a collection of independent cells.

---

# Implementation Algorithm (How)

## Phase 1 — Freeze the target ownership and migration inventory

1. Create a complete current-state inventory from source, not from stale plan counts:
   - production field definitions and serde attributes;
   - production writers and readers for every field;
   - current direct `ChatSessionState` and `SessionCore` consumers;
   - current actor subscription and spawn topology;
   - current handler module dependencies;
   - current dependency graph and cycles;
   - current test targets.
2. Classify the five groups into actual end-state facets without changing runtime:
   - identity/tree metadata;
   - presentation/list metadata;
   - runtime activity metadata;
   - lifecycle execution metadata;
   - location/path environment;
   - history;
   - token accounting;
   - task planning;
   - profile/provider policy;
   - durable MCP configuration;
   - runtime MCP status/stderr;
   - storage policy.
3. Move, from actual writer/reader evidence, no runtime field out of the authoritative aggregate in this inventory phase. Record which runtime projections are eligible for an independent cell; MCP status/stderr is the approved projection.
4. Record the compatibility decisions for currently stale fields without changing runtime behavior:
   - retain `last_history_activity_at` and `last_provider_activity_at` as runtime-only compatibility telemetry and remove their inaccurate watchdog-owner documentation;
   - retain `blobs` as serialized unowned compatibility storage;
   - retain `has_interacted` as runtime-only persistence eligibility and exclude it from `SessionSnapshotMetadata`.
5. Define the final producer/consumer matrix before moving code.
6. Define the production LOC baseline for `jinn-domain` using production source excluding tests, plus a total source count. Record the baseline in the implementation summary.

Gate: `just check`; inventory artifacts and dependency audit complete.

## Phase 2 — Create the neutral context and session-state crates

1. Add `jinn-context` before attempting the state move. Move the kernel-owned context model and pure parsing/loading/expansion behavior there:
   - move `PromptTemplate` to `jinn-context`; make `jinn-session-init-msg` re-export it so message schema IDs and existing imports stay stable; move the full `PromptTemplateStore` API without defining a duplicate.
   - `PathResolveContext`, `PendingPath`, `scan_at_paths`, and degraded attachment scanning.
   - prompt expansion, loading, matching, and store errors.
   - `ContextFile` and its loaders; `Persona` is imported from `jinn-persona-msg`.
   - frontmatter parsing, moving the existing `+++` TOML parser from `jinn-domain`; the YAML skill parser remains in `jinn-skills`.
2. Add `jinn-context` to the workspace and flip session-init, session-state preparation, chat input, Discord, and context-assembly callers to it. Kernel compatibility re-exports remain during the move.
3. Promote the leaf values required by the live aggregate:
   - move `Endpoint` into `jinn-core-types`, preserving the provider-selection compatibility re-export;
   - move the `SessionProfile` data type, serde defaults, constructors, and pure methods into `jinn-core-types`, leaving `SessionSeed::from_preferences` in a higher layer;
   - move phase-machine values/state types into `jinn-session-msg`;
   - move `SessionSummary` and `FrozenTreeNode` data types into `jinn-session-store-msg`;
   - move `TokenRecord`, `TokenStats`, and `AggregatedTokenStats` plus pure ledger aggregation into `jinn-token-count-msg`.
4. Add `crates/jinn-session-state` as a workspace member and workspace dependency.
5. Keep it below actor-bearing slices and below `jinn-domain`. It may depend on:
   - `jinn-core-types`;
   - `jinn-context`;
   - `jinn-persona-msg`;
   - `jinn-skills`;
   - `jinn-session-msg`;
   - `jinn-session-lifecycle-msg`;
   - `jinn-session-store-msg`;
   - `jinn-session-history-msg`;
   - `jinn-token-count-msg`;
   - `jinn-tools-msg`;
   - `jinn-slices`;
   - `jinn-chat-log-view-msg` and `jinn-chat-input-msg` for existing view/input facades;
   - existing `jinn-session-history` for the history editor implementation seam.
6. It must not depend on actor-bearing `jinn-session-store`, `jinn-session-lifecycle`, or a new `jinn-session-turn` implementation.
7. Move the live session types and behavior:
   - `ChatSessionState`;
   - `SessionCore`;
   - `SessionCoreEphemeral`;
   - `SessionUi`;
   - five group definitions;
   - phase machine, mutation accumulator, and steering buffer;
   - constructors and semantic accessors;
   - history restore methods;
   - profile/session lifecycle semantic accessors.
8. Preserve the existing `ChatSessionState` public method signatures during the move. Pure visual-item computation/resolution helpers move from `feat/ui/chat_log/visual_item.rs` into `jinn-chat-log-view-msg`; the `VisualItem` noun already lives there.
9. Retain `last_history_activity_at`, `last_provider_activity_at`, and `blobs` for API/schema compatibility in this migration, but correct their documentation. The clocks remain runtime-only compatibility telemetry; the watchdog continues using its actor-local state. `blobs` remains serialized compatibility storage with no assigned semantic owner. Removal is deferred to a separate cleanup with migration coverage.
10. Keep `has_interacted` runtime-only and out of the durable snapshot. It controls whether a live session may be written, and loaded sessions initialize it to the current effective value.
11. Move the active-session map/invariants to the state crate.
12. Keep the global `jinn-domain::State` and `AppState` as the shared application spine. The state crate provides coherent access services that the kernel and slices can use without a kernel-to-implementation-slice dependency.
13. Update `jinn-domain` compatibility re-exports so existing imports compile during the transition.
14. Run focused tests and verify no consumer behavior changed.

Gate: `just check`; focused `chat_session` and state-layer tests; dependency audit.

## Phase 3 — Promote session protocol and projection homes

1. Keep the Phase 2 leaf-value moves and compatibility re-exports intact.
2. Move store commands to `jinn-session-store-msg`:
   - `SessionLoadRequested`;
   - `SessionForkRequested`;
   - `LoadSessionPickerEntries`;
   - `ArchiveSession`;
   - `ArchiveSessionTree`;
   - store-facing `PersistSession` after proving the store-msg dependency direction is acyclic.
3. Move `CloseSession` and `TeardownSessionTree` to `jinn-session-lifecycle-msg`.
4. Move `SessionLoadCompleted` to `jinn-session-store-msg` only after changing its payload to `{ session_id: SessionId }`. All subscribers must prove the session is already visible in the state layer.
5. Move `SessionClosed`, `MarkSessionInteracted`, `UserInteracted`, `RetryStalledSession`, and the shared `SessionRemoved` fact to `jinn-session-msg`.
6. Flip production imports to the new homes while preserving compatibility re-exports until all callers are migrated.
7. Move the `SessionSummary`/`FrozenTreeNode` and token-value consumers to the new homes; keep functions traversing `SessionMap`/`ChatSessionState` in the state layer.

Gate: `just check`; focused contract serde tests; no dependency cycle.

## Phase 4 — Introduce snapshot-based persistence

1. Define `SessionSnapshot` in the neutral state layer. It contains exactly one complete durable representation:

```rust
#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub revision: SessionRevision,
    pub metadata: SessionSnapshotMetadata,
    pub entries: Vec<ChatEntry>,
    pub token_ledger: Vec<TokenRecord>,
    pub session_state: SessionState,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionRevision(u64);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshotMetadata {
    pub session_id: SessionId,
    pub title: Option<String>,
    pub updated_at: Timestamp,
    pub created_at: Timestamp,
    pub profile: SessionProfile,
    pub cwd: PathBuf,
    pub parent_session: Option<SessionId>,
    #[serde(default)]
    pub fork_ordinal: Option<usize>,
    #[serde(default)]
    pub origin: SessionOrigin,
    #[serde(default)]
    pub project: Option<PathBuf>,
    pub blobs: HashMap<String, JsonValue>,
    pub lifecycle_name: Option<String>,
    pub lifecycle_args: Vec<String>,
    pub lifecycle_script_state: LifecycleScriptState,
    #[serde(default)]
    pub task_list: TaskList,
    #[serde(default)]
    pub enabled_mcp_servers: BTreeSet<String>,
    #[serde(default = "default_persist")]
    pub persist: bool,
}
```

   `SessionSnapshotMetadata` is serialized flat into the existing `sessions.metadata` column. It is not nested under a `metadata` JSON key. `home`, `has_interacted`, the activity clocks, phase/queue/stream state, and MCP runtime status/stderr are not included. `session_state` is carried in the in-process `SessionSnapshot` and written to the indexed `sessions.archived` column.
2. Exclude runtime-only data:
   - phase machine;
   - queue and pending batches;
   - streaming/in-flight guards;
   - pending mutations;
   - busy count;
   - cached context size;
   - discovered resources;
   - MCP runtime status/stderr.
3. Change `SessionStore` and `SessionStoreService` to save `SessionSnapshot` and return `SessionSnapshot` from full-session load.
4. Keep a `std::sync::atomic::AtomicU64` capture counter on the authoritative session aggregate, initialized to zero on creation/load and excluded from serde. `capture_snapshot()` increments it with `fetch_add(1, Ordering::Relaxed)` while holding the aggregate/state read lock and returns that value with the cloned durable data. Atomic ordering is sufficient for the in-process save gate; the application state lock supplies the coherent data boundary. This avoids changing every semantic mutator or persisting a new schema column.
5. Update both direct turn-path persistence and `SessionStoreActor` persistence to call the snapshot service.
6. Keep `PersistableCore` private to `jinn-session-store` as the flat `sessions.metadata` serialization adapter. `SessionSnapshotMetadata` is the canonical durable model in `jinn-session-state`; conversion between them is explicit and tested. Neither type depends on actor-bearing lifecycle/store crates.
7. Keep the metadata JSON flat and field-compatible. Map each old field exactly once into the snapshot metadata DTO; `SessionRevision` is not written to the metadata blob in this migration.
8. Make SQLite save continue to write metadata, history junction rows, attachments, and token ledger in one transaction.
9. Make SQLite full-session load use one read transaction so metadata, entries, attachments, and ledger come from one database version.
10. Make archive/fork behavior explicit around snapshots:
    - fork uses the current source snapshot and writes the new session atomically;
    - archive writes durable archive state before removing the live session;
    - failures do not leave a half-removed live session.
11. Serialize saves per session in `SessionStoreService` with an in-process per-session save gate. Every save request carries `SessionRevision`; while the gate is held, skip the queued request if a newer revision is already queued. Thus captured revisions reach SQLite in monotonic commit order without adding a schema column or changing persisted JSON.

Gate: `just check`; focused SQLite tests; exact JSON/legacy load tests; transaction failure tests.

## Phase 5 — Move the coordinated turn reducer — Part 3

1. Add `jinn-session-turn` under `crates/slices/`.
2. Move the complete coordinated actor implementation as one unit:
   - actor shell and subscriptions;
   - enqueue and turn intake;
   - streaming completion and token handling;
   - tool-call and continuation folds;
   - stall retry;
   - history/context/mutation coordination;
   - direct turn-path snapshot persistence;
   - associated helpers and tests.
3. Preserve the actor’s existing contract set during the move. Do not redistribute handlers to inference, tools, or watchdog merely because they publish inference/tool messages.
4. Replace direct `jinn_domain` state paths with state-layer access services:
   - synchronous read facade for render/UI consumers;
   - explicit mutation operations for turn reducer writes;
   - snapshot capture for persistence;
   - revision-aware initialization/load operations.
5. Keep the phase machine, generation guards, pending batches, and queue coordination in the state layer/turn boundary. They are not independent cells.
6. Move the pure multimodal gate to provider selection after its model vocabulary is rehomed. Relocate the pure image-resolve implementation from the session actor into the existing `feat/image_convert` kernel feature, because it has no session-state dependency; it is not retained as session-actor residue.
7. Complete the handler split while preserving the coordinated turn fold:
   - move `ToolsRegistered`, `ToolsUnregistered`, and `SessionClosed` tool-registry cleanup into `jinn-tools`' existing `ToolOrchestratorActor`, which already owns the registry cell and already handles `ToolsUnregistered`/`SessionClosed`; remove those subscriptions and methods from the session actor;
   - move `ModelsRefreshed` transient-history rendering into `jinn-provider-selection`'s existing `ProviderActor`, which already handles that event; remove the duplicate session-actor subscription;
   - relocate pure image resolution from `session_actor/handlers/image_resolve.rs` into the existing `feat/image_convert` feature;
   - keep pin/unpin coordination, persona selection/picker population, skills-refresh display, and `SubmitHistoryMutations` in the cohesive session-turn reducer because each crosses session, frontend, and/or history-mutation state;
   - no production `session_actor/handlers/` module remains in `jinn-domain`.
8. Update `src/actor_wiring.rs` to activate `jinn-session-turn` in the same readiness position as the current kernel actor.
9. Remove `jinn-domain`’s session actor implementation entirely; compatibility exports may remain only for types, never actor or handler code.
10. Re-verify event ownership after the move: each contract has one actor handler; the turn slice is the owner of coordinated fold contracts.

Gate: `just check`; focused turn/session tests; actor subscription uniqueness audit; no kernel session actor implementation.

## Phase 6 — Finish consumer migration and reduce `jinn-domain`

1. Migrate production consumers of `ChatSessionState` to:
   - the state-layer read facade;
   - explicit projections/snapshots;
   - lower-level message/value contracts;
   - store/lifecycle commands.
2. Migrate store handlers first, then lifecycle, turn dispatch, tools, sidebar, context assembly, token counting, Discord, picker specs, and remaining UI consumers.
3. Add `jinn-session-list` as a non-slice library depending on `jinn-core-types`, `jinn-session-msg`, `jinn-session-store-msg`, and `jinn-sidebar-msg`. It owns only pure projection/visible-tree algorithms over caller-provided `SessionEntry`/session facts; it must not depend on `jinn-domain`, `AppState`, `jinn-slices`, or a slice implementation. Move `build_session_tree`, DFS flattening, effective-parent resolution, and selection-to-session validation there.
4. Move `AppState`/frontend/sidebar-cell orchestration from `sessions_list/state.rs` and `sessions_list/reconcile.rs` into `jinn-sidebar`; keep the existing `SidebarSections` cell as the single live projection writer. `SessionStoreActor` publishes a shared `SessionRemoved` fact after durable removal. `SidebarStateActor` owns visual-parent repair and cursor/active-session reconciliation from that fact. The store actor performs only `SessionMap` removal/replacement and frozen-node insertion.
5. Move close/archive tree arming and subagent-load selection to their actual owners: sidebar owns close/tree prompt state and first-press validation; store/lifecycle actors own commands; `jinn-sidebar` owns the child-session selection adapter. `IntentHandler` keeps only dynamic-intent dispatch.
6. Remove direct production imports of:
   - `jinn_domain::feat::session::ChatSessionState`;
   - `jinn_domain::feat::session::SessionCore`;
   - old `jinn_domain::feat::session` protocol paths;
   - kernel `sessions_list` modules.
7. Remove kernel compatibility re-exports only after all production consumers are migrated and tests prove old paths no longer define canonical behavior.
8. Remove session actor and handler modules from `jinn-domain`; remove now-unused session dependencies and modules.
9. Update `jinn-domain/Cargo.toml` only after the source move is complete. Remove dependencies no longer required, but do not remove dependencies still used by shared context/UI/intent code.
10. Keep the existing global `State`, `AppState`, capability architecture, and `IntentHandler` unless a specific remaining dependency requires a narrow compatibility shim.
11. Replace MCP runtime status/stderr with `McpRuntimeState` in `jinn-mcp-msg`. `jinn-mcp-slice::activate` registers the cell and retains the only write handle in `McpCoordinatorActor`; tools, sidebar, and TUI read it through the shared slot key. Remove `SessionCap` writes and session accessors for status/stderr. The coordinator retains `SessionCap` only to reconcile durable `enabled_mcp_servers` with persisted enablement. Clear a session's runtime entry on close/archive/load initialization. Durable enablement remains in `SessionSnapshotMetadata`.

Gate: `just check`; `rg` proof of no production live-session imports; production LOC before/after report.

## Phase 7 — Documentation, Record, and final gates

1. Regenerate the current actor/message/session ownership inventory.
2. Update `migration.md`, `slices.md`, `catalog.md`, and `cleanup.md` to describe the final state and rationale.
3. Remove the old “SessionCore remains kernel-owned” claims from current-state sections.
4. Leave `post-cleanup.md` and `actor-migration/raw/**` untouched.
5. Amend `.agents/RECORD.md` only after verifying each final claim against code.
6. Run focused tests during implementation and exactly one final `just test`.
7. Run `just lint` and `just fmt-fix`; resolve all warnings.
8. Audit dependency graph, snapshot consistency, serde shape, legacy loads, live-session visibility, and production LOC reduction.
9. Commit coherent phases using Fossil, preserving the unrelated user `CHANGELOG.md` edit.

---

# Anti-Goals (Out of Scope)

- Do not make the five broad groups into independent session cells.
- Do not move atomic history/phase/stream/queue state into independently locked cells.
- Do not implement `CellReader`/`CellWriter` typed-cell helpers in this task.
- Do not make a SQLite transaction responsible for repairing torn in-memory state.
- Do not move only one stream/tool/stall handler into another actor while its coordinated state remains in the kernel.
- Do not move the complete session aggregate directly into `jinn-session-store`, `jinn-session-lifecycle`, or another actor-bearing implementation crate.
- Do not create a `jinn-domain` dependency on `jinn-session-store`, `jinn-session-lifecycle`, or `jinn-session-turn`.
- Do not create a cycle among store, lifecycle, turn, and state crates.
- Do not move `jinn-domain`’s shared `State`, `AppState`, `IntentHandler`, or all UI code solely to reduce line count.
- Do not move chat-input, chat-log rendering, picker infrastructure, or context assembly-only logic in this session program. The neutral prompt/context model required by the live aggregate is included; full context-assembly extraction remains separate cleanup.
- Do not change SQLite schema tables or migration versions unless required to implement a compatible snapshot representation. Existing on-disk JSON must remain loadable.
- Do not change user-visible session behavior, history behavior, tool-loop ordering, lifecycle behavior, fork/archive behavior, or search behavior except where an explicit bug fix is covered by a test.
- Do not modify `/mnt/zed/repos/jinn/actor-migration/post-cleanup.md`.
- Do not modify `/mnt/zed/repos/jinn/actor-migration/raw/**`.
- Do not rewrite historical completed plans as current-state documentation.
- Do not add external libraries without a concrete need; use existing workspace dependencies.
- Do not preserve direct `ChatSessionState` access for consumers merely to avoid import migration.

---

# Edge Cases & Gotchas

- **`SessionState` name collision:** `jinn-domain::SessionState` currently aliases `SessionMap`; the loaded/archived enum is `chat_session::SessionState` and now belongs to `jinn-session-store-msg`. Do not export the enum at the `jinn-domain` crate root under the old alias.

- **Flat serde:** The five broad groups are flattened. The snapshot metadata DTO must produce the same flat JSON keys. Never serialize a new nested `identity`, `lifecycle`, `history_work`, `integrations`, or `storage` wrapper into persisted metadata unless a schema migration is explicitly planned.

- **`home`:** `home` is runtime-only and absent from the persisted metadata. The snapshot must not add it.

- **`has_interacted`:** Current SQLite mapping omits it despite the field’s serde shape. Keep it runtime-only as a persistence-eligibility field and do not add it to `SessionSnapshotMetadata`.

- **Activity clocks:** Current production search found no production readers for `last_history_activity_at` and `last_provider_activity_at`. Retain them as runtime-only compatibility telemetry, remove the stale watchdog-owner comments, and leave removal to a later cleanup.

- **`blobs`:** Current production search found no production writer or reader. Retain the serialized field for compatibility, document it as unowned compatibility storage, and leave removal to a later migration-aware cleanup.

- **Model alloy rotation:** `SessionProfile::model` can be mutated during dispatch/model resolution folds, not only by `ProviderActor`. Do not make the profile a single-owner provider cell without defining where alloy rotation occurs.

- **Task list:** `task_list` is tools-owned but currently persisted in the session metadata blob. It must remain in the complete durable snapshot until a separate persistence model explicitly replaces it.

- **MCP split:** Durable `enabled_mcp_servers` stays in the snapshot. Runtime `mcp_server_status` and `mcp_server_stderr` move to an MCP-owned cell and are excluded from the snapshot. The tool orchestrator’s status gate must read the MCP projection, not stale session state.

- **Cell authority:** `Slices::reader` currently returns `TypedCell<T>`, so readers can call `update`. The MCP cell migration must be written with one explicit writer and documented read consumers; it must not imply the API already enforces capability separation.

- **Load visibility:** A session is not published as loaded until all aggregate/snapshot fields are initialized. No subscriber may observe a partially reconstructed session.

- **Snapshot revision:** A snapshot captured before a concurrent update is valid for its captured revision. The atomic capture counter is an in-process ordering token only; the `State` lock supplies the coherent data boundary. `SessionStoreService` serializes saves per session and skips a queued older revision when a newer capture is already queued.

- **Read transaction:** SQLite’s current full-session load performs several queries. Use one read transaction so metadata, history, attachments, and token ledger cannot be observed from different database versions.

- **Archive failure:** Existing archive paths can remove a live session after a save error. The new explicit archive flow must not remove live state before durable archive success.

- **Turn actor completeness:** Streaming, tool-call, enqueue, retry, and persistence are one coordinated reducer. Do not move individual files to different slices merely because they consume inference/tool messages.

- **HistoryEditor is not a runtime owner:** It is a mutation primitive. The turn reducer remains the semantic coordinator for history changes that affect phase/stream/tool state.

- **Compatibility paths:** Temporary re-exports are allowed only during migration. They must not become a second definition of state or a reason for production consumers to remain coupled to `jinn-domain`.

- **Dependency direction:** `jinn-session-state` is neutral and lower-level. `jinn-session-store`, `jinn-session-lifecycle`, and `jinn-session-turn` may depend on it. `jinn-session-state` must not depend on those implementation crates.

- **Protocol direction:** Lifecycle actors publish store commands. If store-msg owns store commands, store-msg must not depend on lifecycle-msg. Keep `PersistableCore` private or place the snapshot in a neutral model layer to avoid this cycle.

- **Test discipline:** Use `just check` for compile checks, `just test-one <filter>` for focused iteration, and one final `just test` per final gate. Never run `cargo test` or `cargo test -p` directly.

- **Fossil selective commits:** The repository’s `just commit` recipe stages all changes. The unrelated user `CHANGELOG.md` edit must be preserved and excluded from task commits.

---

# Navigation Anchors

- `crates/jinn-domain/src/feat/session/chat_session.rs:168-224` — current five-group `SessionCore` and defaults.
- `crates/jinn-domain/src/feat/session/session_lifecycle_fields.rs:21-223` — current broad group definitions and serde attributes.
- `crates/jinn-domain/src/feat/session/chat_session.rs` — `ChatSessionState`, constructors, restore methods, and all semantic accessors.
- `crates/jinn-domain/src/common/session_map.rs:38-54` — active-session map and invariants.
- `crates/jinn-domain/src/common/state.rs:15-20` — global state lock.
- `crates/jinn-domain/src/common/tcaps/session.rs:92-135` — current broad session write projection.
- `crates/jinn-domain/src/feat/session/session_store.rs:35-69` — current live-state store trait.
- `crates/jinn-domain/src/feat/session/session_store/service.rs:21-70` — current service wrapper.
- `crates/slices/jinn-session-store/src/sqlite.rs:523-620` — `PersistableCore` and conversions.
- `crates/slices/jinn-session-store/src/sqlite.rs:218-273` — current multi-query load path.
- `crates/slices/jinn-session-store/src/sqlite.rs:719-782` — current one-transaction save path.
- `crates/slices/jinn-session-store/src/sqlite.rs:1124-1180` — fork transaction.
- `crates/slices/jinn-session-store/src/session_store_actor/handlers/load.rs` — load/reconstruction.
- `crates/slices/jinn-session-store/src/session_store_actor/handlers/archive.rs` — archive and live removal.
- `crates/slices/jinn-session-lifecycle/src/session_lifecycle_actor/handlers/{setup,teardown,close,cwd}.rs` — lifecycle writes.
- `crates/jinn-domain/src/feat/session/session_actor.rs` — current turn actor subscription chain and spawn.
- `crates/jinn-domain/src/feat/session/session_actor/handlers/streaming.rs` — atomic stream completion fold.
- `crates/jinn-domain/src/feat/session/session_actor/handlers/tool_calls.rs` — atomic tool continuation fold.
- `crates/jinn-domain/src/feat/session/session_actor/handlers/enqueue.rs` — turn intake and dispatch fold.
- `crates/jinn-domain/src/feat/session/session_actor/handlers/stall_retry.rs` — retry fold.
- `crates/jinn-domain/src/feat/session/session_actor/handlers/persistence.rs` — direct turn-path snapshot persistence.
- `crates/slices/jinn-mcp-slice/src/coordinator.rs:459-485` — current MCP status/stderr writes through session state.
- `src/actor_wiring.rs:316-339` — current store/lifecycle/kernel session actor activation order.
- `crates/jinn-slices/src/cell.rs` and `crates/jinn-slices/src/slices.rs` — typed-cell limitation and independent-cell model.
- `.plans/session-partition/handoff-to-part2.md` — historical three-part intent, stale field map, and Part 3 inventory.
- `.plans/session-core-broad-groups-migration-refresh/plan.md` — current completed scaffolding contract and anti-goals.
- `.agents/RECORD.md` — authoritative current facts to amend at implementation end.

---

# Dependency Mappings

## New workspace members

- `jinn-context` — neutral prompt-template store/expansion, path resolution, context-file model/loaders, and frontmatter support required by `ChatSessionState`.
- `jinn-session-list` — owner-neutral loaded-session projection, visible-tree, and visual-parent algorithms; frontend state remains in `jinn-sidebar-msg`.
- `jinn-session-state` — neutral session aggregate, state map, snapshot, and reconstruction contracts.
- `jinn-session-turn` — cohesive turn reducer slice.

## New internal dependencies

### `jinn-context` may depend on

- `jinn-core-types`
- `jinn-persona-msg`
- `serde`, `serde_json`, `jiff`, `regex`, `fuzzy-matcher`, `tokio`, `toml`, `error-stack`, and `wherror`

`jinn-session-init-msg` may re-export `jinn_context::PromptTemplate`; `jinn-context` must not depend on `jinn-session-init-msg`. Neither crate may depend on `jinn-domain` or an actor-bearing implementation crate.

### `jinn-session-list` may depend on

- `jinn-core-types`
- `jinn-session-msg`
- `jinn-session-store-msg`
- `jinn-sidebar-msg`

It must not depend on `jinn-domain`, `jinn-slices`, `jinn-sidebar`, `jinn-session-store`, or any other implementation crate.

### `jinn-session-state` may depend on

- `jinn-core-types`
- `jinn-context`
- `jinn-persona-msg`
- `jinn-skills`
- `jinn-session-msg`
- `jinn-session-lifecycle-msg`
- `jinn-session-store-msg`
- `jinn-session-history-msg`
- `jinn-session-history` if the history editor implementation remains library-only and acyclic
- `jinn-token-count-msg`
- `jinn-tools-msg`
- `jinn-slices`
- `jinn-chat-log-view-msg` and `jinn-chat-input-msg` for existing view/input facades
- `serde`, `serde_json`, `jiff`, `uuid`, and existing error/runtime crates

### `jinn-session-state` must not depend on

- `jinn-domain`
- `jinn-session-store`
- `jinn-session-lifecycle`
- `jinn-session-turn`
- any actor-bearing implementation crate

### `jinn-session-turn` may depend on

- `jinn-session-state`
- `jinn-domain` only for genuinely shared application services/capabilities that remain in the kernel
- `jinn-slices`
- `jinn-core-types`
- `jinn-session-msg`
- `jinn-session-history-msg`
- `jinn-session-lifecycle-msg`
- `jinn-session-store-msg`
- `jinn-inference-msg`
- `jinn-tools-msg`
- `jinn-turn-dispatch-msg`
- `jinn-context-curation-msg`
- `jinn-token-count-msg`
- `trouper`
- `tokio`
- existing workspace runtime/error crates

### `jinn-session-turn` must not depend on

- `jinn-session-store` implementation
- `jinn-session-lifecycle` implementation
- `jinn-domain` session implementation paths after the move
- any new `jinn-domain` → implementation edge

### `jinn-session-store` may depend on

- `jinn-session-state`
- `jinn-domain` only for shared `State`/service/capability infrastructure that remains kernel
- lower-level contract crates
- SQLite/runtime dependencies already present

### `jinn-session-store` must not depend on

- `jinn-session-lifecycle` implementation
- `jinn-session-turn` implementation
- `jinn-domain` after the session implementation is removed, except for explicit shared service seams

## Existing low-level homes

- `jinn-core-types`: `SessionId`, `ChatEntry`, `ChatHistory`, `HistoryMutation`, `ModelSelection`, `ReasoningEffort`, and, after extraction, `SessionProfile`/endpoint values.
- `jinn-session-msg`: shared session identity, phase, lifecycle event, and phase-machine vocabulary.
- `jinn-session-lifecycle-msg`: lifecycle commands/events and lifecycle leaf vocabulary.
- `jinn-session-store-msg`: store commands/events, `SessionState`, summaries, frozen tree projections, and search data.
- `jinn-session-history-msg`: history, pin, mutation, citation, and task-list events.
- `jinn-token-count-msg`: token records/statistics.
- `jinn-tools-msg`: tool/task vocabulary.
- `jinn-mcp-msg`: MCP events and status values.
- `jinn-slices`: cells, views, routes, focus, and slice activation mechanisms.

## Dependency direction target

```text
jinn-core-types / *-msg / jinn-context
          ↑
jinn-session-state
          ↑
jinn-domain shared spine
          ↑
session-turn / session-store / session-lifecycle implementation slices
          ↑
composition
```

The exact edge from `jinn-domain` to `jinn-session-state` is permitted. The reverse edge from state to kernel or implementation slices is not.

## External libraries

No new external library is required. Use the existing workspace versions of:

- `serde`;
- `serde_json`;
- `jiff`;
- `uuid`;
- `tokio`;
- `trouper`;
- `error-stack`;
- `wherror`;
- `rusqlite`/database crates already used by the store.

---

# Test Strategies

## Phase 1 — Inventory

- Add a structural audit test or script that records current field paths, production writers, and production readers.
- Run `just check`.
- Run `cargo metadata --format-version 1 --no-deps` and verify the baseline dependency graph has no cycle.
- Run the source-size audit and retain the before/after production and total counts for the implementation summary; do not create a separate report document.
- Verify that current `SessionCore` has exactly five flattened groups plus ephemeral.

## Phase 2 — State/context crate

- Add structural dependency proof for `jinn-context`.
- Add `jinn-context` and verify prompt-store, attachment-path, context-file, frontmatter, and `PromptTemplate` re-export behavior.
- Move the leaf values required by the live aggregate to core/session/token/store homes.
- Add `jinn-session-state` and move/verify the complete live aggregate and `SessionMap`.
- Run `just check`, focused context tests, `just test-one chat_session`, and focused state tests.

## Phase 3 — Protocol and projection homes

- Test every moved command/event/projection type for schema and serde compatibility.
- Convert `SessionLoadCompleted` to ID-only and prove load visibility before publication.
- Flip all production imports to the new protocol/value homes while retaining temporary compatibility re-exports.
- Run `just check`, focused store/session protocol tests, and the dependency audit.

## Phase 4 — Snapshot persistence

- Add a test proving `SessionSnapshot` contains every field currently represented by `PersistableCore`, history, attachments, and token ledger exactly once.
- Add an exact serialized JSON-shape test for a populated snapshot.
- Add a legacy flat JSON load test.
- Add a save transaction rollback test that fails after metadata write and verifies history/ledger are unchanged.
- Add a load-read-transaction test that observes one coherent version.
- Add a snapshot revision test and stale-save ordering test.
- Add fork snapshot tests for parent, ordinal, origin, profile, history, and task permissions.
- Add archive failure test proving live session remains visible when durable archive fails.
- Run `just test-one sqlite`.
- Run focused store integration tests.

## Phase 5 — Turn reducer

- Move existing session actor tests with implementation changes only.
- Add a test that stream completion updates history, phase, stream guard, pending batch, and persistence snapshot consistently.
- Add a test that tool-call continuation preserves ordering and tool-loop atomicity.
- Add a test that retry updates the generation guard and redispatches exactly once.
- Add a test that turn-path snapshot capture sees the post-mutation revision.
- Add a test that a lifecycle transition publishes a valid snapshot request.
- Run `just test-one session_actor` or the closest focused filter.
- Run `just check`.
- Audit that each production contract has exactly one handler registration.

## Phase 6 — Consumer migration

- Add integration tests for:
  - session load visibility;
  - store actor reconstruction;
  - lifecycle actor setup/teardown;
  - sidebar sessions-list reconciliation;
  - fork and archive flows;
  - Discord final-reply reads;
  - tools child-session inheritance;
  - token count actor reads;
  - context assembly inputs;
  - picker session rows.
- Run `rg` proofs:
  - no production `jinn_domain::feat::session::ChatSessionState`;
  - no production `jinn_domain::feat::session::SessionCore`;
  - no old store/lifecycle protocol imports;
  - no direct kernel session actor handler implementation.
- Run `just check`.

## Phase 7 — Documentation and final gates

- Regenerate migration document inventories and compare actor counts, subscriptions, contract homes, and state ownership.
- Verify the four current migration documents no longer claim that session implementation is kernel-owned.
- Verify `post-cleanup.md` and `raw/**` hashes are unchanged.
- Run one final `just test`.
- Run `just lint`.
- Run `just fmt-fix` and `cargo fmt -- --check`.
- Run final dependency, snapshot, serde, legacy-load, archive/fork, and production-LOC audits.
- Commit coherent changes with Fossil while preserving the unrelated `CHANGELOG.md` edit.

---

# Acceptance Criteria

- `jinn-domain` contains no `SessionCore` definition.
- `jinn-domain` contains no `ChatSessionState` definition.
- `jinn-domain` contains no session turn actor handler implementation.
- `jinn-session-state` owns the authoritative live session aggregate and snapshot contracts.
- `jinn-context` owns portable prompt/context models and loaders used by session state and discovery.
- `jinn-session-turn` owns the complete coordinated turn reducer.
- `jinn-session-list` owns owner-neutral session projection and visible-tree algorithms, while `jinn-sidebar` owns sidebar live-state reconciliation.
- `jinn-session-store` consumes `SessionSnapshot`, not `ChatSessionState`.
- No production slice imports `jinn_domain::feat::session` for live session state.
- No `jinn-domain` → `jinn-session-store`, `jinn-domain` → `jinn-session-lifecycle`, or `jinn-domain` → `jinn-session-turn` implementation dependency is introduced.
- No dependency cycle is introduced.
- The session aggregate remains one authoritative coherent capture boundary for atomic turn state.
- Independent MCP runtime state is excluded from the durable session snapshot and is owned by the MCP coordinator through `McpRuntimeState`.
- Persisted enabled MCP configuration remains in the session snapshot.
- Existing flat metadata JSON remains loadable and compatible.
- Current metadata JSON shape is preserved unless a separately reviewed migration is required.
- Metadata, history, attachments, and token ledger commit in one SQLite transaction.
- Full-session load reads one coherent database snapshot.
- A session is not published as loaded before all live/snapshot state is initialized.
- Fork behavior remains correct for metadata, history, ordinal, origin, and tool permissions.
- Archive behavior does not remove a live session when durable archive fails.
- Streaming, tool-call, enqueue, retry, and persistence ordering remain unchanged.
- The implementation includes a published before/after production LOC report showing measurable removal from `jinn-domain`.
- The final Record contains only facts verified against the completed implementation.
- The typed-cell read/write capability API is not part of this task.
- `just test`, `just lint`, and formatting checks pass with zero failures and zero warnings.

---

# Phases

1. **Freeze the target ownership and migration inventory.** Capture current field, writer, reader, actor, handler, consumer, dependency, serde, and LOC facts. Reclassify the five broad groups into actual end-state facets and record compatibility decisions for activity clocks, `blobs`, and `has_interacted`.
2. **Create neutral context and session-state layers.** Create `jinn-context`, promote leaf values required by the live aggregate, then move the authoritative live aggregate and `SessionMap` into `jinn-session-state` without moving actor implementations or creating cycles.
3. **Promote session protocol and projection homes.** Move store/lifecycle/session commands and events, summaries, frozen projections, and shared values to existing lower-level crates; preserve compatibility exports until consumers are migrated.
4. **Introduce snapshot-based persistence.** Change the store seam and SQLite implementation to save/load complete snapshots, add coherent read transactions, preserve flat JSON compatibility, and define revision/ordering behavior.
5. **Move the coordinated turn reducer — Part 3.** Move the complete session actor implementation and all handlers that coordinate history, phase, streaming, tools, retries, context mutations, and direct persistence into `jinn-session-turn` as one owner.
6. **Finish consumer, sessions-list, and MCP projection migration.** Migrate all production session consumers, move sessions-list projection algorithms to `jinn-session-list` and frontend reconciliation to `jinn-sidebar`, move MCP runtime state to `McpRuntimeState`, remove compatibility shims and old kernel session modules, and publish the production LOC reduction.
7. **Finalize documentation, Record, and gates.** Refresh migration documents, amend the Record with verified end-state facts, run all tests/lints/audits, and commit coherent changes without the unrelated changelog edit.

---

# Record Updates

The following entries are proposed for implementation only. They are written to `.agents/RECORD.md` after the final implementation is verified. The existing entry stating that `SessionCore` remains the kernel-owned atomic persistence unit must be removed or replaced by these facts.

- (session) Live session state is owned by the `jinn-session-state` crate, which preserves the authoritative atomic session aggregate and runtime turn state.
- (session) Durable session persistence uses a complete `SessionSnapshot` containing session metadata, history, task state, and token accounting.
- (session) The session turn reducer is owned by `jinn-session-turn` and coordinates history, phase, streaming, tools, retries, and persistence.
- (session) `SessionStoreActor` persists and reconstructs complete `SessionSnapshot` values through SQLite.
- (session) SQLite session persistence commits metadata, history, attachments, and token-ledger changes in one transaction.
- (mcp) MCP runtime status and stderr are stored in an MCP-owned live cell, while persisted MCP enablement remains part of the session snapshot.
- (arch) `jinn-domain` retains shared application state, capabilities, service seams, and frontend orchestration rather than the complete session-domain implementation.
- (context) `jinn-context` owns prompt-template storage/expansion, path resolution, context-file models, and context loaders used by session state and discovery.
