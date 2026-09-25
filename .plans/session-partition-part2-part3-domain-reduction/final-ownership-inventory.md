# Final Production Actor / Handler / State Inventory

Measured after the complete session partition and consumer migration.

## Runtime actor count

- Production `ServiceActor` implementations: **37**.
- Production builder registrations: **36**. `SessionInitSupervisor` and
  `DiscoveryNotifier` are singletons; `SessionDiscoveryWorker` is one keyed
  group per active session. Dynamic MCP connections and task listeners add
  instances, not contracts.
- `DiscordStatusActor` implements `ServiceActor` but runs a channel drain rather
  than a builder mailbox.
- Production handler registrations: **136**.
- Distinct production handled message types: **106**.

The source audit excludes test-only handlers, including
`McpRestartForTest`, the context assembly test bridge, test-harness recorders,
and unit-test builder stubs.

## Session-family actor ownership

| Actor / group | Home | Path | Production handled contracts | Responsibility |
|---|---|---|---|---|
| `SessionPersistenceActor` | `jinn-session-turn` | `session` | `EnqueueUserMessage`, `SubmitSteeringMessage`, `EnqueueResumeTurn`, `PushChatEntry`, `SubmitHistoryMutations`, `MarkSessionInteracted`, `RetryStalledSession`, `SendToLlmProvider`, `PinChatEntry`, `UnpinChatEntry`, `LoadPersonaPickerEntries`, `StreamToken`, `StreamCompleted`, `ToolUseStarted`, `ToolCallReceived`, `ToolCallStreaming`, `ToolExecutionCompleted`, `ToolBatchCompleted`, `ToolExecutionStarted`, `ToolExecutionOutput`, `CitationsReceived`, `ChatEntryPinChanged`, `TaskListUpdated`, `SkillsLoaded`, `PromptTemplatesLoaded`, `PersonasLoaded` | Cohesive turn reducer: enqueue, history/context folds, streaming, tool continuation, retry, generation guards, and turn-path snapshot persistence. |
| `SessionStoreActor` | `jinn-session-store` | `session-store` | `SessionLoadRequested`, `LoadSessionPickerEntries`, `SessionForkRequested`, `PersistSession`, `ArchiveSession`, `ArchiveSessionTree`, `EnvironmentLoaded` | Snapshot load/hydration, fork, archive, picker loading, and explicit persistence. |
| `SearchIndexActor` | `jinn-session-store` | `search-index` | `ReindexTick` | SQLite FTS index maintenance. |
| `SessionLifecycleActor` | `jinn-session-lifecycle` | `session-lifecycle` | `RunSessionSetup`, `RunSessionTeardown`, `FinishSessionSetup`, `FinishSessionTeardown`, `CancelLifecycleCommand`, `SetSessionCwd`, `CloseSession`, `TeardownSessionTree` | Setup, teardown, cancellation, close, and cwd lifecycle. |
| `SidebarStateActor` | `jinn-sidebar` | `sidebar-state` | `SessionRemoved` | Reconciles sidebar cursor/visual parents after authoritative session removal. |
| `McpCoordinatorActor` | `jinn-mcp-slice` | `jinn.mcp.coordinator` | `SessionLoadCompleted`, `SessionCreated`, `McpEnablementChanged`, `SessionClosed`, `SessionArchived`, `SessionTeardownFinished`, `RestartMcpServer`, `McpServerStatus`, `McpServerLog` | Reconciles configured connections and is the sole writer of MCP runtime status/stderr. |
| `McpActor` | `jinn-mcp-slice` | `jinn.mcp.connection.<session>.<seq>` | `ExecuteTool`, `McpConnectionStateProbe` | One dynamic connection actor per session/server. |
| `TokenCountActor` | `jinn-token-count` | `token-count` | `HistoryAppended`, `SessionLoadCompleted` | Token ledger/cache updates. |
| `ContextSizeActor` | `jinn-context-assembly` | `context-size` | `HistoryAppended`, `ContextOverrideChanged`, `ActiveSessionChanged`, `ChatEntryPinChanged`, `SessionLoadCompleted` | Cached assembly-size projection. |
| `ToolOrchestratorActor` | `jinn-tools` | `jinn.tools.orchestrator` | `RegisterTools`, `ExecuteToolBatch`, `CancelToolBatch`, `ToolExecutionCompleted`, `SessionClosed`, `ToolsUnregistered` | Tool registry, execution, and session-scoped cleanup. |
| `ProviderActor` | `jinn-provider-selection` | `jinn.provider.actor` | `ProviderSwitch`, `LoadProviderPickerEntries`, `LoadEndpointPickerEntries`, `RefreshEndpointPickerEntries`, `ModelsRefreshed`, `ModelCacheLoaded` | Provider/profile selection, model cache, and transient model-refresh rendering. |
| `SessionInitSupervisor` | `jinn-session-init` | `session-init-supervisor` | `SessionCreated`, `SessionSetupCompleted`, `SessionLoadCompleted`, `SessionCwdChanged`, `ScanSkills`, `RescanPromptTemplates`, `ScanContextFiles` | Discovery activation and partition routing. |
| `SessionDiscoveryWorker` | `jinn-session-init` | `jinn.discovery/<session_id>` | `RunDiscovery`, `RescanSkills`, `RescanPrompts`, `RescanContext` | Per-session resource discovery. |
| `DiscoveryNotifier` | `jinn-session-init` | `discovery-notifier` | `SessionDiscoverySettled` | Settles aggregate discovery work. |

## State ownership

| Concern | Canonical home | Current boundary |
|---|---|---|
| Authoritative live session aggregate | `jinn-session-state` | `SessionCore`, five flattened field groups, `SessionCoreEphemeral`, and `ChatSessionState` remain one coherent capture boundary. `State`/`SessionMap` in the shared kernel reference this owner. |
| Durable persistence payload | `jinn-session-state::snapshot` | `SessionSnapshot` contains revision, flat metadata, entries/attachments, and token ledger. |
| Session registry and active-session invariants | `jinn-session-state::session_map` | `SessionMap` owns live sessions, load guard, active selection, and default cwd. |
| Portable context vocabulary | `jinn-context` | Prompt templates/store, attachment path resolution, and `ContextFile` loaders. |
| Session-list projection algorithms | `jinn-session-list` | `SessionEntry`, visible tree, visual-parent repair; live reconciliation remains sidebar-owned. |
| Durable MCP enablement | session aggregate / snapshot | Remains `enabled_mcp_servers`; it is durable session configuration. |
| MCP runtime status/stderr | `jinn-mcp-msg::runtime_state` | MCP-owned cell keyed by session; coordinator is sole writer; excluded from snapshots. |
| Turn reducer | `jinn-session-turn` | One actor owns the atomic enqueue/stream/tool/retry/history/context fold. |
| Store operations | `jinn-session-store` | SQLite backend, snapshot conversion, load/fork/archive/search. |
| Lifecycle operations | `jinn-session-lifecycle` | Setup/teardown/close/cwd and process cancellation. |
| Sidebar live reconciliation | `jinn-sidebar` | `SidebarStateActor` consumes `SessionRemoved` and writes sidebar/session reconciliation fields under capabilities. |

## Canonical session-family contract homes

- `jinn-core-types`: `SessionId`, `SessionProfile`, chat/history/tool/model
  foundational values.
- `jinn-session-msg`: phase values, `SessionPhaseChanged`, `SessionClosed`,
  `SessionRemoved`, `MarkSessionInteracted`, `UserInteracted`,
  `RetryStalledSession`, setup/teardown/archive completion events.
- `jinn-session-history-msg`: history mutation, pin, citation, and task-list
  commands/events.
- `jinn-session-lifecycle-msg`: setup/teardown/close/cwd commands, lifecycle
  events, script state, and built-in lifecycle vocabulary.
- `jinn-session-store-msg`: load/fork/archive/persist commands,
  `SessionLoadCompleted`, `SessionState`, summaries, frozen tree nodes, and
  search/transcript models.
- `jinn-token-count-msg`: token records, statistics, and pure aggregation.
- `jinn-mcp-msg`: MCP commands/events/status and the live runtime-state value.

## Dependency direction

```text
core/msg/value crates
          |
          v
jinn-context, jinn-session-state, jinn-session-list
          |
          v
jinn-domain shared spine
          |
          v
session-turn / session-store / session-lifecycle / sidebar / MCP slices
          |
          v
composition
```

`jinn-domain` does not depend on `jinn-session-turn`, `jinn-session-store`, or
`jinn-session-lifecycle`. Neutral state/context crates do not depend back on
`jinn-domain`. Actor-bearing slices may consume the shared kernel spine for
`State`, `Services`, capabilities, image conversion, and intent-adjacent types.

## Source comments corrected by this inventory

- `jinn-session-turn/src/lib.rs`: remove the obsolete statement that activation
  forwards to a kernel actor; the implementation is now in the slice.
- `src/actor_wiring.rs`: replace “this kernel actor” with the session-turn slice.
