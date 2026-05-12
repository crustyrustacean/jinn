# Architecture

nullslop is a TUI chat application built on an intent-driven architecture with an actor bridge. User input flows through a single `IntentHandler` that validates, mutates shared state, and emits domain commands. Domain-specific actors run asynchronously, communicate through the actor host's pub/sub routing, and write their state fields directly into shared `AppState`. There is no bus.

The application supports two execution modes — an interactive TUI and a headless mode that can send messages or replay keystroke scripts without a terminal.

## Data Flow

```
  Keyboard / Mouse / Script
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

The `IntentHandler` is the single decision point for all user input. Its `handle` method takes an `Intent`, validates it, mutates `AppState` directly, and returns an `IntentResult` carrying zero or more domain `Command`s:

```rust
pub fn handle(intent: &Intent, state: &mut AppState) -> IntentResult
```

An intent does exactly one of three things:

1. **Fail validation** → no state change, no commands
2. **Mutate `AppState` directly** (pure UI: scroll, cursor, mode switch) → no commands
3. **Mutate `AppState` + emit `Command`s** (domain: submit message, switch provider) → commands forwarded to the actor system

The `IntentHandler` never accesses external services. It never emits events. It only reads/writes `AppState` and returns commands for the caller to send.

## Validators

Each intent has a dedicated validator function. Validators are plain functions — no registries or trait objects:

- **Infallible intents** — always succeed, no validation logic needed. These don't need a validator.
- **Fallible intents** — return `Result<(), SpecificError>` with a custom error enum per intent.

All validators take `&AppState` as input. On validation failure, the `IntentHandler` match arm does nothing (no-op). Example:

```rust
pub fn validate_submit_message(state: &AppState) -> Result<(), SubmitMessageError> {
    if state.active_chat_input().is_empty() {
        return Err(SubmitMessageError::EmptyBuffer);
    }
    Ok(())
}
```

## Shared State

`AppState` is the single source of truth — one struct containing all UI and domain state. `State` wraps it in an `Arc<RwLock<AppState>>` for cross-thread access.

`AppState` is divided into domain-grouped sub-structs. Each sub-struct is owned by a specific subsystem — the intent handler owns frontend state, the session actor owns session state, the provider actor owns provider state, and so on. Cross-boundary writes are a code review red flag.

Both the synchronous intent handler (on the main thread) and the async actor system share the same `State`:

- **Intent handler** — acquires write lock, validates, mutates, returns commands, releases lock
- **Domain actors** — acquire write lock per handler, mutate their owned fields, release lock, then emit
- **Renderer** — acquires read lock on each tick, draws to screen

`AppState` implements `Default` for easy test construction.

## Actors

Actors run asynchronously as tokio tasks. They communicate through the actor host's pub/sub routing — no direct bus required.

### Actor trait

Every actor implements the `Actor` trait:

```rust
trait Actor {
    type Message: Send + 'static;

    fn activate(ctx: &mut ActorContext) -> Self;
    async fn handle(&mut self, msg: ActorEnvelope<Self::Message>, ctx: &ActorContext);
    async fn shutdown(self);
}
```

- `activate` — constructor. Subscribes to events/commands, extracts peer refs, receives injected services.
- `handle` — unified message handler for all incoming messages.
- `shutdown` — cleanup after the run loop exits.

### ActorEnvelope

Every message an actor receives is wrapped in an `ActorEnvelope`:

- `Event(Event)` — an event the actor subscribed to
- `Command(Command)` — a command the actor registered for
- `Direct(M)` — typed direct message from another actor (the actor's own message type)
- `System(SystemMessage)` — lifecycle broadcasts (`ApplicationReady`, `ApplicationShuttingDown`)
- `Shutdown` — signal to exit the run loop

### Subscriptions

During `activate`, an actor declares what it cares about:

```rust
ctx.subscribe_event::<SomeEvent>();
ctx.subscribe_command::<SomeCommand>();
```

The actor host builds pre-computed routing tables from these declarations. Events are broadcast to all subscribers. Commands are routed to exactly one subscriber — subscribing multiple actors to the same command panics at startup.

### Source filtering

Actors never receive their own emissions. The actor host passes the originating actor's name when routing, and skips the source. This prevents echo loops without requiring actors to filter their own messages.

### ActorRef

Each actor gets a typed, cloneable `ActorRef<M>` handle wrapping a channel sender. `ActorRef` is used for:

- Direct actor-to-actor messaging (via `send(M)`)
- Captured in routing table closures for pub/sub delivery
- Swappable sender for restart scenarios

### ActorContext

Provided to all actor methods. Holds:

- Actor name and description
- Subscription accumulation
- Type-keyed peer `ActorRef` storage (for direct messaging between actors)
- Type-keyed data injection (for service dependencies)
- `MessageSink` access (for emitting commands/events back onto the bus)
- Lifecycle announcements (`announce_started`, `announce_shutdown_completed`)

### MessageSink

How actors emit back to the bus:

```rust
trait MessageSink {
    fn send_command(&self, command: Command) -> SendResult;
    fn send_event(&self, event: Event) -> SendResult;
}
```

A `RecordingSink` implementation records all messages in memory for tests.

### ActorHost

The host manages actor lifecycle and message routing:

- `send_event` — routes to all subscribed actors, skipping the source
- `send_command` — routes to the single registered actor, skipping the source
- `send_system` — broadcasts to all actors regardless of subscriptions
- `shutdown` — initiates graceful shutdown

Two implementations exist: `InMemoryActorHost` (production, tokio tasks with pre-computed routing tables) and `FakeActorHost` (tests, records messages without spawning actors). Routing tables are built once at startup from actor subscriptions — the hot path is lock-free.

### Spawning

Actors are spawned via a `spawn` function per actor module that creates the `ActorRef`, subscribes via `activate`, spawns the tokio task running the receive loop, and returns an `ActorSpawnResult`. The host consumes these results to build its routing tables.

## Rendering

UI elements implement a `UiElement<S>` trait. They read state and draw to a ratatui `Frame`. Elements have no knowledge of intents, commands, or events — they just read state and draw.

```rust
trait UiElement<S> {
    fn name(&self) -> String;
    fn render(&mut self, frame: &mut Frame, area: Rect, state: &S);
    fn is_selectable(&self) -> bool;
}
```

Elements are registered into a `UiRegistry` at startup. The renderer iterates registered elements, allocates screen areas, and calls `render` with the current `AppState`.

## Keymap

The keymap binds physical keys to `Intent` variants scoped by mode and context (e.g., normal mode on the chat tab, input mode, picker mode). When a key matches, the which-key system produces an `Intent` and feeds it into the `IntentHandler`.

## Testing Strategy

- **Intent handler** — call `IntentHandler::handle(intent, &mut state)` directly, assert state changes and returned commands
- **Validators** — call in isolation with known state, assert `Ok`/`Err`
- **UI elements** — render with ratatui `TestBackend`, assert buffer contents
- **State types** — unit tests for behavior (grapheme handling, cursor bounds, etc.)
- **Actors** — send command/event, use `RecordingSink` to assert emitted messages

Tests follow Given/When/Then structure. See `AGENTS.md` for detailed testing patterns.
