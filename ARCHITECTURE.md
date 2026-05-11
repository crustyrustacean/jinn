# Architecture

nullslop is a TUI chat application built on an intent-driven architecture with an actor bridge. User input flows through a single `IntentHandler` that validates, mutates shared state, and emits domain commands. Domain-specific actors run asynchronously, communicate via the actor host's pub/sub routing, and write their state fields directly into shared `AppState`. There is no bus.

## Data Flow

```
  Keyboard / Mouse
         │
         ▼
  Keymap (produces Intent)
         │
         ▼
  IntentHandler (sync, single match block)
    ├── validator  → passes or rejects
    ├── mutate AppState directly (scroll, cursor, mode, etc.)
    └── return IntentResult { commands }
         │
         ▼
  AppCore.sender  →  async forwarding task  →  ActorHost
                                                    │
                                          ┌─────────┴─────────┐
                                          │                   │
                                   Domain actors          other actors
                                   (session, provider,    (LLM, tool-
                                    context)               orchestrator…)
                                          │                   │
                                          ▼                   ▼
                                    write AppState        Commands/Events
                                    (shared RwLock)             │
                                          │                    │
                                          └─────────┬──────────┘
                                                    ▼
                                            TUI renderer reads AppState
```

### Message flow

```
Frontend ──Command──▶ Actor Host ──▶ Subscribed Actors
                                        │
                                        ▼  (actor-to-actor via same pub/sub)
                                   Domain actors write AppState (RwLock)
                                        │
                                        ▼
                                   Renderer reads AppState (next tick)

Unidirectional: frontend → actor system. No feedback loop.
The shared AppState is the feedback — domain actors write their fields,
the renderer reads it on the next tick.
```

## Intent Handler

The `IntentHandler` (in `nullslop-intent`) is the single decision point for all user input. Its `handle` method takes an `Intent`, validates it, mutates `AppState` directly, and returns an `IntentResult` carrying zero or more domain `Command`s:

```rust
pub fn handle(intent: &Intent, state: &mut AppState) -> IntentResult
```

An intent does exactly one of three things:

1. **Fail validation** → no state change, no commands
2. **Mutate `AppState` directly** (pure UI: scroll, cursor, mode switch) → no commands
3. **Mutate `AppState` + emit `Command`s** (domain: submit message, switch provider) → commands forwarded to the actor system

The `IntentHandler` never accesses external services. It never emits events. It only reads/writes `AppState` and returns commands for the caller to send.

## Validators

Each intent has a dedicated validator function in `nullslop-intent/src/validators/`. Validators are plain functions — no registries or trait objects:

- **Infallible validators** — always succeed, no validation logic needed (e.g., scroll, cursor movement). These were removed in Phase 10.7. The remaining validators all have actual logic.
- **Fallible validators** — return `Result<(), SpecificError>` with a custom error enum per intent (e.g., `SubmitMessageError`, `OpenPickerError`)

All validators take `&AppState` as input. On validation failure, the `IntentHandler` match arm does nothing (no-op). Example:

```rust
// nullslop-intent/src/validators/chat_input.rs
pub fn validate_submit_message(state: &AppState) -> Result<(), SubmitMessageError> {
    if state.active_chat_input().is_empty() {
        return Err(SubmitMessageError::EmptyBuffer);
    }
    Ok(())
}
```

## Provider Actor

The Provider actor (in `actors/nullslop-provider-actor`) manages active provider selection, LLM factory, model cache, and provider picker entries. It is the **sole writer** of `active_provider`, `model_cache`, `last_refreshed_at`, and `provider_picker` entries.

**Subscriptions — 2 commands + 1 event:**

| Type    | Subscribes to       | Handler                        |
| ------- | ------------------- | ------------------------------ |
| Command | `ProviderSwitch`    | Swap active provider + factory |
| Command | `LoadPickerEntries` | Load provider picker items     |
| Event   | `ModelsRefreshed`   | Update model cache + picker    |

Needs `State` + `Services` injection (for provider registry, API keys, LLM service).

## Session Actor

The Session actor (in `actors/nullslop-session-actor`) owns the full session lifecycle: message queuing, streaming state, tool call tracking, and session persistence. It is the **sole writer** of session history, input buffers, session phase, and tool call state.

**Subscriptions — 7 commands + 7 events:**

| Type    | Subscribes to           | Handler                                  |
| ------- | ----------------------- | ---------------------------------------- |
| Command | `EnqueueUserMessage`    | Push entry, transition phase, emit `AssemblePrompt` |
| Command | `SetChatInputText`      | Update input buffer                      |
| Command | `PushChatEntry`         | Push entry + emit `ChatEntrySubmitted`   |
| Command | `PushToolResult`        | Push tool result entry                   |
| Command | `SendMessage`           | Forward as `EnqueueUserMessage`          |
| Command | `SessionLoadCompleted`  | Restore history, set active session      |
| Command | `SaveSession`           | Persist session to disk                  |
| Event   | `PromptAssembled`       | Transition to streaming + emit `SendToLlmProvider` |
| Event   | `StreamToken`           | Append token to assistant entry          |
| Event   | `StreamCompleted`       | Finish streaming phase                   |
| Event   | `ToolUseStarted`        | Begin tool call                          |
| Event   | `ToolCallReceived`      | Push tool call entry                     |
| Event   | `ToolCallStreaming`     | Append tool call delta                   |
| Event   | `ToolExecutionCompleted`| Push tool result entry                   |

Needs `State` injection (required). `SessionStoreService` is optional (for persistence).

## Context Actor

The Context actor (in `actors/nullslop-context-actor`) manages prompt assembly, strategy management, pinning, and template loading. It is the **sole writer** of strategy state blobs, prompt templates, and pinned entries.

**Subscriptions — 5 commands + 1 event:**

| Type    | Subscribes to            | Handler                              |
| ------- | ------------------------ | ------------------------------------ |
| Command | `AssemblePrompt`         | Build prompt from strategy + context |
| Command | `SwitchPromptStrategy`   | Switch strategy + emit `RestoreStrategyState` |
| Command | `RestoreStrategyState`   | Store blob + emit `StrategyStateUpdated` |
| Command | `PinChatEntry`           | Pin entry                            |
| Command | `UnpinChatEntry`         | Unpin entry                          |
| Event   | `PromptTemplatesLoaded`  | Update template store                |

Needs `State` + `StrategyFactory` injection.

## ShutdownTracker Actor

The ShutdownTracker (in `actors/nullslop-shutdown-tracker`) manages the full actor lifecycle: startup tracking and shutdown coordination.

**Subscriptions — 3 events + 1 command:**

| Type    | Subscribes to                                             |
| ------- | --------------------------------------------------------- |
| Event   | `ActorStarting`, `ActorStarted`, `ActorShutdownCompleted` |
| Command | `ProceedWithShutdown`                                     |

When all tracked actors complete their shutdown, the tracker emits `ProceedWithShutdown`. It then sends `CoreNotification::ShutdownComplete` via `CoreChannelService`, which the core's `coordinated_shutdown` function is blocking on.

## Shared State

`AppState` (in `nullslop-component`) is the single source of truth — one struct containing all UI and domain state. `State` wraps it in an `Arc<RwLock<AppState>>` for cross-thread access.

Both the synchronous intent handler (on the main thread) and the async actor system share the same `State`:

- **Intent handler** — acquires write lock, validates, mutates, returns commands, releases lock
- **Domain actors** — acquire write lock per handler, mutate their owned fields, release lock, then emit
- **Renderer** — acquires read lock on each 100ms tick, draws to screen

`AppState` implements `Default` for easy test construction.

## Communication Channels

Two service types in `nullslop-services` bridge the core and the actor system:

- **`ActorChannelService`** (core→actor) — wraps `kanal::Sender<AppMsg>`. Methods: `send_command(Command)`, `send_event(Event)`, `send(AppMsg)`. Any holder of `Services` can submit commands/events to the actor system.
- **`CoreChannelService`** (actor→core) — wraps `kanal::Sender<CoreNotification>`. Actors signal lifecycle events (e.g., `ShutdownComplete`) back to the core without polling.

Both follow the service wrapper pattern: thin struct, cheap to clone, `name()` for debugging.

## AppCore

`AppCore` (in `nullslop-core`) is minimal — exactly two fields:

```rust
pub struct AppCore {
    pub state: State,
    pub sender: Sender<AppMsg>,
}
```

No processing loop. No `tick()` method. An async forwarding task (spawned by `spawn_forwarding_task`) continuously drains the `AppMsg` channel and routes messages to the actor host. The main TUI loop is input + rendering only.

## coordinated_shutdown

`coordinated_shutdown` is a free function in `nullslop-core`. It marks shutdown as active, sends `SystemMessage::ApplicationShuttingDown` to all actors, and blocks on receiving `CoreNotification::ShutdownComplete` from the `ShutdownTracker` (via a `tokio::sync::oneshot` channel bridging the async receiver to the synchronous caller). A timeout ensures the application exits even if actors hang.

## Rendering

UI elements implement `UiElement<AppState>` from `nullslop-component-ui`. They read `AppState` and draw to a ratatui `Frame`. Elements have no knowledge of intents, commands, or events — they just read state and draw.

## Actors

Actors run asynchronously on separate threads or processes. They communicate through the actor host's pub/sub routing — no bus required.

```
Host (nullslop)                                  Actor process
────────────────                                 ──────────────
Actor host      ---> JSON over stdin/stdout -->  Actor SDK
(nullslop-actor-host)                            (nullslop-actor)
```

- **`nullslop-actor-host`** — host side: discovers actors, manages lifecycle, routes commands and events via pre-computed `HashMap` subscription tables. Source filtering prevents actors from receiving their own messages.
- **`nullslop-actor`** — SDK for authors: provides the `Actor` trait, `MessageSink`, `ActorContext`, and subscription methods (`subscribe_command::<T>()`, `subscribe_event::<T>()`)

Two host implementations: `ProcessActorHost` (subprocess, JSON over stdio) and `InMemoryActorHost` (OS thread, no serialization). All 55 payload types (28 events + 27 commands) derive `EventMsg`/`CommandMsg` traits providing compile-time-checked `TYPE_NAME`/`NAME` constants for routing.

## Crate Structure

### Common Crates

| Crate                       | Responsibility                                                                                                                                                                    |
| --------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `nullslop-protocol`         | `Command` (domain-only), `Event`, `Intent` enum, `Mode`, `Key`, `AppMsg`, `CoreNotification`, domain types                                                                        |
| `nullslop-intent`           | `IntentHandler` (match block), validators, validator errors, `IntentResult`                                                                                                       |
| `nullslop-component`        | `AppState`, `FrontendState`, `State` (RwLock wrapper), `TuiSignals`, `ChatSessionState` (deferred migration)                                                                      |
| `nullslop-component-ui`     | `UiElement` trait, `UiRegistry`                                                                                                                                                   |
| `nullslop-core`             | `AppCore` (state + sender), `coordinated_shutdown`, `spawn_forwarding_task`, `ActorMessageSink`                                                                                   |
| `nullslop-services`         | `Services` container, `ActorChannelService`, `CoreChannelService`                                                                                                                 |
| `nullslop-tui`              | Terminal, event loop, keymap (produces `Intent`), renderer, `TuiApp`                                                                                                              |
| `nullslop-actor`            | Actor SDK (`Actor` trait, `ActorContext`, `MessageSink`, `ActorRef`, `RecordingSink`)                                                                                             |
| `nullslop-actor-host`       | Actor host implementations (process-based, in-memory, fake)                                                                                                                       |
| `nullslop-cli`              | CLI argument parsing                                                                                                                                                              |

### Slice Protocol Crates

| Crate                              | Responsibility                                                                                     |
| ---------------------------------- | -------------------------------------------------------------------------------------------------- |
| `nsslice-shutdown-protocol`        | `ShutdownTrackerState`                                                                             |
| `nsslice-provider-protocol`        | `ProviderState`                                                                                    |
| `nsslice-session-management-protocol` | `PersistedSession`, `SessionStore`, `JsonlSessionStore`, `SessionStoreService`              |
| `nsslice-context-protocol`         | `PromptAssembly` trait, strategy types, `StrategyFactory`, `StrategyDiscovery`                     |
| `nsslice-dashboard-protocol`       | `DashboardState`, `ActorStatus`                                                                    |
| `nsslice-pinned-panel-protocol`    | `PinnedPanelState`                                                                                 |
| `nsslice-chat-input-box-protocol`  | `ChatInputBoxState`, `AutocompleteMatch`, `AutocompleteState`                                      |

### Slice Crates

| Crate                              | Responsibility                                                                                     |
| ---------------------------------- | -------------------------------------------------------------------------------------------------- |
| `nsslice-echo`                     | Echo actor (example/demo)                                                                          |
| `nsslice-shutdown`                 | Shutdown tracker actor                                                                             |
| `nsslice-llm`                      | LLM streaming actor                                                                                |
| `nsslice-tools`                    | Tool orchestrator actor                                                                            |
| `nsslice-provider`                 | Provider actor + LLM discover actor + UI elements                                                  |
| `nsslice-session-management`       | Session actor + persistence + intents + validators                                                 |
| `nsslice-context`                  | Context actor + prompt scan actor                                                                  |
| `nsslice-dashboard`                | Dashboard UI + intents                                                                             |
| `nsslice-pinned-panel`             | Pinned panel UI + intents + validators                                                             |
| `nsslice-chat-input-box`           | Chat input UI + intents + validators                                                               |
| `nsslice-chat-log`                 | Chat log UI (display only)                                                                         |
| `nsslice-status-bar`               | Status bar UI (display only)                                                                       |
| `nsslice-char-counter`             | Char counter UI (display only)                                                                     |
| `nsslice-picker`                   | Picker intents + validators + keymap/strategy entries                                              |
| `nsslice-chat-entry-selection`     | Chat entry selection intents + validators                                                          |
| `nsslice-navigation`               | Navigation intents                                                                                 |
| `nsslice-global`                   | Global intents (quit, toggle which-key, interrupt)                                                 |

## Keymap

The keymap (in `nullslop-tui`) binds physical keys to `Intent` variants scoped by mode (`Normal`, `Input`, `Dashboard`, `Pinned`, `Picker`). When a key matches, the which-key system produces an `Intent` and feeds it into the `IntentHandler`.

## Testing Strategy

- **Intent handler** — test `IntentHandler::handle(intent, &mut state)` directly, assert state changes and returned commands
- **Validators** — test in isolation: call validator with known state, assert `Ok`/`Err`
- **UI elements** — test with ratatui `TestBackend`: render with known state, assert buffer contents
- **State types** — unit tests for behavior (grapheme handling, cursor bounds, etc.)
- **Actors** — test with `RecordingSink` from `nullslop-actor`: send command/event, assert messages emitted

Tests follow Given/When/Then structure. See `AGENTS.md` for detailed testing patterns.
