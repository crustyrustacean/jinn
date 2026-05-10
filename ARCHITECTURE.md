# Architecture

nullslop is a TUI chat application built on an intent-driven architecture with an actor bridge. User input flows through a single `IntentHandler` that validates, mutates shared state, and emits domain commands. Actors run asynchronously and communicate via the actor host's pub/sub routing. A dedicated `Projector` actor writes events back into shared state. There is no bus.

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
                                    Coordinator          other actors
                                    (subscribes to        (LLM, session,
                                     Commands)             tool-orchestrator…)
                                          │                   │
                                          ▼                   ▼
                                    new Commands/Events   Commands/Events
                                          │                   │
                                          └─────────┬─────────┘
                                                    ▼
                                            Projector (subscribes to Events)
                                                    │
                                                    ▼
                                            writes AppState (shared RwLock)
                                                    │
                                                    ▼
                                            TUI renderer reads AppState
```

### Message flow

```
Frontend ──Command──▶ Actor Host ──▶ Subscribed Actors
                                        │
                                        ▼  (actor-to-actor via same pub/sub)
                                   Projector writes AppState (RwLock)
                                        │
                                        ▼
                                   Renderer reads AppState (next tick)

Unidirectional: frontend → actor system. No feedback loop.
The shared AppState is the feedback — the projector writes it,
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

## Coordinator Actor

The Coordinator (in `actors/nullslop-coordinator`) subscribes to domain `Command`s via the actor host's pub/sub routing. It orchestrates multi-step workflows (e.g., enqueue message → assemble prompt → send to LLM) and emits new `Command`s and `Event`s through its `MessageSink`.

The coordinator does NOT handle shutdown or lifecycle concerns. It uses the `nullslop-actor` SDK and receives injected `Services` for external dependencies.

## Projector Actor

The Projector (in `actors/nullslop-projector`) subscribes to domain `Event`s and writes `AppState` via shared `State`. It is a pure event→state projection with zero command or event emissions.

**Subscriptions — 12 events:**

| Subscribes to            | Purpose                                     |
| ------------------------ | ------------------------------------------- |
| `StreamToken`            | Appends streaming text to assistant entries |
| `StreamCompleted`        | Marks stream as finished                    |
| `ToolCallReceived`       | Records incoming tool call                  |
| `ToolUseStarted`         | Begins a tool call for streaming deltas     |
| `ToolCallStreaming`      | Appends tool call streaming deltas          |
| `ToolExecutionCompleted` | Records tool execution result               |
| `ToolsRegistered`        | Updates available tools list                |
| `ProviderSwitched`       | Updates active provider                     |
| `ModelsRefreshed`        | Refreshes model cache                       |
| `PromptTemplatesLoaded`  | Updates prompt template list                |
| `PromptStrategySwitched` | Updates active strategy                     |
| `StrategyStateUpdated`   | Updates strategy state                      |

Lock discipline: acquire write lock → mutate → release. Never hold the lock during async work or when emitting messages.

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
- **Projector** — acquires write lock per event, mutates, releases lock
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

| Crate                       | Responsibility                                                                                                                                                                    |
| --------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `nullslop-protocol`         | `Command` (domain-only), `Event`, `Intent` enum, `Mode`, `Key`, `AppMsg`, `CoreNotification`, domain types                                                                        |
| `nullslop-intent`           | `IntentHandler` (match block), validators, validator errors, `IntentResult`                                                                                                       |
| `nullslop-component`        | State structs, UI elements, `AppState`, `State` (RwLock wrapper), picker entries                                                                                                  |
| `nullslop-component-ui`     | `UiElement` trait, `UiRegistry`                                                                                                                                                   |
| `nullslop-core`             | `AppCore` (state + sender), `coordinated_shutdown`, `spawn_forwarding_task`, `ActorMessageSink`                                                                                   |
| `nullslop-services`         | `Services` container, `ActorChannelService`, `CoreChannelService`                                                                                                                 |
| `nullslop-tui`              | Terminal, event loop, keymap (produces `Intent`), renderer, `TuiApp`                                                                                                              |
| `nullslop-actor`            | Actor SDK (`Actor` trait, `ActorContext`, `MessageSink`, `ActorRef`, `RecordingSink`)                                                                                             |
| `nullslop-actor-host`       | Actor host implementations (process-based, in-memory, fake)                                                                                                                       |
| `nullslop-cli`              | CLI argument parsing                                                                                                                                                              |
| `nullslop-coordinator`      | Coordinator actor — subscribes to Commands, orchestrates workflows                                                                                                                |
| `nullslop-projector`        | Projector actor — subscribes to Events, writes AppState                                                                                                                           |
| `nullslop-shutdown-tracker` | ShutdownTracker actor — manages actor lifecycle tracking                                                                                                                          |
| Other actors                | Domain actors: `nullslop-llm`, `nullslop-session-actor`, `nullslop-context-actor`, `nullslop-tool-orchestrator`, `nullslop-echo`, `nullslop-prompt-scan`, `nullslop-llm-discover` |

## Keymap

The keymap (in `nullslop-tui`) binds physical keys to `Intent` variants scoped by mode (`Normal`, `Input`, `Dashboard`, `Pinned`, `Picker`). When a key matches, the which-key system produces an `Intent` and feeds it into the `IntentHandler`.

## Testing Strategy

- **Intent handler** — test `IntentHandler::handle(intent, &mut state)` directly, assert state changes and returned commands
- **Validators** — test in isolation: call validator with known state, assert `Ok`/`Err`
- **UI elements** — test with ratatui `TestBackend`: render with known state, assert buffer contents
- **State types** — unit tests for behavior (grapheme handling, cursor bounds, etc.)
- **Actors** — test with `RecordingSink` from `nullslop-actor`: send command/event, assert messages emitted

Tests follow Given/When/Then structure. See `AGENTS.md` for detailed testing patterns.
