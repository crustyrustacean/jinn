# Phase 1: Current Session Field Inventory

## Scope and classification method

Inventory date: 2026-09-25.

The inventory covers the five flattened groups declared in
`crates/jinn-domain/src/feat/session/session_lifecycle_fields.rs` and
`SessionCore` in `crates/jinn-domain/src/feat/session/chat_session.rs`.

Access is classified as:

- **raw group access**: a production access through
  `session.core.<group>.<field>`;
- **semantic method**: a production access through a public or
  crate-private `ChatSessionState` method;
- **serde/store adapter**: persistence conversion performed by
  `jinn-session-store`;
- **constructor/default**: initialization in defaults, constructors, or
  load reconstruction.

Tests are excluded from writer/reader ownership counts. Tests remain the
behavioral compatibility evidence.

## Serialized schema

Every group has `#[serde(flatten)]` on `SessionCore`, so its fields are flat
keys in persisted metadata. `SessionCore::ephemeral` has `#[serde(skip)]`.

| Group | Field | Type | Serde behavior | Durable snapshot decision |
| --- | --- | --- | --- | --- |
| identity | `session_id` | `SessionId` | required | yes |
| identity | `updated_at` | `Timestamp` | required | yes |
| identity | `last_history_activity_at` | `Timestamp` | `skip` | no, runtime turn clock |
| identity | `last_provider_activity_at` | `Timestamp` | `skip` | no, runtime turn clock |
| identity | `created_at` | `Timestamp` | required | yes |
| identity | `title` | `Option<String>` | default; omitted when none | yes |
| identity | `parent_session` | `Option<SessionId>` | default | yes |
| identity | `fork_ordinal` | `Option<usize>` | default | yes |
| identity | `origin` | `SessionOrigin` | default | yes |
| identity | `project` | `Option<PathBuf>` | default | yes |
| identity | `has_interacted` | `bool` | default | **no in snapshot**; runtime persistence eligibility only |
| lifecycle | `cwd` | `PathBuf` | default `PathBuf::from(".")` | yes |
| lifecycle | `home` | `PathBuf` | `skip` | no, process/session environment |
| lifecycle | `lifecycle_name` | `Option<String>` | default | yes |
| lifecycle | `lifecycle_args` | `Vec<String>` | default | yes |
| lifecycle | `lifecycle_script_state` | `LifecycleScriptState` | default | yes |
| history_work | `history` | `ChatHistory` | required in flattened core shape; normalized to entry rows by SQLite | yes, as entries |
| history_work | `token_ledger` | `Vec<TokenRecord>` | default | yes |
| history_work | `task_list` | `jinn_tools_msg::TaskList` | default | yes, despite being tools-owned at runtime |
| integrations | `profile` | `SessionProfile` | default | yes |
| integrations | `blobs` | `HashMap<String, serde_json::Value>` | default | yes, to preserve existing metadata |
| integrations | `enabled_mcp_servers` | `BTreeSet<String>` | default | yes, durable MCP configuration |
| integrations | `mcp_server_status` | `BTreeMap<String, McpConnectionStatus>` | `skip` | no, MCP runtime cell |
| integrations | `mcp_server_stderr` | `BTreeMap<String, String>` | `skip` | no, MCP runtime cell |
| storage | `session_state` | `SessionState` | default | yes |
| storage | `persist` | `bool` | default `true` | yes |

## Raw production access by field

### `session_id`

- **Raw writer**: `ChatSessionState::set_session_id` in `chat_session.rs`.
- **Raw reader**: `ChatSessionState::session_id` in `chat_session.rs`.
- **External semantic readers**: sessions-list construction, session-store load/archive, tools/task child and fetch/search paths, Discord/thread routing, token counting, context assembly, sidebar activation/continuation, picker host routing, and tests.
- **Direct raw exception**: `session_actor/handlers/misc.rs` reads
  `core.identity.session_id` while reconciling a tool registry after history
  mutation.
- **Store adapter**: `SessionCore` identity conversion in
  `jinn-session-store/src/sqlite.rs`.

### `updated_at`

- **Raw constructor/default writer**: identity default.
- **Raw writer**: `restore_updated_at`, `touch` in `chat_session.rs`.
- **Raw reader**: `updated_at` in `chat_session.rs`.
- **Semantic readers**: sessions list/session summary ordering, tree aggregate,
  store summaries, and sidebar session projections.
- **Store adapter**: persisted metadata conversion in SQLite.

### `last_history_activity_at`

- **Raw writer**: `push_entry_raw`, `begin_streaming`, `append_stream_token`,
  `append_thinking_token`, `set_last_history_activity_at`, and historical
  entry insertion paths in `chat_session.rs`.
- **Raw reader**: `last_history_activity_at` in `chat_session.rs`.
- **External production consumer**: session stall retry/watchdog code calls the
  semantic getter/setter through `ChatSessionState`.
- **Decision**: retain as runtime turn state in the authoritative aggregate.

### `last_provider_activity_at`

- **Raw writer**: `append_stream_token`, `append_thinking_token`,
  `set_last_provider_activity_at`, and provider-output/tool-call progression
  paths in `chat_session.rs`.
- **Raw reader**: `last_provider_activity_at` in `chat_session.rs`.
- **External production consumer**: session stall retry/watchdog code calls the
  semantic getter/setter through `ChatSessionState`.
- **Decision**: retain as runtime turn state in the authoritative aggregate.

### `created_at`

- **Raw constructor/default writer**: identity default.
- **Raw restore writer**: `restore_created_at` in `chat_session.rs`.
- **Raw reader**: `created_at` in `chat_session.rs`.
- **Semantic readers**: sessions-list/status/tree projection paths.
- **Store adapter**: persisted metadata conversion in SQLite.

### `title`

- **Raw writer**: `set_title` in `chat_session.rs`.
- **Raw reader**: `title` in `chat_session.rs`.
- **External production writers**: enqueue derives a first title; turn-dispatch
  queue sets a title; sidebar rename sets a title; session fork helpers clone a
  title; the Discord key/thread paths prepare title state.
- **External production readers**: sessions list, sidebar preview, project
  scope/summaries, token/tree aggregation, Discord thread display, context
  assembly, and tool search/fetch.
- **Store adapter**: persisted metadata and summary conversion in SQLite.

### `parent_session`

- **Raw restore/construction writer**: `restore_parent_session`,
  `set_parent_session`, child construction in `chat_session.rs`.
- **Raw reader**: `parent_session`, `is_persistable` in `chat_session.rs`.
- **External production readers**: session-store archive teardown, lifecycle
  teardown, sessions-list/tree reconciliation, project ancestry, token tree
  aggregation, and status rendering.
- **Store adapter**: persisted metadata, fork, and summary conversion.

### `fork_ordinal`

- **Raw writer**: `set_fork_ordinal` in `chat_session.rs`; fork construction in
  store/session actor helper code.
- **Raw reader**: `fork_ordinal` in `chat_session.rs`.
- **External production readers**: token/tree aggregation and status rendering.
- **Store adapter**: persisted metadata and fork conversion.

### `origin`

- **Raw constructor writer**: identity default and `new_child` construction.
- **Raw reader**: `origin` in `chat_session.rs`.
- **External production reader**: sessions-list projection distinguishes
  subagent sessions.
- **Store adapter**: persisted metadata and fork conversion.

### `project`

- **Raw writer**: `set_project` in `chat_session.rs`.
- **Raw reader**: `project`, `is_persistable` path (indirect via other fields)
  in `chat_session.rs`.
- **External production writers**: session creation and tools/task child
  inheritance.
- **External production readers**: project resolution and session summaries.
- **Store adapter**: persisted metadata and summary conversion.

### `has_interacted`

- **Raw writer**: `mark_interacted` in `chat_session.rs`.
- **Raw reader**: `has_interacted` and `is_persistable` in `chat_session.rs`.
- **External production writers**: turn persistence paths, store load/startup
  reconstruction, sidebar rename, and tools/task child creation.
- **External production readers**: sidebar rename decision and tests.
- **Store adapter**: the current SQLite metadata conversion intentionally omits
  this field despite its `Serialize` field presence.
- **Decision**: runtime-only persistence eligibility outside the durable
  snapshot. Loaded sessions are marked interacted during reconstruction, which
  preserves the current observable behavior.

### `cwd`

- **Raw writer**: `set_cwd` in `chat_session.rs`.
- **Raw reader**: `cwd`, attachment path resolution, and `expand_entry` in
  `chat_session.rs`.
- **External production writers**: session creation, lifecycle setup/cwd,
  inherited child creation, and store load initialization.
- **External production readers**: skills/context/MCP scanning, status/sidebar
  rendering, store summaries, tool dispatch, and context assembly.
- **Store adapter**: persisted metadata conversion in SQLite.

### `home`

- **Raw writer**: `set_home` in `chat_session.rs`.
- **Raw reader**: attachment path resolution in `chat_session.rs`.
- **External production writers**: session lifecycle setup and tools/task child
  construction.
- **External production readers**: no direct public getter and no production
  consumer outside path resolution.
- **Store adapter**: omitted through `#[serde(skip)]`.

### `lifecycle_name`

- **Raw writer**: `set_lifecycle_name` in `chat_session.rs`.
- **Raw reader**: `lifecycle_name` and `is_persistable` in `chat_session.rs`.
- **External production writers**: session creation and close/replacement
  creation.
- **External production readers**: lifecycle setup/teardown command lookup and
  sidebar session preview.
- **Store adapter**: persisted metadata conversion in SQLite.

### `lifecycle_args`

- **Raw writer**: `set_lifecycle_args` in `chat_session.rs`.
- **Raw reader**: `lifecycle_args` in `chat_session.rs`.
- **External production writers**: session creation.
- **External production readers**: lifecycle setup/teardown and store fork
  tests/projection paths.
- **Store adapter**: persisted metadata conversion in SQLite.

### `lifecycle_script_state`

- **Raw semantic writer**: `advance_lifecycle_after_setup` and
  `advance_lifecycle_after_teardown` in `chat_session.rs`.
- **Raw reader**: `lifecycle_script_state` in `chat_session.rs`.
- **External production readers**: lifecycle setup/close/teardown and render
  context.
- **Store adapter**: persisted metadata conversion in SQLite.

### `history`

- **Raw writer**: all history mutation and in-place entry update paths in
  `chat_session.rs`; direct inspection occurs in
  `session_actor/handlers/stall_retry.rs` and
  `session_actor/handlers/misc.rs`.
- **Raw reader**: every history/chat-log/navigation/projection path in
  `chat_session.rs`; token count actor, context assembly, Discord, tools,
  sidebar, store, and session actor handlers consume the semantic slice.
- **Store adapter**: normalized to ordered entry rows and attachments by
  SQLite.
- **Decision**: remains part of the authoritative aggregate because it must be
  atomically coordinated with phase, stream indices, pending mutations, token
  ledger, and turn state.

### `token_ledger`

- **Raw writer**: `push_token_record`, `finalize_last_token_record`,
  `set_last_token_model`, and `restore_token_ledger` in `chat_session.rs`.
- **Raw reader**: `token_ledger` in `chat_session.rs`.
- **External production readers/writers**: turn streaming/tool handlers append
  and finalize records; status bar, token count actor, token/tree aggregation,
  and store persistence read them.
- **Store adapter**: token-ledger rows in the same SQLite transaction as
  metadata/history.

### `task_list`

- **Raw semantic writer**: `task_list_mut` in `chat_session.rs`.
- **Raw reader**: `task_list` in `chat_session.rs`.
- **External production writers**: tools todo commands and context assembly
  seeds.
- **External production readers**: tools todo commands, sidebar task list and
  session preview, picker task list, and status/sidebar render checks.
- **Store adapter**: serialized inside session metadata and therefore part of
  the complete durable snapshot.
- **Decision**: runtime ownership remains tools/state boundary, but the durable
  snapshot must retain it to preserve current persistence.

### `profile`

- **Raw writer**: profile and its fields are mutated by `ChatSessionState`
  semantic methods in `chat_session.rs`.
- **Raw reader**: profile and endpoint/model/persona/tool/skill methods in
  `chat_session.rs`.
- **External production writers**: provider selection model/alloy rotation,
  enqueue fallback model, context assembly test/assembly behavior, persona
  selection, skill tool disabled-skill policy, and tool-loop disabled tools.
- **External production readers**: inference/context assembly, validators,
  pickers, status bar, store/fork, and MCP/tool gating.
- **Store adapter**: flat metadata profile fields are copied by SQLite.
- **Decision**: retain in the authoritative session aggregate because
  dispatch/model resolution currently rotates alloy and mutates profile during
  a coordinated turn.

### `blobs`

- **Raw semantic writer**: `blobs_mut` in `chat_session.rs`.
- **Raw semantic reader**: `blobs` in `chat_session.rs`.
- **External production consumers**: none found outside compatibility tests.
- **Store adapter**: copied by current flat metadata conversion.
- **Decision**: retain the serialized map for schema compatibility, but assign
  no production subsystem owner in this migration. New code must not depend on
  it as a new extension mechanism.

### `enabled_mcp_servers`

- **Raw writer**: MCP enable/disable/reconcile semantic methods in
  `chat_session.rs`.
- **Raw reader**: `enabled_mcp_servers` in `chat_session.rs`.
- **External production writers**: session creation/close seed, MCP coordinator
  reconciliation, and child-session inheritance.
- **External production readers**: tools namespace/status gating.
- **Store adapter**: durable set in flat metadata.
- **Decision**: remain in the session snapshot as durable MCP configuration.

### `mcp_server_status`

- **Raw writer**: `set_mcp_server_status` in `chat_session.rs`.
- **Raw reader**: `mcp_server_status` in `chat_session.rs`.
- **External production writer**: `McpCoordinatorActor` in
  `jinn-mcp-slice/src/coordinator.rs`.
- **External production readers**: tools tool-call gate, sidebar MCP section,
  and TUI MCP rendering.
- **Store adapter**: omitted through `#[serde(skip)]`.
- **Decision**: move to one MCP-owned runtime cell keyed by session id. It is
  independently live and excluded from `SessionSnapshot`.

### `mcp_server_stderr`

- **Raw writer**: `set_mcp_server_stderr` in `chat_session.rs`.
- **Raw reader**: `mcp_server_stderr` in `chat_session.rs`.
- **External production writer**: `McpCoordinatorActor` debounced log handler.
- **External production reader**: TUI MCP rendering.
- **Store adapter**: omitted through `#[serde(skip)]`.
- **Decision**: move with MCP runtime status into the MCP-owned runtime cell.

### `session_state`

- **Raw writer**: `set_session_state` in `chat_session.rs`.
- **Raw reader**: `session_state` in `chat_session.rs`.
- **External production writer**: session store archive handler.
- **External production readers**: no material non-test semantic consumer found
  beyond store/load initialization.
- **Store adapter**: durable state in flat metadata.
- **Decision**: retain in snapshot and state-layer lifecycle projection.

### `persist`

- **Raw writer**: `set_persist` in `chat_session.rs`.
- **Raw reader**: `persist` and `is_persistable` in `chat_session.rs`.
- **External production writer**: child-session construction can set it at
  construction time.
- **External production readers**: session persistence eligibility.
- **Store adapter**: durable policy in flat metadata.
- **Decision**: retain in snapshot.

## Direct raw group access outside the aggregate

Production code outside `chat_session.rs` accesses raw groups only at these
locations:

- `jinn-domain/src/feat/session/session_actor/handlers/misc.rs`: reads identity
  id and directly applies one history context override.
- `jinn-domain/src/feat/session/session_actor/handlers/stall_retry.rs`: scans
  partial/complete history entries.
- `jinn-session-store/src/sqlite.rs`: converts flattened groups to/from
  `PersistableCore` and normalized rows.

This is a small enough exception set to move first behind state-layer semantic
operations or a snapshot adapter.

## Dependency notes

- Identity fields use `SessionId` and `Timestamp` and are dependency-light.
- Location fields use `PathBuf` plus lifecycle vocabulary from
  `jinn-session-lifecycle-msg`.
- History work requires `ChatHistory`, token vocabulary, and `jinn-tools-msg`.
- Integrations require `SessionProfile`, JSON values, and MCP status vocabulary.
- Storage requires `SessionState` from `jinn-session-store-msg`.
- The five flattened groups may move as definitions to a neutral state crate
  without introducing a cycle once session profile/phase/token/store projections
  are promoted to lower-level homes.

## End-state facet classification

The five broad groups are review units, not ownership domains. The final session
state is classified by behavior and persistence coupling as follows.

| Facet | Fields | Durable snapshot | Runtime owner and boundary |
| --- | --- | --- | --- |
| Identity and tree | `session_id`, `created_at`, `updated_at`, `parent_session`, `fork_ordinal`, `origin` | all | `jinn-session-state` aggregate; created/forked once, touched and projected through semantic operations |
| Presentation metadata | `title`, `project` | both | state-layer projection over identity/tree; title rename/derivation and project stamping are explicit operations rather than new cells |
| Runtime activity | `last_history_activity_at`, `last_provider_activity_at` | neither | `jinn-session-state` turn aggregate, written through history/provider activity methods; the turn reducer is the semantic writer |
| Persistence eligibility | `has_interacted`, `persist` | `persist` only | `jinn-session-state` aggregate. Interaction is runtime eligibility; explicit `persist` policy is durable and part of snapshot metadata |
| Location environment | `cwd`, `home` | `cwd` only | state-layer aggregate; lifecycle owns semantic cwd/home changes, context/tool readers consume projections |
| Lifecycle execution | `lifecycle_name`, `lifecycle_args`, `lifecycle_script_state` | all | `jinn-session-lifecycle` is the semantic owner; values remain embedded in the authoritative aggregate for coherent capture and load publication |
| History | `history` | entries plus attachments | `jinn-session-state` owns the history editor and aggregate; history-curation and turn actors publish mutations through the turn/state boundary |
| Token accounting | `token_ledger` | all | `jinn-token-count-msg` owns records/statistics values; `jinn-session-turn` coordinates append/finalize, and state-layer aggregate remains the durable owner |
| Task planning | `task_list` | all | tools semantic mutations through state boundary; embedded in authoritative aggregate/snapshot for compatibility and atomic load/save |
| Provider profile | `profile` including persona, endpoint, model, disabled tools/skills | all | provider/context/tool owners publish semantic changes; profile remains embedded because dispatch may rotate alloy while a turn is active |
| Durable MCP configuration | `enabled_mcp_servers` | yes | MCP coordinator reconciles it against user intent; state-layer aggregate remains the durable owner |
| MCP runtime | `mcp_server_status`, `mcp_server_stderr` | no | independent `McpRuntimeState` cell keyed by `SessionId`, written only by `McpCoordinatorActor` |
| Compatibility extension data | `blobs` | yes | no production owner; retained solely as an existing flat-schema extension point, not used by new production code |
| Storage lifecycle | `session_state` | yes | store/lifecycle semantic owner; embedded in the state layer until an archived/loaded transition has been made explicit and observable |

### Facet boundaries that do not split atomic turn state

`history`, both activity clocks, token ledger, phase machine, streaming indices,
pending mutations, queue state, and in-flight tool-loop state are one turn
fold. They may expose semantic operations and projections, but they are not
independent cells in this migration.

`profile` and `task_list` have other semantic owners, but they are still
persisted parts of one session revision. Moving either to an independently
locked cell would allow a snapshot to contain profile/task values from a
different revision than history and turn state.

`enabled_mcp_servers` remains distinct from MCP runtime status because it is
durable user configuration reconciled by MCP, while status and stderr are
transient projections written by the coordinator.

### Facet classification decisions

- The broad `identity` group splits into identity/tree, presentation metadata,
  runtime activity, and persistence eligibility.
- The broad `lifecycle` group splits into location environment and lifecycle
  execution metadata.
- The broad `history_work` group splits into history, token accounting, and
  task planning without changing the authoritative capture boundary.
- The broad `integrations` group splits into provider profile, durable MCP
  configuration, MCP runtime, and compatibility extension data.
- The broad `storage` group splits into persistence eligibility and storage
  lifecycle.

## Compatibility decisions

### Activity clocks

Retain `last_history_activity_at` and `last_provider_activity_at`. The current
production code reads and writes both through the live session, and stall retry
uses the pair to distinguish history progress from provider responsiveness.
Both remain `#[serde(skip)]`, are absent from `SessionSnapshot`, initialize to
`Timestamp::now()` for a newly created aggregate, and move with the
authoritative turn state in `jinn-session-state`. Their documentation must
describe the state-layer/turn writer and stall-retry reader accurately; they are
not identity metadata, are not restored from persistence, and are not snapshot
fields.

### Generic blobs

Retain `blobs` in the flattened metadata schema. No production code outside the
aggregate's compatibility accessors uses it, so removing it would create an
unnecessary on-disk compatibility risk and provide no architectural benefit.
The snapshot must copy it exactly once, and no new production subsystem may use
it as an ownerless extension channel.

### Interaction eligibility

`has_interacted` remains runtime-only state used by `is_persistable`. It is
excluded from `SessionSnapshot` and from the snapshot metadata DTO because the
current SQLite adapter omits it. Load/startup reconstruction explicitly marks a
successfully restored session interacted, preserving current persistence
eligibility after restart. Only `persist` is durable policy metadata.

## Production live-session consumer matrix

This matrix includes explicit type references and implicit access through
`AppState::active_session`, `AppState::session*`, and `SessionMap`. It excludes
test-only construction and assertions unless the file also contains production
behavior.

| Consumer | Current access | End-state contract |
| --- | --- | --- |
| `jinn-domain` shared `AppState`, `SessionMap`, and `State` | Own `SessionMap<ChatSessionState>`; expose borrowed/mutable live sessions through the global lock | `jinn-session-state` owns `ChatSessionState`/`SessionMap`; shared `State` retains the lock and capability entry points |
| IntentHandler, global intent, chat entry selection, session/picker intents, lifecycle intent | Mutate and read live sessions synchronously | Read projections plus explicit synchronous state operations; no production imports of a kernel-defined live type |
| `jinn-session-turn` source-to-be | Coordinated mutation of history, phase, streams, tools, queue, profile, interaction, and persistence | Own the complete turn fold and use state-layer semantic operations/snapshot capture |
| `jinn-session-store` | Explicit `ChatSessionState` and `SessionCore`; save/load live state; raw group conversion in SQLite | Store `SessionSnapshot`; reconstruct state through `jinn-session-state`; keep flat `PersistableCore` private |
| `jinn-session-lifecycle` | Mutable live session for cwd/home, lifecycle metadata, archive/close | State-layer semantic operations and complete snapshot publication |
| `jinn-context-assembly` | Reads full history/profile/cwd/discovered resources; mutates profile, task list, context size, and discovery state | State-layer `AssemblyInputs` and explicit context-size/resource operations |
| `jinn-context-curation` | Publishes history mutations and reads history/phase | History/turn contracts; no concrete live aggregate import |
| `jinn-token-count` | Helper accepts `&ChatSessionState`; actor reads/mutates history counts | History/token projection and state-layer mutation operation |
| `jinn-tools` | Builds concrete child sessions, reads cwd/status/ledger, mutates task list and policy | `SessionSeed`/read projections/commands; no direct live aggregate construction |
| `jinn-discord` | Reads phase/title/history and prepares title writes | Session read projection and intent/command publication |
| `jinn-sidebar` | Reconciles live session map, renames, task preview, MCP section, sessions preview, pins | Owner-neutral session-entry/tree projection for list algorithms; sidebar state actor reconciles from session events |
| `jinn-mcp-slice` | Writes durable enabled servers and runtime status/stderr through `SessionCap` | MCP-owned runtime cell plus a single state-layer reconciliation for durable enablement |
| `jinn-provider-selection` and `jinn-boot` | Read/mutate profile model, endpoint, and alloy policy | Profile command/projection/state-layer operation |
| `jinn-status-bar` and `jinn-tui` | Read profile, token ledger, fork ordinal, cwd, lifecycle, and MCP runtime | Read-only projections; MCP runtime from MCP cell |
| `jinn-term` | Reads phase to build route rows | Phase from `jinn-session-msg` |
| `jinn-picker-specs` | Spec tests construct/mutate live sessions | Specs use lower-level context/profile/test projection fixtures; no live aggregate dependency |
| `jinn-session-history` | Sealed history primitive currently accepts an `impl` private to kernel | Move implementation accessor support to `jinn-session-state` without exposing mutable aggregate internals |
| `jinn-domain` token/tree/summary projections | Accept concrete `ChatSessionState` or maps of it | Accept read projections/lower-level values |
| `src/headless.rs` and `src/actor_wiring.rs` | Read final history and set startup cwd through `AppState` | Shared state API and startup lifecycle operation |

### Import-coupling observations

- Only `jinn-token-count`, `jinn-context-assembly`, `jinn-mcp-slice`,
  `jinn-discord`, `jinn-tools`, and `jinn-sidebar` have direct production
  imports of `jinn_domain::feat::session::ChatSessionState` in their feature
  source today; several other picker occurrences are test modules.
- The store is the strongest direct dependency: its trait, service, SQLite
  implementation, load/startup/archive handlers, and protocol payload all use
  the live type.
- `SessionLoadCompleted` currently transports a full `ChatSessionState`, which
  duplicates the aggregate and makes protocol movement depend on the kernel.
  It must become ID-only after load publication is made atomic.
- The MCP runtime maps are read by tools, sidebar, and TUI but written only by
  the MCP coordinator; they are the clearest independently safe live cell.
- Context assembly mixes read projection and state mutation. It must retain the
  global shared-state context but stop depending on a concrete kernel aggregate.

## Session actor topology and handler inventory

### Runtime shape

- Actor path: `session`.
- Mailbox capacity: 65,536.
- Overload policy: `Block`, so provider token bursts backpressure publishers
  instead of dropping terminal events.
- Subscription count: 30 message contracts.
- Current subtree size: 9,542 physical lines including `session_actor.rs` and
  `session_actor/helpers.rs`.
- Test-aware count: 2,756 physical lines before the first top-level
  `#[cfg(test)]` section in each actor file, followed by 6,786 lines of
  interleaved test code/helpers. This is the implementation baseline for the
  actor move, not a claim that test lines leave `jinn-domain`; focused
  behavioral tests move with their production ownership.

### Subscription classification

| Contract | Current handler area | End-state classification |
| --- | --- | --- |
| `EnqueueUserMessage` | enqueue | session-turn: expand/attach/title/interact/phase/queue/dispatch fold |
| `SubmitSteeringMessage` | enqueue | session-turn: queue and prompt-boundary coordination |
| `EnqueueResumeTurn` | enqueue | session-turn |
| `PushChatEntry` | enqueue | session-turn: atomic history entry and event fold |
| `PinChatEntry` | context | session-turn while pin and history/selection changes remain one fold |
| `UnpinChatEntry` | context | session-turn while pin and history/selection changes remain one fold |
| `LoadPersonaPickerEntries` | context | session-turn/state boundary; reads profile and populates frontend picker |
| `MarkSessionInteracted` | persistence | session-turn: eligibility mutation, event, and snapshot persistence |
| `SubmitHistoryMutations` | misc | session-turn: accumulator/pending batch/history/phase/override-event fold |
| `RetryStalledSession` | stall_retry | session-turn: activity clocks, generation guard, partial cleanup, redispatch |
| `SendToLlmProvider` | stall_retry | session-turn: single dispatch-receipt guard write point |
| `StreamToken` | streaming | session-turn: provider/history/activity-clock fold |
| `StreamCompleted` | streaming | session-turn: completion/cancel/error, token finalize, queue, mutation, phase, persistence |
| `ToolUseStarted` | tool_calls | session-turn |
| `ToolCallReceived` | tool_calls | session-turn |
| `ToolCallStreaming` | tool_calls | session-turn |
| `ToolExecutionCompleted` | tool_calls | session-turn |
| `ToolBatchCompleted` | tool_calls | session-turn: tool-loop/phase/history/pending mutations/continuation fold |
| `ToolExecutionStarted` | tool_calls | session-turn |
| `ToolExecutionOutput` | tool_calls | session-turn |
| `CitationsReceived` | streaming | session-turn: history append and history event |
| `ChatEntryPinChanged` | actor dispatch to persistence | session-turn snapshot persistence reaction |
| `TaskListUpdated` | actor dispatch to persistence | session-turn snapshot persistence reaction |
| `ModelsRefreshed` | misc | move rendering effect to `ProviderActor`; do not retain in session-turn |
| `SkillsLoaded` | misc | session-turn/state update because discovered skills participate in assembly |
| `ToolsRegistered` | context | move per-session context-cache effect to `ToolOrchestratorActor` |
| `ToolsUnregistered` | context | move per-session context-cache effect to `ToolOrchestratorActor` |
| `SessionClosed` | context | move session-scoped context-tool cache cleanup to `ToolOrchestratorActor` |
| `PromptTemplatesLoaded` | context | session-turn/state update because discovered prompt templates participate in expansion/assembly |
| `PersonasLoaded` | context | session-turn/state update because persona fallback changes the active profile |

### Handler-module dependencies

- `enqueue.rs` combines prompt expansion, async image resolution, history
  mutation, title/interaction, multimodal gating, provider selection, queueing,
  and dispatch. It cannot be separated from turn state until the context/model
  services are dependency-safe.
- `streaming.rs` combines provider events, local token counting, history entries,
  phase/generation guards, pending mutations, tool-use branching, queue drain,
  activity clocks, override events, and persistence. This is the strongest
  single proof that streaming belongs to the cohesive turn reducer.
- `tool_calls.rs` combines tool-call history updates, phase transitions, pending
  mutations, steering, context assembly, provider continuation, generation
  guards, and persistence.
- `stall_retry.rs` reads history completeness, both runtime activity clocks,
  generation state, and queue state before redispatch.
- `misc.rs` combines the provider-owned models refresh rendering effect with
  history mutation accumulation. Split the former after the whole actor moves.
- `context.rs` combines pin/prompt/persona state updates with tools-owned
  per-session context-cache effects. Move the actor first, then transfer the
  tools-owned effects.
- `image_resolve.rs` is pure image classification/conversion support shared by
  turn attachment flow. Image resolution belongs in the existing image-convert
  feature; the turn reducer retains the attachment decision/fold.
- `multimodal_gate.rs` is a pure model-capability policy and belongs in
  provider selection after profile/model vocabulary is promoted.
- `persistence.rs` captures the live aggregate, checks eligibility, and calls the
  shared store. Snapshot capture replaces the clone in the move.

### Cross-state and service dependencies

- Shared `State`, `SessionCap`, and `FrontendCap` provide the current coherent
  lock boundary.
- `Services` supplies `SessionStoreService` and `BusService`; the final turn
  slice consumes the global DI context rather than pulling individual services
  from it.
- `TiktokenCounter` and `HistoryWorkerChatEntryTokenCache` support token ledger
  mutation and mutation-cost accumulation.
- `ImageConverterService` supports attachment conversion.
- `build_assembly_inputs`/`assemble_via_service` couple stream/tool completion
  to context assembly and therefore remain in the same turn boundary.
- Tool event schemas move none of the fold by themselves; they stay subscribed
  to the turn reducer because the session aggregate records their ordered
  history and continuation state.

The activation must keep the existing single-mailbox order and the point at
which the actor is live. The final path may remain `session` for behavioral
compatibility even when the implementation crate is named
`jinn-session-turn`.

## `jinn-domain` size baseline

Measured on 2026-09-25 before implementation changes.

### Language-level baseline

`tokei crates/jinn-domain` reports:

- 240 Rust files;
- 70,401 physical Rust lines;
- 53,763 Rust code lines;
- 7,569 Rust comment lines;
- 9,069 Rust blank lines;
- plus the crate manifest's 109 TOML code lines.

The primary before/after metric is **53,763 Rust code lines**, because it is
computed consistently and is insensitive to comment/blank formatting.

### Strict source-physical baseline

A separate classifier reports:

- 26 dedicated test/test-support files, 19,307 physical lines;
- 198 files with no top-level inline test module, 54,096 physical lines;
- 16 production files with an inline `#[cfg(test)]` module, 3,819 lines before
  that module;
- strict production-candidate total: **57,915 physical lines**;
- all Rust physical total: **77,556 lines**.

The strict classifier is a migration metric, not a Cargo feature graph. Files
such as `test_harness.rs` and `test_services.rs` are dedicated test support and
are excluded from the production candidate even when a test-harness feature can
compile them elsewhere. Final reporting must use the exact same classifier.

## Baseline dependency graph

`cargo metadata --format-version 1 --no-deps` completed successfully, proving
Cargo can resolve the current 71-package workspace graph without a dependency
cycle. The captured metadata is `/tmp/jinn-cargo-metadata-before.json` with
SHA-256 `41e91ba142be933d63f49569d23848cae780b0023a1db67d3e175387124bb746`.

Current relevant direction:

- `jinn-core-types` and `jinn-slices` are foundational.
- `jinn-session-msg`, `jinn-session-store-msg`, `jinn-session-history-msg`,
  `jinn-token-count-msg`, `jinn-tools-msg`, and `jinn-persona-msg` are
  dependency-light contract/value layers.
- `jinn-domain` depends on those lower layers and on `jinn-session-history`,
  `jinn-skills`, and provider/picker support, but not on the actor-bearing
  session store/lifecycle implementations.
- `jinn-session-store`, `jinn-session-lifecycle`, `jinn-context-assembly`,
  `jinn-token-count`, `jinn-provider-selection`, `jinn-mcp-slice`,
  `jinn-sidebar`, `jinn-tools`, and other implementation slices currently
  depend on `jinn-domain`.
- `jinn-session-store-msg` depends only on `jinn-core-types`.
- `jinn-session-lifecycle-msg` already depends on `jinn-session-msg`, picker,
  preferences, theme, and slices; store commands therefore cannot move there
  without introducing a likely store-msg/lifecycle-msg cycle if store-msg also
  publishes lifecycle contracts.

The new `jinn-context` and `jinn-session-state` crates must depend on foundational
/value crates only. `jinn-domain` may consume the new neutral crates; actor
implementation crates may also consume them. No neutral crate may depend back
on `jinn-domain` or an actor-bearing session implementation.
