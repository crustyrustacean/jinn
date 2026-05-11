# Plan: Mega-Merge — `nullslop-domain`

## Problem

`nullslop-protocol` and 24 `nsslice-*` slice crates form a tangled dependency web. Protocol types (`ChatEntry`, `SessionId`, `Command`, `Event`) live in one crate while the slice logic that uses them is spread across many crates. Working on a single feature requires touching `nullslop-protocol` for types, `nsslice-*-protocol` for state, `nsslice-*` for actors/intents/elements, and `nullslop-component` for `AppState` integration. This creates excessive rummaging and layers of indirection for no meaningful benefit.

## Solution

Merge all 25 crates (24 slices + `nullslop-protocol`) plus `nullslop-component-ui` into a single crate called **`nullslop-domain`**. The module tree mirrors the current slice/protocol structure. External crates (`nullslop-actor`, `nullslop-actor-host`, `nullslop-component`, `nullslop-tui`, `nullslop-services`, `nullslop-intent`, `nullslop-core`, `nullslop-providers`, `nullslop-selection-widget`, etc.) become dependencies of the mega crate and stay separate.

### Key decisions (from conversation)

- **Crate name**: `nullslop-domain` — "the stuff that makes everything work"
- **Protocol types re-exported at crate root**: Types like `Command`, `Event`, `SessionId`, `ChatEntry`, etc. live in `src/protocol/` internally but are re-exported from `lib.rs` so consumers can write `use nullslop_domain::Command` instead of `use nullslop_domain::protocol::Command`
- **`nullslop-component-ui` merges in**: Its 4 files (`UiElement` trait, `UiRegistry`, fake, element) move into the mega crate since slices implement `UiElement` and it currently depends on `nullslop-protocol` anyway

## Acceptance Criteria

- [x] All 25 crates (`nullslop-protocol` + 24 `nsslice-*`) and `nullslop-component-ui` are absorbed into `nullslop-domain` (code merged)
- [x] `nullslop-domain` module tree mirrors the current slice/protocol directory structure
- [x] Protocol types are re-exported from the crate root (e.g., `nullslop_domain::Command` works and equals `nullslop_protocol::Command`)
- [x] No circular dependencies in the migrated crates
- [x] Downstream crates updated to depend on `nullslop-domain`: nullslop-intent, nullslop-tui, nullslop-core, root binary, nullslop-e2e
- [x] `just test` passes (1694 unit tests + 10 cucumber tests)
- [x] 10 of 24 old slice crates removed from workspace
- [ ] **BLOCKED**: Remaining 15 slice crates + nullslop-protocol + nullslop-component-ui still exist due to circular deps with nullslop-component, nullslop-services, nullslop-providers, nullslop-actor, nullslop-actor-host
- [ ] **BLOCKED**: Full `just ci` (blocked on same circular dep resolution)

---

## Implementation Phases

- [x] Phase 1: Create `nullslop-domain` skeleton with merged `Cargo.toml`
  - [ ] Create `crates/common/nullslop-domain/Cargo.toml` with the union of all dependencies from the 26 source crates (25 slices/protocol + component-ui)
  - [ ] The merged deps from the source crates (external dependencies):
    - `serde`, `serde_json` (from protocol)
    - `jiff` (from protocol, session-management-protocol)
    - `derive_more` (from protocol)
    - `ratatui-which-key` optional feature (from protocol)
    - `uuid` (from protocol)
    - `nullslop-protocol-derive` (from protocol — proc-macro stays separate)
    - `nullslop-selection-widget` (from protocol, picker, provider, chat-input-box, session-management)
    - `ratatui` (from protocol, component-ui, all UI slices)
    - `nullslop-actor` (from actor slices: echo, shutdown, llm, context, tools, session-management, provider)
    - `nullslop-actor-host` (from actor slices: echo, shutdown, llm, context, tools, session-management, provider)
    - `nullslop-component` (from intent/UI slices: chat-input-box, chat-log, dashboard, picker, pinned-panel, chat-entry-selection, navigation, session-management, global, shutdown)
    - `nullslop-services` (from actor slices: shutdown, llm, context, session-management, provider, picker)
    - `nullslop-providers` (from provider slice, provider-protocol, status-bar)
    - `nullslop-prompt-template` (from context slice)
    - `async-trait` (from context-protocol)
    - `wherror`, `error-stack` (from context-protocol, session-management-protocol)
    - `dirs` (from session-management-protocol)
    - `tokio` (from various actor slices)
    - `tracing` (from context slice)
    - `kanal` (not directly — comes via actor/actor-host)
    - `parking_lot` (not directly — comes via actor/actor-host)
    - `unicode-segmentation` (from chat-input-box)
    - `throbber-widgets-tui` (from provider — for streaming indicator)
    - `humantime` (from provider — for time formatting)
    - `fuzzy-matcher` (from picker — for search)
  - [ ] Create `src/lib.rs` with the module structure and re-exports (see module tree below)
  - [ ] Add `nullslop-domain` to workspace `Cargo.toml` members and `[workspace.dependencies]`
  - [ ] Verify `cargo check -p nullslop-domain` compiles (empty modules are fine at this stage)

- [x] Phase 2: Move `nullslop-protocol` contents into `nullslop-domain/src/protocol/`
  - [ ] Copy all 59 files from `nullslop-protocol/src/` into `nullslop-domain/src/protocol/`
  - [ ] Fix all internal `crate::` references to become `crate::protocol::` where needed
  - [ ] Add re-exports from `lib.rs`: `pub use protocol::{Command, Event, AppMsg, Intent, IntentResult, ...}` for all currently public types
  - [ ] Move protocol tests (`chat_tests.rs`, `custom/derive_tests.rs`) into the protocol subtree
  - [ ] Verify `cargo check -p nullslop-domain` passes
  - [ ] Verify `cargo test -p nullslop-domain` passes (protocol tests only at this stage)

- [x] Phase 3: Merge in `nullslop-component-ui`
  - [ ] Copy `element.rs`, `registry.rs`, `fake.rs` from `nullslop-component-ui/src/` into `nullslop-domain/src/component_ui/` (new module)
  - [ ] Add `pub mod component_ui;` and re-export `UiElement`, `UiRegistry` from `lib.rs`
  - [ ] Fix any `crate::` references in the moved files
  - [ ] Verify `cargo check -p nullslop-domain` passes

- [x] Phase 4: Merge in protocol-only slices (leaf crates with no actor/intent/element code)
  These are the small `-protocol` crates that only define types/state. They merge into their domain's module directory.
  - [ ] `nsslice-shutdown-protocol` → `src/shutdown/` (ShutdownTrackerState)
  - [ ] `nsslice-dashboard-protocol` → `src/dashboard/` (DashboardState, ActorEntry, ActorStatus)
  - [ ] `nsslice-chat-input-box-protocol` → `src/chat_input_box/` (ChatInputBoxState, AutocompleteState, AutocompleteMatch)
  - [ ] `nsslice-pinned-panel-protocol` → `src/pinned_panel/` (PinnedPanelState)
  - [ ] `nsslice-provider-protocol` → `src/provider/` (ProviderState)
  - [ ] `nsslice-session-management-protocol` → `src/session/` (PersistedSession, SessionStore trait, JsonlSessionStore)
  - [ ] `nsslice-chat-session-protocol` → `src/chat_session/` (ChatSessionState — merges alongside nsslice-chat-input-box-protocol's state)
  - [ ] `nsslice-context-protocol` → `src/context/` (PromptAssembly trait, strategy types, token estimator)
  - [ ] Fix all `crate::` references and update imports from `nullslop_protocol::` to `crate::protocol::` within these files
  - [ ] Verify `cargo check -p nullslop-domain` passes

- [x] Phase 5: Merge in actor slices (echo, shutdown, llm, context, tools, session-management, provider)
  These contain actors that implement `Actor` trait from `nullslop-actor`.
  - [ ] `nsslice-echo` → `src/echo/` (EchoActor, spawn fn)
  - [ ] `nsslice-shutdown` → `src/shutdown/` (ShutdownTrackerActor — merges alongside ShutdownTrackerState from phase 4)
  - [ ] `nsslice-llm` → `src/llm/` (LlmActor, session module)
  - [ ] `nsslice-tools` → `src/tools/` (ToolOrchestratorActor, builtin module)
  - [ ] `nsslice-context` → `src/context/` (ContextActor, prompt_scan — merges alongside strategy types from phase 4)
  - [ ] `nsslice-session-management` → `src/session/` (SessionActor, persistence, entries, intent, validator — merges alongside SessionStore from phase 4)
  - [ ] `nsslice-provider` → `src/provider/` (ProviderActor, DiscoverActor, UI elements, entries — merges alongside ProviderState from phase 4)
  - [ ] Fix all `crate::` references and update imports from `nullslop_protocol::` to `crate::protocol::`
  - [ ] Fix imports from `nsslice_*_protocol::` to `crate::` (now same crate)
  - [ ] Verify `cargo check -p nullslop-domain` passes

- [x] Phase 6: Merge in intent-only slices (global, navigation, chat-entry-selection, picker)
  - [ ] `nsslice-global` → `src/global/` (intent + validator for quit/interrupt/which-key)
  - [ ] `nsslice-navigation` → `src/navigation/` (intent for scroll/tab/external-editor)
  - [ ] `nsslice-chat-entry-selection` → `src/chat_entry_selection/` (intent + validator)
  - [ ] `nsslice-picker` → `src/picker/` (intent + validator + render + entry loading)
  - [ ] Fix imports
  - [ ] Verify `cargo check -p nullslop-domain` passes

- [x] Phase 7: Merge in UI element slices (chat-input-box, chat-log, dashboard, pinned-panel, status-bar, char-counter)
  - [ ] `nsslice-chat-input-box` → `src/chat_input_box/` (element + intent + validator + autocomplete render — merges alongside ChatInputBoxState from phase 4)
  - [ ] `nsslice-chat-log` → `src/chat_log/` (ChatLogElement)
  - [ ] `nsslice-dashboard` → `src/dashboard/` (DashboardElement + intent — merges alongside DashboardState from phase 4)
  - [ ] `nsslice-pinned-panel` → `src/pinned_panel/` (PinnedPanelElement + intent + validator — merges alongside PinnedPanelState from phase 4)
  - [ ] `nsslice-status-bar` → `src/status_bar/` (StatusBarElement)
  - [ ] `nsslice-char-counter` → `src/char_counter/` (CharCounterElement)
  - [ ] Fix imports — these use `UiElement` from `nullslop-component-ui`, now at `crate::component_ui::`
  - [ ] Verify `cargo check -p nullslop-domain` passes

- [x] Phase 8: Move all tests from merged crates into `nullslop-domain`
  Tests were moved alongside their code in phases 5-7. 575 tests pass.

- [ ] Phase 9: Update downstream crates to depend on `nullslop-domain`
  **BLOCKED by circular dependencies.** `nullslop-domain` depends on `nullslop-component`, `nullslop-services`, `nullslop-actor`, `nullslop-actor-host`, `nullslop-providers`. These crates CANNOT depend back on `nullslop-domain` without creating cycles.
  
  Completed so far:
  - [x] `nullslop-intent` — successfully migrated (nullslop-intent → nullslop-domain → nullslop-component works)
  
  Blocked (circular):
  - [ ] `nullslop-component` — CYCLE: nullslop-domain → nullslop-component → nullslop-domain
  - [ ] `nullslop-services` — CYCLE: nullslop-domain → nullslop-services → nullslop-domain
  - [ ] `nullslop-providers` — CYCLE: nullslop-domain → nullslop-providers → nullslop-domain
  - [ ] `nullslop-actor` — CYCLE: nullslop-domain → nullslop-actor → nullslop-domain
  - [ ] `nullslop-actor-host` — CYCLE: nullslop-domain → nullslop-actor-host → nullslop-domain
  - [ ] `nullslop-prompt-template` — CYCLE: nullslop-domain → nullslop-component → nullslop-prompt-template → nullslop-domain
  
  Remaining (unblocked, need to try):
  - [x] `nullslop-tui` — replace nullslop-protocol + 9 nsslice-* deps with nullslop-domain
  - [x] `nullslop-core` — replace nullslop-protocol + nullslop-component-ui
  - [x] Root `nullslop` binary crate — replace 25+ slice/protocol deps with nullslop-domain
  - [x] `nullslop-e2e` test crate — replace nullslop-protocol + 5 nsslice-* deps

  **Resolution path for blocked crates**: The blocked crates need the dependency direction reversed. `nullslop-domain` currently depends on `nullslop-component` (for AppState) and `nullslop-services` (for Services DI). To break the cycle, AppState and Services types would need to move into `nullslop-domain` or a new shared crate. This is a separate, significant refactor beyond the scope of this plan.

- [ ] Phase 10: Remove old crates and cleanup
  **PARTIALLY DONE.** 10 of 24 slice crates removed. Remaining crates still referenced by blocked downstream crates.
  Removed:
  - [x] nsslice-char-counter, nsslice-chat-entry-selection, nsslice-chat-log
  - [x] nsslice-echo, nsslice-global, nsslice-llm
  - [x] nsslice-navigation, nsslice-picker, nsslice-status-bar, nsslice-tools
  
  Still present (used by blocked crates):
  - [ ] nsslice-chat-input-box (used by nullslop-component, nullslop-tui)
  - [ ] nsslice-chat-input-box-protocol (used by nullslop-component)
  - [ ] nsslice-chat-session-protocol (used by nullslop-component)
  - [ ] nsslice-context (used by nullslop-services)
  - [ ] nsslice-context-protocol (used by nullslop-services)
  - [ ] nsslice-dashboard (used by nullslop-component)
  - [ ] nsslice-dashboard-protocol (used by nullslop-component)
  - [ ] nsslice-pinned-panel (used by nullslop-component)
  - [ ] nsslice-pinned-panel-protocol (used by nullslop-component)
  - [ ] nsslice-provider (used by nullslop-component)
  - [ ] nsslice-provider-protocol (used by nullslop-component)
  - [ ] nsslice-session-management (used by nullslop-component)
  - [ ] nsslice-session-management-protocol (used by nullslop-services)
  - [ ] nsslice-shutdown (used by nullslop-component)
  - [ ] nsslice-shutdown-protocol (used by nullslop-component)
  
  Cannot remove until circular dependency issue is resolved (Phase 9 blocked items).
  - [ ] Remove all old crate entries from workspace `Cargo.toml` `[workspace.dependencies]`
  - [ ] Update `workspace.members` glob patterns (remove `crates/slices/*`, keep `crates/common/*`)
  - [ ] Run `just ci` to verify everything passes

---

## Proposed Module Tree

```
nullslop-domain/src/
├── lib.rs                          # Re-exports all public types at crate root
│
├── protocol/                       # ← from nullslop-protocol (59 files)
│   ├── mod.rs
│   ├── command.rs                  # Command mega-enum
│   ├── event.rs                    # Event mega-enum
│   ├── app_msg.rs                  # AppMsg
│   ├── intent.rs                   # Intent enum
│   ├── intent_result.rs            # IntentResult
│   ├── core_notification.rs        # CoreNotification
│   ├── action.rs                   # CommandAction
│   ├── key.rs                      # Key, KeyEvent, Modifiers
│   ├── mode.rs                     # Mode
│   ├── actor_name.rs               # ActorName
│   ├── picker_kind.rs              # PickerKind
│   ├── prompt_template.rs          # PromptTemplate
│   ├── chat.rs                     # ChatEntry, ChatEntryId, ChatEntryKind, PinPosition
│   ├── chat_tests.rs
│   ├── tab/                        # ActiveTab, TabDirection
│   ├── custom/                     # CommandMsg, EventMsg traits + derive tests
│   ├── actor/                      # Actor lifecycle payloads
│   ├── chat_input/                 # Chat input payloads
│   ├── context/                    # Context payloads + PromptStrategyId
│   ├── provider/                   # Provider payloads + LlmMessage + entries_to_messages
│   ├── session/                    # Session payloads + SessionId
│   ├── system/                     # System payloads (KeyDown, ModeChanged, LoadPickerEntries)
│   ├── tool/                       # Tool payloads + ToolCall, ToolResult, ToolDefinition
│   ├── provider_picker/            # PickerEntry + PickerItem impl + render
│   ├── session_picker/             # SessionEntry + PickerItem impl + render
│   ├── context_strategy_picker/    # StrategyEntry + PickerItem impl + render
│   └── keymap_picker/              # KeymapEntry + PickerItem impl + render
│
├── component_ui/                   # ← from nullslop-component-ui (4 files)
│   ├── mod.rs
│   ├── element.rs                  # UiElement trait
│   ├── registry.rs                 # UiRegistry
│   └── fake.rs                     # FakeUiElement for tests
│
├── provider/                       # ← nsslice-provider + nsslice-provider-protocol (11 files)
│   ├── mod.rs
│   ├── state.rs                    # ProviderState (from -protocol)
│   ├── actor.rs                    # ProviderActor
│   ├── discover.rs                 # DiscoverActor
│   ├── entries.rs                  # Provider picker entry loading
│   ├── entries_tests.rs
│   ├── indicator.rs                # StreamingIndicatorElement
│   ├── loader.rs                   # Model loader
│   ├── queue_element.rs            # QueueDisplayElement
│   ├── render.rs                   # Provider render
│   └── render_tests.rs
│
├── session/                        # ← nsslice-session-management + protocol (17 files)
│   ├── mod.rs
│   ├── persisted_session.rs        # PersistedSession, SessionSummary (from -protocol)
│   ├── session_store/              # SessionStore trait + JsonlSessionStore (from -protocol)
│   │   ├── mod.rs
│   │   ├── jsonl.rs
│   │   └── service.rs
│   ├── actor/                      # SessionActor
│   │   ├── mod.rs
│   │   └── handlers/               # command, event, persistence handlers
│   ├── entries.rs                  # Session picker entry loading
│   ├── intent.rs                   # Session management intents
│   ├── persistence.rs              # Persistence logic
│   ├── render.rs                   # Session render
│   └── validator.rs                # Session validators
│
├── context/                        # ← nsslice-context + nsslice-context-protocol (19 files)
│   ├── mod.rs
│   ├── strategy/                   # Prompt assembly strategies (from -protocol)
│   │   ├── mod.rs
│   │   ├── types.rs
│   │   ├── discovery.rs
│   │   ├── factory.rs
│   │   ├── passthrough.rs
│   │   ├── sliding_window.rs
│   │   ├── token_budget.rs
│   │   ├── token_estimator.rs
│   │   ├── compaction.rs
│   │   └── compaction_data.rs
│   ├── actor/                      # ContextActor
│   │   ├── mod.rs
│   │   └── handlers/               # assembly, caching, pinning, strategy
│   └── prompt_scan.rs              # PromptScanActor
│
├── llm/                            # ← nsslice-llm (2 files)
│   ├── mod.rs
│   └── session.rs
│
├── tools/                          # ← nsslice-tools (2 files)
│   ├── mod.rs
│   └── builtin.rs
│
├── echo/                           # ← nsslice-echo (1 file)
│   └── mod.rs
│
├── shutdown/                       # ← nsslice-shutdown + nsslice-shutdown-protocol (2 files)
│   ├── mod.rs                      # ShutdownTrackerActor
│   └── state.rs                    # ShutdownTrackerState
│
├── chat_input_box/                 # ← nsslice-chat-input-box + nsslice-chat-input-box-protocol (8 files)
│   ├── mod.rs
│   ├── state/                      # ChatInputBoxState, AutocompleteState (from -protocol)
│   │   ├── mod.rs
│   │   ├── chat_input_box.rs
│   │   └── autocomplete.rs
│   ├── element.rs                  # ChatInputBoxElement
│   ├── intent.rs                   # Chat input intents (16 intents)
│   ├── validator.rs                # Input validators
│   ├── autocomplete_render.rs      # Autocomplete popup render
│   └── autocomplete_render_tests.rs
│
├── chat_log/                       # ← nsslice-chat-log (1 file)
│   └── mod.rs                      # ChatLogElement
│
├── chat_entry_selection/           # ← nsslice-chat-entry-selection (2 files)
│   ├── mod.rs
│   ├── intent.rs
│   └── validator.rs
│
├── chat_session/                   # ← nsslice-chat-session-protocol (2 files)
│   ├── mod.rs
│   └── tests.rs
│
├── dashboard/                      # ← nsslice-dashboard + nsslice-dashboard-protocol (4 files)
│   ├── mod.rs
│   ├── state.rs                    # DashboardState, ActorEntry, ActorStatus (from -protocol)
│   ├── element.rs                  # DashboardElement
│   └── intent.rs                   # Dashboard intents
│
├── global/                         # ← nsslice-global (2 files)
│   ├── mod.rs
│   ├── intent.rs
│   └── validator.rs
│
├── navigation/                     # ← nsslice-navigation (1 file)
│   ├── mod.rs
│   └── intent.rs
│
├── picker/                         # ← nsslice-picker (7 files)
│   ├── mod.rs
│   ├── intent.rs
│   ├── validator.rs
│   ├── render.rs
│   ├── picker_render_tests.rs
│   ├── keymap_entries.rs
│   ├── strategy_entries.rs
│   └── strategy_entries_tests.rs
│
├── pinned_panel/                   # ← nsslice-pinned-panel + nsslice-pinned-panel-protocol (5 files)
│   ├── mod.rs
│   ├── state.rs                    # PinnedPanelState (from -protocol)
│   ├── element.rs                  # PinnedPanelElement
│   ├── intent.rs
│   └── validator.rs
│
├── status_bar/                     # ← nsslice-status-bar (1 file)
│   └── mod.rs                      # StatusBarElement
│
└── char_counter/                   # ← nsslice-char-counter (1 file)
    └── mod.rs                      # CharCounterElement
```

## Dependency Graph (after merge)

```
External crates that STAY separate:
  nullslop-protocol-derive (proc-macro — must be separate)
  nullslop-actor           (generic actor runtime)
  nullslop-actor-host      (generic actor host trait)
  nullslop-component       (AppState + state management — consumes domain)
  nullslop-tui             (terminal event loop, keymap, rendering)
  nullslop-intent          (IntentHandler — dispatches to domain intent fns)
  nullslop-core            (app core loop)
  nullslop-services        (DI container)
  nullslop-providers       (LLM provider implementations — heavy external deps)
  nullslop-selection-widget (generic ratatui picker widget)
  nullslop-prompt-template  (prompt template loading)
  nullslop-cli             (CLI arg parsing)
  nullslop-workflow         (workflow engine)

New merged crate:
  nullslop-domain  →  nullslop-actor, nullslop-actor-host, nullslop-component,
                       nullslop-services, nullslop-providers, nullslop-selection-widget,
                       nullslop-prompt-template, nullslop-protocol-derive
                       + ratatui, ratatui-which-key (opt), serde, jiff, uuid, etc.

Downstream deps updated:
  nullslop-component  →  nullslop-domain (replaces protocol + 7 nsslice-*-protocol deps)
  nullslop-intent     →  nullslop-domain (replaces protocol + 8 nsslice-* deps)
  nullslop-tui        →  nullslop-domain (replaces protocol + component-ui + 9 nsslice-* deps)
  nullslop-services   →  nullslop-domain (replaces protocol + 2 nsslice-* deps)
  nullslop-core       →  nullslop-domain (replaces protocol + component-ui)
  nullslop-providers  →  nullslop-domain (replaces protocol)
  nullslop-actor      →  nullslop-domain (replaces protocol)
  nullslop-actor-host →  nullslop-domain (replaces protocol)
  nullslop-prompt-template →  nullslop-domain (replaces protocol)
  nullslop (binary)   →  nullslop-domain (replaces 25+ deps)
  nullslop-e2e        →  nullslop-domain (replaces protocol + 5 nsslice-* deps)
```

## Import migration guide

For downstream crates, the migration is mechanical:

```rust
// BEFORE
use nullslop_protocol::{Command, Event, SessionId, ChatEntry};
use nsslice_shutdown_protocol::ShutdownTrackerState;
use nsslice_provider_protocol::ProviderState;
use nsslice_context_protocol::PromptAssembly;

// AFTER
use nullslop_domain::{Command, Event, SessionId, ChatEntry};
use nullslop_domain::shutdown::ShutdownTrackerState;
use nullslop_domain::provider::ProviderState;
use nullslop_domain::context::PromptAssembly;
```

Protocol types are re-exported at the crate root, so `nullslop_domain::Command` works directly. Domain-specific types are accessed via their module path.

## Scale

- **Total source files**: ~160 files (59 protocol + 97 slices + 4 component-ui)
- **Total lines of code**: ~27,500 lines (5,100 protocol + 22,400 slices + ~200 component-ui)
- **Crates eliminated**: 26 (24 slices + nullslop-protocol + nullslop-component-ui)
- **Downstream crates updated**: 11

## Risks & mitigations

- **Compile time regression**: Any change to any type in `nullslop-domain` forces recompilation of all downstream crates. This is the accepted tradeoff for colocating code. Mitigated by the fact that `nullslop-domain` is a library crate — Rust incremental compilation still works within the crate's modules.
- **Import path churn**: Every downstream crate needs import updates. This is mechanical and can be done with find/replace patterns.
- **Feature flag propagation**: `nullslop-protocol` has a `which-key` feature. This becomes a feature on `nullslop-domain` instead.
- **Test discovery**: Tests move from individual crates into the mega crate's test tree. `cargo test -p nullslop-domain` runs all of them at once.
