# Merge: Actors into Slices + State Migration

> **REQUIRED READING BEFORE EACH PHASE:** Read `AGENTS.md` and `VSA-INSTRUCTIONS.md` before starting any implementation. These files contain the coding conventions, testing patterns, and the autonomous agent loop instructions that must be followed. This note exists here because context compaction during long autonomous sessions may drop earlier context.

## Problem

Actors live in `actors/` directory, separate from their owning slices. State types live in `nullslop-component`, making it a dependency bottleneck. The goal: dissolve `actors/` by moving all actors into their domain slices, move state types into slice protocol crates so `nullslop-component` shrinks, and dissolve domain library crates (`nullslop-context`, `nullslop-session`) into their owning slices. The plan moves the needle closer to the goal — it does not need to achieve the goal entirely.

## Decisions

### State types go into `-protocol` crates, not `nullslop-protocol`

Every state type belongs to its slice's protocol crate: `ShutdownTrackerState` → `nsslice-shutdown-protocol`, `DashboardState` → `nsslice-dashboard-protocol`, etc. Nothing state-related goes into `nullslop-protocol` itself — it stays clean (command/event/intent types only). This was decided during planning: the user explicitly rejected putting small state types into `nullslop-protocol`, saying "if the state type is part of a vertical slice then it should go into a slice."

### `AppState` and `FrontendState` stay in `nullslop-component`

They import field types from protocol crates. `nullslop-component` shrinks to `AppState` + `FrontendState` + `State` wrapper + `TuiSignals`. AppState is not a slice — it's a shared data structure (per the VSA definitions in `VSA-INSTRUCTIONS.md`). Some `FrontendState` fields are system-level (`mode`, `active_tab`, `should_quit`, `tui_signals`) and don't belong to any slice — that's expected and fine.

### Domain crates merge into slices

`nullslop-context` (1,918 lines, prompt assembly strategies) and `nullslop-session` (933 lines, persistence) are each only used by their single owning actor + `src/app.rs`. They move inside their slice, not as standalone crates. The user confirmed: "if a developer is thinking 'i want to add a new way to compose prompts for sending to LLMs' → prompt assembly. slices are ideally individual features that I can find easily."

### Every actor gets a slice home

Including infrastructure actors like shutdown tracker. The user's reasoning: "a slice is a domain boundary, not a UI boundary. Shutdown has its own state, its own actor, its own behavior. If I want to change shutdown logic, I go to the shutdown slice. Whether it's cross-cutting is irrelevant — it's still its own domain."

### Each slice exports a `spawn()` function

So `src/app.rs` doesn't need to know actor internals (channel types, context setup, data injection). Done after all actors are settled to avoid rework.

### Phase ordering

Simple independent actors first (proves the pattern), then absorptions into existing slices, then the big combined moves (session + context merge domain crates), then state migration, then spawning refactor, then cleanup. This was deliberate: if the simple phases go wrong, the pattern can be corrected before touching the complex ones.

## Slice Map

| Slice | Details |
|-------|---------|
| `nsslice-echo` | Echo actor (example/demo) |
| `nsslice-shutdown` | Shutdown tracker actor + `ShutdownTrackerState` in protocol crate |
| `nsslice-llm` | LLM streaming actor |
| `nsslice-tools` | Tool orchestrator actor |
| `nsslice-context` | Context actor + prompt scan actor + prompt assembly strategies (absorbs `nullslop-context`) |
| `nsslice-provider` | Provider actor + LLM discover actor + provider UI elements + picker entries + render |
| `nsslice-session-management` | Session actor + session persistence (absorbs `nullslop-session`) + `ChatSessionState` in protocol crate + session picker entries + render |
| `nsslice-dashboard` | Dashboard UI + intents + `DashboardState` in protocol crate |
| `nsslice-pinned-panel` | Pinned panel UI + intents + validators + `PinnedPanelState` in protocol crate |
| `nsslice-chat-input-box` | Chat input UI + intents + validators + `ChatInputBoxState` in protocol crate + autocomplete render |
| `nsslice-chat-log` | Chat log UI (display only) |
| `nsslice-status-bar` | Status bar UI (display only) |
| `nsslice-char-counter` | Char counter UI (display only) |
| `nsslice-picker` | Picker intents + validators + keymap/strategy entries + renders |
| `nsslice-chat-entry-selection` | Chat entry selection intents + validators |
| `nsslice-navigation` | Navigation intents |
| `nsslice-global` | Global intents (quit, toggle which-key, interrupt) |

## AppState Field Ownership After Migration

### `SessionState` — moves to `nsslice-session-management-protocol`

| Field | Type | Natural slice |
|-------|------|---------------|
| `sessions` | `HashMap<SessionId, ChatSessionState>` | `nsslice-session-management` |
| `active_session` | `SessionId` | `nsslice-session-management` |
| `session_loading` | `bool` | `nsslice-session-management` |

### `ContextAssemblyState` — moves to `nsslice-context-protocol`

| Field | Type | Natural slice |
|-------|------|---------------|
| `strategy_state` | `HashMap<(SessionId, PromptStrategyId), JsonValue>` | `nsslice-context` |
| `prompt_templates` | `PromptTemplateStore` | `nsslice-context` |

### `ProviderState` — moves to `nsslice-provider-protocol`

| Field | Type | Natural slice |
|-------|------|---------------|
| `active_provider` | `String` | `nsslice-provider` |
| `model_cache` | `Option<ModelCache>` | `nsslice-provider` |
| `last_refreshed_at` | `Option<Timestamp>` | `nsslice-provider` |
| `provider_picker` | `SelectionState<PickerEntry>` | `nsslice-provider` |

### `ShutdownCoordinatorState` — moves to `nsslice-shutdown-protocol`

| Field | Type | Natural slice |
|-------|------|---------------|
| `shutdown_tracker` | `ShutdownTrackerState` | `nsslice-shutdown` |

### `FrontendState` — stays in `nullslop-component` (fields typed from protocol crates)

| Field | Natural slice | System-level |
|-------|---------------|--------------|
| `mode` | | Yes |
| `active_tab` | | Yes |
| `should_quit` | | Yes |
| `tui_signals` | | Yes |
| `active_picker_kind` | `nsslice-picker` | |
| `pinned_panel` | `nsslice-pinned-panel` | |
| `dashboard` | `nsslice-dashboard` | |
| `default_strategy` | `nsslice-context` | |
| `all_keymap_entries` | `nsslice-picker` | |
| `keymap_picker` | `nsslice-picker` | |
| `keymap_picker_show_all` | `nsslice-picker` | |
| `keymap_picker_origin_scope` | `nsslice-picker` | |
| `session_picker` | `nsslice-session-management` | |
| `context_strategy_picker` | `nsslice-picker` | |

All fields map cleanly. No blockers.

## Acceptance Criteria

1. `actors/` directory no longer exists — all actors live in `crates/slices/`
2. Each actor has a natural slice home
3. `nullslop-context` and `nullslop-session` crates no longer exist — their code lives in owning slices
4. State types (`DashboardState`, `PinnedPanelState`, `ShutdownTrackerState`, `ChatSessionState`, `ChatInputBoxState`, `ProviderState`, `SessionState`, `ContextAssemblyState`) live in their slice's `-protocol` crate
5. `nullslop-component` shrinks to `AppState` + `FrontendState` + `State` wrapper + `TuiSignals` (~750 lines)
6. No slice-to-slice dependencies (only `-protocol` crates are shared)
7. Each actor-bearing slice exports a `spawn()` function
8. `src/app.rs` no longer imports individual actor crate types directly
9. `just test` passes — no regressions

## Implementation Phases

- [x] **Phase 1: Create `nsslice-echo`** — move echo actor from `actors/nullslop-echo` to `crates/slices/nsslice-echo`. Proves the actor-to-slice pattern before touching complex actors.
  - [ ] Create `crates/slices/nsslice-echo/` with Cargo.toml, `src/lib.rs` (copy from `actors/nullslop-echo/src/lib.rs`)
  - [ ] Add `nullslop-actor`, `nullslop-protocol`, `tokio`, `tracing`, `serde_json` as dependencies (same as current actor)
  - [ ] Add `nsslice-echo` to root `Cargo.toml` workspace members and `[dependencies]`
  - [ ] Update `src/app.rs` imports: `use nullslop_echo::EchoActor;` → `use nsslice_echo::EchoActor;` (and `EchoDirectMsg`)
  - [ ] Remove `actors/nullslop-echo` from workspace members in root `Cargo.toml`
  - [ ] Delete `actors/nullslop-echo/` directory
  - [ ] Run `just test`

- [x] **Phase 2: Create `nsslice-shutdown`** — move shutdown tracker actor + create first `-protocol` crate for state. Proves the protocol crate pattern.
  - [ ] Create `crates/slices/nsslice-shutdown-protocol/` with Cargo.toml (minimal deps: `wherror`, `error-stack`, `serde`, `jiff` as needed by `ShutdownTrackerState`)
  - [ ] Move `ShutdownTrackerState` (58 lines) from `nullslop-component/src/shutdown_tracker/state.rs` to `nsslice-shutdown-protocol/src/lib.rs`
  - [ ] Update `nullslop-component` to depend on `nsslice-shutdown-protocol` and import `ShutdownTrackerState` from there
  - [ ] Delete `nullslop-component/src/shutdown_tracker/` directory, remove `pub mod shutdown_tracker;` from `nullslop-component/src/lib.rs`
  - [ ] Create `crates/slices/nsslice-shutdown/` with Cargo.toml, move actor from `actors/nullslop-shutdown-tracker/src/lib.rs`
  - [ ] Add to root `Cargo.toml` workspace members and `[dependencies]`
  - [ ] Update `src/app.rs` imports
  - [ ] Delete `actors/nullslop-shutdown-tracker/` directory
  - [ ] Run `just test`

- [x] **Phase 3: Create `nsslice-llm`** — move LLM streaming actor (1,263 lines, the second largest). No state migration — just the actor.
  - [ ] Create `crates/slices/nsslice-llm/` with Cargo.toml, move actor from `actors/nullslop-llm/src/lib.rs`
  - [ ] Dependencies: `nullslop-actor`, `nullslop-protocol`, `nullslop-providers`, `nullslop-services`, `tokio`, `futures`, `tracing`
  - [ ] Add to root `Cargo.toml` workspace members and `[dependencies]`
  - [ ] Update `src/app.rs` imports: `LlmActor`, `LlmDirectMsg`
  - [ ] Delete `actors/nullslop-llm/` directory
  - [ ] Run `just test`

- [x] **Phase 4: Create `nsslice-tools`** — move tool orchestrator (1,475 lines, the largest single actor). No state migration.
  - [ ] Create `crates/slices/nsslice-tools/` with Cargo.toml, move actor from `actors/nullslop-tool-orchestrator/src/lib.rs`
  - [ ] Dependencies: `nullslop-actor`, `nullslop-protocol`, `tokio`, `tracing`, `serde_json`, `jiff`
  - [ ] Add to root `Cargo.toml` workspace members and `[dependencies]`
  - [ ] Update `src/app.rs` imports: `ToolOrchestratorActor`, `ToolOrchestratorDirectMsg`
  - [ ] Delete `actors/nullslop-tool-orchestrator/` directory
  - [ ] Run `just test`

- [x] **Phase 5: Absorb into `nsslice-provider`** — move provider actor + LLM discover actor into the existing slice. Create `nsslice-provider-protocol` for `ProviderState`.
  - [ ] Create `crates/slices/nsslice-provider-protocol/` with Cargo.toml
  - [ ] Move `ProviderState` (from `nullslop-component/src/app_state.rs`) into `nsslice-provider-protocol/src/lib.rs`. Requires: `nullslop-protocol` (for `PickerEntry`), `nullslop-providers` (for `ModelCache`), `nullslop-selection-widget`, `jiff`
  - [ ] Update `nullslop-component` to depend on `nsslice-provider-protocol`, import `ProviderState` from there
  - [ ] Move provider actor code into `nsslice-provider/src/actor.rs` (or inline in lib.rs — follow existing slice pattern)
  - [ ] Move LLM discover actor code into `nsslice-provider/src/discover.rs`
  - [ ] Add dependencies to `nsslice-provider/Cargo.toml`: `nullslop-actor`, `nullslop-services`, `nullslop-component`, `llm`, `wherror`, `error-stack`, `jiff`
  - [ ] Update `src/app.rs` imports: `ProviderActor`, `ProviderDirectMsg`, `DiscoverActor`, `DiscoverDirectMsg`
  - [ ] Delete `actors/nullslop-provider-actor/` and `actors/nullslop-llm-discover/` directories
  - [ ] Remove both from root `Cargo.toml` workspace members
  - [ ] Run `just test`

- [x] **Phase 6: Absorb into `nsslice-session-management`** — the biggest phase. Session actor + merge `nullslop-session` crate + state migration. ~NOTE: State migration (`ChatSessionState`, `SessionState` → protocol crate) deferred — blocked on Phase 10 (`ChatInputBoxState` protocol crate).~ Persistence types moved to `nsslice-session-management-protocol`.
  - [ ] Create `crates/slices/nsslice-session-management-protocol/` with Cargo.toml
  - [ ] Move `ChatSessionState` (1,799 lines + tests) from `nullslop-component/src/chat_session/` into the protocol crate. Move tests with the state.
  - [ ] Move `SessionState` from `nullslop-component/src/app_state.rs` into the protocol crate
  - [ ] Update `nullslop-component` to depend on `nsslice-session-management-protocol`, import `ChatSessionState` and `SessionState` from there
  - [ ] Delete `nullslop-component/src/chat_session/` directory, remove `pub mod chat_session;` from lib.rs
  - [ ] Merge `nullslop-session` crate (933 lines: `JsonlSessionStore`, `PersistedSession`, `SessionStoreService`) into `nsslice-session-management/src/` (new `persistence.rs` or `session_store.rs`)
  - [ ] Move session actor from `actors/nullslop-session-actor/src/lib.rs` into `nsslice-session-management/src/actor.rs`
  - [ ] Add dependencies: `nullslop-actor`, `nullslop-component`, `nullslop-session-management-protocol`, `jiff`, `serde_json`, `tokio`, `tracing`
  - [ ] Update `src/app.rs` imports: `SessionPersistenceActor`, `SessionPersistenceDirectMsg`, `JsonlSessionStore`, `SessionStoreService`, `PersistedSession`
  - [ ] Delete `actors/nullslop-session-actor/` and `crates/nullslop-session/` directories
  - [ ] Remove both from root `Cargo.toml` workspace members and `[workspace.dependencies]`
  - [ ] Run `just test`

- [x] **Phase 7: Create `nsslice-context`** — context actor + prompt scan actor + merge `nullslop-context` crate + state migration. ~NOTE: State migration (`ContextAssemblyState`, `PromptTemplateStore` → protocol crate) deferred.~ Strategy types moved to `nsslice-context-protocol`.
  - [ ] Create `crates/slices/nsslice-context/` with Cargo.toml
  - [ ] Create `crates/slices/nsslice-context-protocol/` with Cargo.toml
  - [ ] Move `ContextAssemblyState` from `nullslop-component/src/app_state.rs` into the protocol crate
  - [ ] Move `PromptTemplateStore` from `nullslop-component/src/prompt_template/` into the protocol crate (or keep in `nullslop-prompt-template` — assess which is cleaner at implementation time)
  - [ ] Update `nullslop-component` to depend on `nsslice-context-protocol`
  - [ ] Merge `nullslop-context` crate (1,918 lines: prompt assembly strategies, `PromptAssembly` trait, `StrategyFactory`, all strategy implementations) into `nsslice-context/src/strategy/`
  - [ ] Move context actor from `actors/nullslop-context-actor/src/lib.rs` into `nsslice-context/src/actor.rs`
  - [ ] Move prompt scan actor from `actors/nullslop-prompt-scan/src/lib.rs` into `nsslice-context/src/prompt_scan.rs`
  - [ ] Add dependencies: `nullslop-actor`, `nullslop-component`, `nullslop-context-protocol`, `nullslop-services`, `nullslop-prompt-template`, `serde_json`, `tokio`, `tracing`, `wherror`, `error-stack`
  - [ ] Update `src/app.rs` imports: `PromptAssemblyActor`, `ContextDirectMsg`, `PromptScanActor`, `PromptScanDirectMsg`, `DefaultStrategyFactory`, `DefaultStrategyDiscovery`
  - [ ] Delete `actors/nullslop-context-actor/`, `actors/nullslop-prompt-scan/`, `crates/nullslop-context/` directories
  - [ ] Remove all three from root `Cargo.toml` workspace members and `[workspace.dependencies]`
  - [ ] Run `just test`

- [x] **Phase 8: Migrate `DashboardState`** — create protocol crate for existing slice.
  - [ ] Create `crates/slices/nsslice-dashboard-protocol/` with Cargo.toml
  - [ ] Move `DashboardState` (309 lines) from `nullslop-component/src/dashboard/state.rs` into the protocol crate
  - [ ] Update `nullslop-component` to depend on `nsslice-dashboard-protocol`, import `DashboardState` from there
  - [ ] Update `nsslice-dashboard` to depend on its own protocol crate instead of `nullslop-component` for `DashboardState`
  - [ ] Delete `nullslop-component/src/dashboard/` directory, remove `pub mod dashboard;` from lib.rs
  - [ ] Run `just test`

- [x] **Phase 9: Migrate `PinnedPanelState`** — create protocol crate for existing slice.
  - [ ] Create `crates/slices/nsslice-pinned-panel-protocol/` with Cargo.toml
  - [ ] Move `PinnedPanelState` (297 lines) from `nullslop-component/src/pinned_panel/state.rs` into the protocol crate
  - [ ] Update `nullslop-component` to depend on `nsslice-pinned-panel-protocol`, import `PinnedPanelState` from there
  - [ ] Update `nsslice-pinned-panel` to depend on its own protocol crate instead of `nullslop-component` for `PinnedPanelState`
  - [ ] Delete `nullslop-component/src/pinned_panel/` directory, remove `pub mod pinned_panel;` from lib.rs
  - [ ] Run `just test`

- [x] **Phase 10: Migrate `ChatInputBoxState`** — create protocol crate for existing slice. Largest state migration.
  - [ ] Create `crates/slices/nsslice-chat-input-box-protocol/` with Cargo.toml
  - [ ] Move `ChatInputBoxState` (1,039 lines + 451 lines tests) from `nullslop-component/src/chat_input_box/` into the protocol crate. Move tests with the state.
  - [ ] Update `nullslop-component` to depend on `nsslice-chat-input-box-protocol`, import `ChatInputBoxState` from there
  - [ ] Update `nsslice-chat-input-box` to depend on its own protocol crate instead of `nullslop-component` for `ChatInputBoxState`
  - [ ] Delete `nullslop-component/src/chat_input_box/` directory, remove `pub mod chat_input_box;` from lib.rs
  - [ ] Run `just test`

- [x] **Phase 11: Refactor spawning** — each actor-bearing slice exports a `spawn()` function.
  - [ ] Define the spawn function signature convention. Each function takes the dependencies it needs (services, state, handle) and returns the `ActorResult` + `ActorRef`. Example:
    ```rust
    // In nsslice-echo/src/lib.rs
    pub fn spawn(
        sink: Arc<dyn MessageSink>,
        handle: &tokio::runtime::Handle,
    ) -> (ActorRef<EchoDirectMsg>, ActorResult) { ... }
    ```
  - [ ] Add `spawn()` to `nsslice-echo`
  - [ ] Add `spawn()` to `nsslice-shutdown`
  - [ ] Add `spawn()` to `nsslice-llm`
  - [ ] Add `spawn()` to `nsslice-tools`
  - [ ] Add `spawn()` to `nsslice-provider` (spawns both provider + discover actors)
  - [ ] Add `spawn()` to `nsslice-session-management`
  - [ ] Add `spawn()` to `nsslice-context` (spawns both context + prompt-scan actors)
  - [ ] Simplify `src/app.rs` `create_core_with_actor_host()` to call slice spawn functions instead of inline channel creation, context setup, and actor activation
  - [ ] Run `just test`

- [x] **Phase 12: Dissolve `actors/` directory** — remove all actor crates and old domain crates from the workspace. ~Completed incrementally during Phases 5-7.~
  - [ ] Verify all 9 actor directories are gone from `actors/`
  - [ ] Verify `crates/nullslop-context/` and `crates/nullslop-session/` are gone
  - [ ] Remove all `actors/*` from workspace members glob in root `Cargo.toml`
  - [ ] Remove all actor crate entries from `[workspace.dependencies]`
  - [ ] Remove `nullslop-context` and `nullslop-session` from `[workspace.dependencies]`
  - [ ] Remove actor crate entries from root `[dependencies]`
  - [ ] Remove `nullslop-context` and `nullslop-session` from root `[dependencies]`
  - [ ] Remove all `use nullslop_*_actor::` imports from `src/app.rs`
  - [ ] If `actors/` directory is empty, delete it
  - [ ] Run `just test`

- [x] **Phase 13: Final cleanup** — remove empty shells from `nullslop-component`, update docs.
  - [ ] Remove empty state subdirectories: `chat_session/`, `chat_input_box/`, `dashboard/`, `pinned_panel/`, `shutdown_tracker/`, `prompt_template/`
  - [ ] Update `nullslop-component/src/lib.rs` — remove all `pub mod` declarations for deleted directories, remove re-exports that moved to protocol crates
  - [ ] Verify `nullslop-component` is down to `app_state.rs`, `state.rs`, `tui_signals.rs`, `lib.rs`
  - [ ] Update `ARCHITECTURE.md` crate table to reflect new structure
  - [ ] Update `AGENTS.md` module structure section
  - [ ] Run `just test`

- [x] **Phase 14: Move non-slice crates into `crates/common/`** — reorganize directory structure so the workspace has two clear top-level groups: `common` and `slices`.
  - [ ] Create `crates/common/` directory
  - [ ] Move all non-slice crates from `crates/` into `crates/common/`: `nullslop-protocol`, `nullslop-protocol-derive`, `nullslop-component`, `nullslop-component-ui`, `nullslop-core`, `nullslop-intent`, `nullslop-services`, `nullslop-tui`, `nullslop-actor`, `nullslop-actor-host`, `nullslop-cli`, `nullslop-providers`, `nullslop-selection-widget`, `nullslop-workflow`, `nullslop-prompt-template`
  - [ ] Update all `path = "crates/..."` references in root `Cargo.toml` to `path = "crates/common/..."`
  - [ ] Update all `path = "actors/..."` references (should already be gone after Phase 12)
  - [ ] Update `members` glob in root `Cargo.toml`: `members = ["crates/common/*", "crates/slices/*"]`
  - [ ] Update `AGENTS.md` module structure and `ARCHITECTURE.md` to reflect the new layout
  - [ ] Run `just test`
